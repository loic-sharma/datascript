//! The interactive REPL on a terminal: reedline with the submit rule, highlighting from the
//! lexer, Tab completion and persistent history.

use std::borrow::Cow;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use datascript_compiler::FunctionCatalog;
use datascript_diagnostics::FileId;
use datascript_syntax::{OPERATOR_NAMES, SyntaxKind, lex};
use nu_ansi_term::{Color, Style};
use reedline::{
    ColumnarMenu, Completer, Emacs, FileBackedHistory, Highlighter, KeyCode, KeyModifiers,
    MenuBuilder, Prompt, PromptEditMode, PromptHistorySearch, Reedline, ReedlineEvent,
    ReedlineMenu, Signal, StyledText, Suggestion, ValidationResult, Validator,
    default_emacs_keybindings,
};

use super::session::COMMANDS;
use super::{Printer, ReplSession, Step, input};
use crate::args::ColorChoice;

/// History entries kept.
const HISTORY_SIZE: usize = 1000;

/// Runs the interactive loop until `.quit`, `.exit` or Ctrl-D. Returns the exit code.
pub fn run(
    mut repl: ReplSession,
    printer: Printer,
    runtime: &tokio::runtime::Runtime,
    color: ColorChoice,
) -> u8 {
    let names = Arc::new(Mutex::new(repl.completion_names()));
    // The buffer as last drawn: reedline clears it before reporting Ctrl-C, so this is how the
    // loop knows whether the prompt was empty.
    let last_buffer = Arc::new(Mutex::new(String::new()));
    let mut editor = editor(color, names.clone(), last_buffer.clone());
    let prompt = ReplPrompt;
    println!(
        "DataScript {} — type .help for help",
        env!("CARGO_PKG_VERSION")
    );
    loop {
        match editor.read_line(&prompt) {
            Ok(Signal::Success(text)) => {
                if text.trim().is_empty() {
                    continue;
                }
                let step = runtime.block_on(submit(&mut repl, &printer, &text));
                *lock(&names) = repl.completion_names();
                if let Step::Quit = step {
                    return 0;
                }
            }
            Ok(Signal::CtrlC) => {
                if lock(&last_buffer).trim().is_empty() {
                    eprintln!("press Ctrl-D or type .quit to exit");
                }
            }
            Ok(Signal::CtrlD) => return 0,
            // reedline 0.49 has no other signals; any later one is treated as Ctrl-D.
            #[allow(unreachable_patterns)]
            Ok(_) => return 0,
            Err(e) => {
                eprintln!("error: cannot read from the terminal: {e}");
                return crate::EXIT_USAGE;
            }
        }
    }
}

/// Submits one input. Checking and running happen on a worker thread, so the main thread
/// notices Ctrl-C even while a query computes. Ctrl-C drops the work, which cancels any running
/// query, and returns to the prompt; saved lets are kept (D-048).
async fn submit(repl: &mut ReplSession, printer: &Printer, text: &str) -> Step {
    if input::is_meta(text) {
        let outcome = tokio::select! {
            outcome = repl.meta(text) => outcome,
            _ = tokio::signal::ctrl_c() => return cancelled(),
        };
        return printer.meta(repl, outcome);
    }
    let prepared = repl.prepare(text);
    let Some(analysis) = interruptible(repl.check_task(&prepared)).await else {
        return cancelled();
    };
    let checked = repl.commit(prepared, analysis);
    if printer.checked(repl, &checked) {
        return Step::Continue { failed: false };
    }
    let Some(task) = repl.run_task(&checked, Box::new(std::io::stdout())) else {
        return Step::Continue { failed: false };
    };
    let Some(report) = interruptible(task).await else {
        return cancelled();
    };
    let ran = repl.finish_run(&checked, report);
    printer.ran(repl, &ran);
    Step::Continue { failed: false }
}

/// Runs `work` on a worker thread until it finishes (`Some`) or Ctrl-C is pressed (`None`, and
/// the work is aborted, which drops and so cancels it).
async fn interruptible<T: Send + 'static>(
    work: impl Future<Output = T> + Send + 'static,
) -> Option<T> {
    let mut handle = tokio::spawn(work);
    tokio::select! {
        joined = &mut handle => match joined {
            Ok(value) => Some(value),
            Err(e) if e.is_panic() => std::panic::resume_unwind(e.into_panic()),
            Err(_) => None,
        },
        _ = tokio::signal::ctrl_c() => {
            handle.abort();
            None
        }
    }
}

fn cancelled() -> Step {
    eprintln!("query cancelled");
    Step::Continue { failed: false }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

fn editor(
    color: ColorChoice,
    names: Arc<Mutex<Vec<String>>>,
    last_buffer: Arc<Mutex<String>>,
) -> Reedline {
    let mut keybindings = default_emacs_keybindings();
    keybindings.add_binding(
        KeyModifiers::NONE,
        KeyCode::Tab,
        ReedlineEvent::UntilFound(vec![
            ReedlineEvent::Menu("completion_menu".to_string()),
            ReedlineEvent::MenuNext,
        ]),
    );
    let menu = ColumnarMenu::default().with_name("completion_menu");
    Reedline::create()
        .with_history(history())
        .with_validator(Box::new(SubmitRule))
        .with_highlighter(Box::new(DsHighlighter {
            color: color_on(color),
            last_buffer,
        }))
        .with_completer(Box::new(DsCompleter { names }))
        .with_menu(ReedlineMenu::EngineCompleter(Box::new(menu)))
        .with_edit_mode(Box::new(Emacs::new(keybindings)))
}

/// Whether to highlight: the same rule as diagnostics, for standard output.
fn color_on(color: ColorChoice) -> bool {
    let no_color = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
    match color {
        ColorChoice::Always => true,
        ColorChoice::Never => false,
        ColorChoice::Auto => !no_color && std::io::stdout().is_terminal(),
    }
}

/// History in `<data dir>/datascript/history`, or in memory if that file cannot be used.
fn history() -> Box<FileBackedHistory> {
    let file = history_path().ok_or_else(|| "there is no data directory".to_string());
    let opened = file.and_then(|path| {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        FileBackedHistory::with_file(HISTORY_SIZE, path.clone())
            .map_err(|e| format!("{}: {e}", path.display()))
    });
    match opened {
        Ok(history) => Box::new(history),
        Err(reason) => {
            eprintln!("warning: history will not be saved ({reason})");
            Box::new(
                FileBackedHistory::new(HISTORY_SIZE)
                    .expect("an in-memory history with a nonzero size"),
            )
        }
    }
}

fn history_path() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join("datascript").join("history"))
}

/// Enter submits by the REPL's rule (D-048).
struct SubmitRule;

impl Validator for SubmitRule {
    fn validate(&self, line: &str) -> ValidationResult {
        if input::submits(line) {
            ValidationResult::Complete
        } else {
            ValidationResult::Incomplete
        }
    }
}

struct ReplPrompt;

impl Prompt for ReplPrompt {
    fn render_prompt_left(&self) -> Cow<'_, str> {
        Cow::Borrowed("ds> ")
    }

    fn render_prompt_right(&self) -> Cow<'_, str> {
        Cow::Borrowed("")
    }

    fn render_prompt_indicator(&self, _mode: PromptEditMode) -> Cow<'_, str> {
        Cow::Borrowed("")
    }

    fn render_prompt_multiline_indicator(&self) -> Cow<'_, str> {
        Cow::Borrowed("..> ")
    }

    fn render_prompt_history_search_indicator(&self, search: PromptHistorySearch) -> Cow<'_, str> {
        Cow::Owned(format!("(search: {}) ", search.term))
    }
}

/// Colors tokens by kind, and remembers the buffer for the Ctrl-C hint.
struct DsHighlighter {
    color: bool,
    last_buffer: Arc<Mutex<String>>,
}

impl Highlighter for DsHighlighter {
    fn highlight(&self, line: &str, _cursor: usize) -> StyledText {
        line.clone_into(&mut lock(&self.last_buffer));
        let mut styled = StyledText::new();
        if !self.color {
            styled.push((Style::new(), line.to_string()));
            return styled;
        }
        for token in lex(line, FileId(0)).tokens {
            let text = token.text(line);
            styled.push((style_of(token.kind, text), text.to_string()));
        }
        styled
    }
}

/// The style of a token: keywords and operator names, strings, numbers, comments, lexer errors.
fn style_of(kind: SyntaxKind, text: &str) -> Style {
    use SyntaxKind::*;
    match kind {
        LET_KW | TRUE_KW | FALSE_KW | NULL_KW | AND_KW | OR_KW | NOT_KW => Color::Magenta.bold(),
        IDENT if text == "from" || OPERATOR_NAMES.contains(&text) => Color::Blue.bold(),
        PIPE | NEGATED_OP | IN_TILDE => Color::Blue.bold(),
        STRING => Color::Green.normal(),
        LONG | REAL | TIMESPAN | DATETIME => Color::Cyan.normal(),
        LINE_COMMENT | BLOCK_COMMENT => Color::DarkGray.italic(),
        ERROR => Color::Red.underline(),
        _ => Style::new(),
    }
}

/// Completes operator, function, saved let and host table names, and meta commands at the start
/// of the input. Columns come later (M3-02).
struct DsCompleter {
    names: Arc<Mutex<Vec<String>>>,
}

impl Completer for DsCompleter {
    fn complete(&mut self, line: &str, pos: usize) -> Vec<Suggestion> {
        let before = &line[..pos];
        let typed = before.trim_start();
        let (start, candidates): (usize, Vec<String>) =
            if typed.starts_with('.') && !typed.contains(char::is_whitespace) {
                // Still typing the command name.
                let commands = COMMANDS.iter().filter(|c| c.starts_with(typed));
                (pos - typed.len(), commands.map(|c| c.to_string()).collect())
            } else {
                let (start, word) = word_before(line, pos);
                if word.is_empty() {
                    return Vec::new();
                }
                let saved = lock(&self.names);
                let mut all: Vec<String> = OPERATOR_NAMES
                    .iter()
                    .copied()
                    .chain(FunctionCatalog::builtin().names())
                    .map(str::to_string)
                    .chain(saved.iter().cloned())
                    .filter(|c| c.starts_with(word))
                    .collect();
                all.sort();
                all.dedup();
                (start, all)
            };
        candidates
            .into_iter()
            .map(|value| Suggestion {
                value,
                description: None,
                style: None,
                extra: None,
                span: reedline::Span::new(start, pos),
                append_whitespace: true,
                display_override: None,
                match_indices: None,
            })
            .collect()
    }
}

/// The name being typed before `pos`, and where it starts. Operator names may contain `-`
/// (`project-away`).
fn word_before(line: &str, pos: usize) -> (usize, &str) {
    let before = &line[..pos];
    let start = before
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .last()
        .map_or(pos, |(i, _)| i);
    (start, &line[start..pos])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete(line: &str, names: &[&str]) -> Vec<String> {
        let mut completer = DsCompleter {
            names: Arc::new(Mutex::new(names.iter().map(|s| s.to_string()).collect())),
        };
        completer
            .complete(line, line.len())
            .into_iter()
            .map(|s| s.value)
            .collect()
    }

    #[test]
    fn completes_operators_functions_lets_and_tables() {
        assert!(complete("from t | whe", &[]).contains(&"where".to_string()));
        assert!(complete("from t | project-a", &[]).contains(&"project-away".to_string()));
        assert!(complete("from t | extend y = strl", &[]).contains(&"strlen".to_string()));
        assert_eq!(
            complete("from big_o", &["big_orders", "small"]),
            ["big_orders"]
        );
        assert!(complete("from t | take ", &[]).is_empty());
    }

    #[test]
    fn completes_meta_commands_at_the_start_only() {
        assert_eq!(complete(".sc", &[]), [".schema"]);
        assert_eq!(complete(".", &[]).len(), COMMANDS.len());
        assert!(complete(".schema ord", &["orders"]).contains(&"orders".to_string()));
    }

    #[test]
    fn highlighting_is_plain_without_color() {
        let highlighter = DsHighlighter {
            color: false,
            last_buffer: Arc::new(Mutex::new(String::new())),
        };
        let styled = highlighter.highlight("from t | take 5", 0);
        assert_eq!(styled.buffer.len(), 1);
        assert_eq!(*lock(&highlighter.last_buffer), "from t | take 5");
        let highlighter = DsHighlighter {
            color: true,
            last_buffer: Arc::new(Mutex::new(String::new())),
        };
        let styled = highlighter.highlight("from t | take 5 // n", 0);
        let text: String = styled.buffer.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(text, "from t | take 5 // n");
        assert_eq!(styled.buffer[0].0, Color::Blue.bold());
    }
}
