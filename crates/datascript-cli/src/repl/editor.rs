//! The interactive REPL on a terminal: reedline with the submit rule, highlighting from the
//! lexer, Tab completion and persistent history.

use std::borrow::Cow;
use std::collections::HashSet;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use datascript_compiler::ide::{Completion, CompletionKind, FileCompletion};
use datascript_compiler::source::format_of_path;
use datascript_diagnostics::FileId;
use datascript_syntax::{OPERATOR_NAMES, SyntaxKind, display_name, lex};
use nu_ansi_term::{Color, Style};
use reedline::{
    ColumnarMenu, Completer, Emacs, FileBackedHistory, Highlighter, InputMode, KeyCode,
    KeyModifiers, MenuBuilder, Prompt, PromptEditMode, PromptHistorySearch, Reedline,
    ReedlineEvent, ReedlineMenu, Signal, StyledText, Suggestion, ValidationResult, Validator,
    default_emacs_keybindings,
};

use super::session::{COMMANDS, CompletionContext};
use super::{Printer, ReplSession, Step, input};
use crate::args::{ColorChoice, TableArg};

/// History entries kept.
const HISTORY_SIZE: usize = 1000;

/// Runs the interactive loop until `.quit`, `.exit` or Ctrl-D. Returns the exit code.
pub fn run(
    mut repl: ReplSession,
    printer: Printer,
    runtime: &tokio::runtime::Runtime,
    color: ColorChoice,
) -> u8 {
    let completion = Arc::new(Mutex::new(repl.completion_context()));
    // The buffer as last drawn: reedline clears it before reporting Ctrl-C, so this is how the
    // loop knows whether the prompt was empty.
    let last_buffer = Arc::new(Mutex::new(String::new()));
    let mut editor = editor(
        color,
        completion.clone(),
        runtime.handle().clone(),
        last_buffer.clone(),
    );
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
                *lock(&completion) = repl.completion_context();
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
    context: Arc<Mutex<CompletionContext>>,
    runtime: tokio::runtime::Handle,
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
    let menu = completion_menu();
    Reedline::create()
        .with_history(history())
        .with_validator(Box::new(SubmitRule))
        .with_highlighter(Box::new(DsHighlighter {
            color: color_on(color),
            last_buffer,
        }))
        .with_completer(Box::new(DsCompleter { context, runtime }))
        .with_menu(ReedlineMenu::EngineCompleter(Box::new(menu)))
        .with_edit_mode(Box::new(Emacs::new(keybindings)))
}

/// The completion menu, configured with `InputMode::FullBuffer` so `DsCompleter` sees the whole
/// buffer on mid-line and mid-word Tab completion instead of only the prefix up to the cursor.
fn completion_menu() -> ColumnarMenu {
    ColumnarMenu::default()
        .with_name("completion_menu")
        .with_input_mode(InputMode::FullBuffer)
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

/// The most file names one completion lists (same cap as the language server).
const MAX_FILES: usize = 500;

/// Completes columns, operators, functions, saved lets, keywords, files, host tables and meta
/// commands. Language items come from `datascript_compiler::ide::complete` over the saved lets
/// and the current input (M3-07); meta commands and host tables are added by the REPL.
struct DsCompleter {
    context: Arc<Mutex<CompletionContext>>,
    runtime: tokio::runtime::Handle,
}

impl Completer for DsCompleter {
    fn complete(&mut self, line: &str, pos: usize) -> Vec<Suggestion> {
        let pos = line.floor_char_boundary(pos.min(line.len()));
        let context = lock(&self.context).clone();
        let before = &line[..pos];
        let trimmed = before.trim_start();
        if trimmed.starts_with('.') {
            return complete_meta(&context, &self.runtime, line, pos, trimmed);
        }
        complete_query(&context, &self.runtime, line, pos)
    }
}

fn complete_meta(
    context: &CompletionContext,
    runtime: &tokio::runtime::Handle,
    line: &str,
    pos: usize,
    trimmed: &str,
) -> Vec<Suggestion> {
    let cmd_start = pos - trimmed.len();
    if !trimmed.contains(char::is_whitespace) {
        let end = line[cmd_start..]
            .find(char::is_whitespace)
            .map_or(line.len(), |i| cmd_start + i);
        let append_ws = !line[end..].starts_with(char::is_whitespace);
        return COMMANDS
            .iter()
            .filter(|c| c.starts_with(trimmed))
            .map(|c| suggestion((*c).to_string(), None, cmd_start, end, append_ws))
            .collect();
    }
    let cmd = trimmed
        .split_whitespace()
        .next()
        .expect("trimmed starts with '.'");
    let cmd_end = cmd_start + cmd.len();
    match cmd {
        ".schema" => {
            // Check `.schema <arg>` as `from <arg>`, matching `ReplSession::schema`.
            let synthetic = format!("from{}", &line[cmd_end..]);
            let synth_pos = 4 + (pos - cmd_end);
            let mut items = complete_query(context, runtime, &synthetic, synth_pos);
            for item in &mut items {
                item.span = reedline::Span::new(
                    cmd_end + item.span.start.saturating_sub(4),
                    cmd_end + item.span.end.saturating_sub(4),
                );
            }
            items
        }
        ".format" => {
            let arg = line[cmd_end..pos].trim_start();
            if arg.contains(char::is_whitespace) {
                return Vec::new();
            }
            let start = pos - arg.len();
            let end = line[pos..]
                .find(char::is_whitespace)
                .map_or(line.len(), |i| pos + i);
            let append_ws = !line[end..].starts_with(char::is_whitespace);
            ["table", "csv", "json"]
                .into_iter()
                .filter(|f| f.starts_with(arg))
                .map(|f| suggestion(f.to_string(), None, start, end, append_ws))
                .collect()
        }
        _ => Vec::new(),
    }
}

fn complete_query(
    context: &CompletionContext,
    runtime: &tokio::runtime::Handle,
    line: &str,
    pos: usize,
) -> Vec<Suggestion> {
    let completions = runtime.block_on(context.complete(line, pos));
    let start = completions.start;
    let end = completions.end;
    let typed = &line[start..pos];
    let space_follows = line[end..].starts_with(char::is_whitespace);

    let mut items = completions.items;
    if is_table_position(line, start, completions.files.as_ref()) {
        let shadowed = context.let_names_before(line, pos);
        add_host_tables(&mut items, context.tables(), &shadowed);
    }

    let mut out = Vec::new();
    for item in items {
        // Reedline cannot expand LSP snippets (`$0`, `${1:name}`), so insert the plain label.
        let value = if item.snippet {
            item.label
        } else {
            item.insert
        };
        if !value.starts_with(typed) {
            continue;
        }
        let append_whitespace = item.kind != CompletionKind::Function && !space_follows;
        out.push(suggestion(
            value,
            item.detail,
            start,
            end,
            append_whitespace,
        ));
    }

    if let Some(files) = &completions.files {
        out.extend(file_suggestions(
            files,
            &context.base_dir().join(&files.dir),
            typed,
            start,
            end,
            space_follows,
        ));
    }

    out
}

/// Whether the cursor is in a table-source position (`from`, `join`, `union`) where a host table
/// name fits, rather than inside quotes or a reader call (`read_csv(`).
fn is_table_position(text: &str, start: usize, files: Option<&FileCompletion>) -> bool {
    if !files.is_some_and(|f| f.quote.is_none()) {
        return false;
    }
    let prev = lex(&text[..start], FileId(0))
        .tokens
        .into_iter()
        .rev()
        .find(|t| !t.kind.is_trivia());
    !matches!(prev.map(|t| t.kind), Some(SyntaxKind::L_PAREN))
}

/// Adds host tables (`--table name=path`) alongside tabular lets, skipping any name already
/// shadowed by a `let` in scope.
fn add_host_tables(items: &mut Vec<Completion>, tables: &[TableArg], shadowed: &HashSet<String>) {
    let mut extra: Vec<Completion> = tables
        .iter()
        .filter_map(|t| {
            if shadowed.contains(&t.name) {
                return None;
            }
            let label = display_name(&t.name);
            if items.iter().any(|i| i.label == label) {
                return None;
            }
            Some(Completion {
                insert: label.clone(),
                label,
                kind: CompletionKind::Table,
                detail: Some(format!("host table ({})", t.path)),
                documentation: None,
                snippet: false,
            })
        })
        .collect();
    extra.sort_by(|a, b| a.label.cmp(&b.label));
    let insert_at = items
        .iter()
        .position(|i| i.kind > CompletionKind::Table)
        .unwrap_or(items.len());
    items.splice(insert_at..insert_at, extra);
}

/// Lists data files and directories in `dir` matching `typed`, matching the language server's
/// filtering and ordering (`datascript-lsp/src/ide.rs`).
fn file_suggestions(
    files: &FileCompletion,
    dir: &Path,
    typed: &str,
    start: usize,
    end: usize,
    space_follows: bool,
) -> Vec<Suggestion> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<(bool, String, String)> = entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_owned();
            if name.starts_with('.') {
                return None;
            }
            let is_dir = e.file_type().ok()?.is_dir();
            if !is_dir && format_of_path(&name).is_none() {
                return None;
            }
            let label = if is_dir {
                format!("{name}/")
            } else {
                name.clone()
            };
            let value = match (files.quote, is_dir) {
                (Some(_), _) => label,
                (None, true) => format!("'{}{label}", files.dir),
                (None, false) => format!("'{}{label}'", files.dir),
            };
            value.starts_with(typed).then_some((is_dir, name, value))
        })
        .collect();
    // Directories first, then files, each by name.
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    found.truncate(MAX_FILES);
    found
        .into_iter()
        .map(|(is_dir, _, value)| {
            let append_whitespace = files.quote.is_none() && !is_dir && !space_follows;
            let detail = Some(if is_dir { "directory" } else { "file" }.to_string());
            suggestion(value, detail, start, end, append_whitespace)
        })
        .collect()
}

fn suggestion(
    value: String,
    description: Option<String>,
    start: usize,
    end: usize,
    append_whitespace: bool,
) -> Suggestion {
    Suggestion {
        value,
        description,
        style: None,
        extra: None,
        span: reedline::Span::new(start, end),
        append_whitespace,
        display_override: None,
        match_indices: None,
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use datascript_compiler::SourceRef;
    use datascript_datafusion::DataFusionEngine;
    use datascript_engine::{FixedClock, Session};

    use super::*;

    fn workspace_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    fn repl_with_tables(tables: &[(&str, &str)]) -> ReplSession {
        let mut engine = DataFusionEngine::default();
        let mut args = Vec::new();
        for &(name, path) in tables {
            let source = SourceRef::for_path(path);
            engine = engine.with_table(name, source);
            args.push(TableArg {
                name: name.to_string(),
                path: path.to_string(),
            });
        }
        let mut repl = ReplSession::new(
            Session::new(Arc::new(engine), Arc::new(FixedClock(0))),
            args,
        );
        repl.set_base_dir(workspace_root());
        repl
    }

    fn suggestions_at(
        repl: &ReplSession,
        rt: &tokio::runtime::Runtime,
        line: &str,
        pos: usize,
    ) -> Vec<Suggestion> {
        let mut completer = DsCompleter {
            context: Arc::new(Mutex::new(repl.completion_context())),
            runtime: rt.handle().clone(),
        };
        completer.complete(line, pos)
    }

    fn values(repl: &ReplSession, rt: &tokio::runtime::Runtime, line: &str) -> Vec<String> {
        suggestions_at(repl, rt, line, line.len())
            .into_iter()
            .map(|s| s.value)
            .collect()
    }

    #[test]
    fn completes_columns_from_a_file_source() {
        let rt = runtime();
        let repl = repl_with_tables(&[]);
        let line = "from 'tests/data/orders.csv' | where am";
        let items = suggestions_at(&repl, &rt, line, line.len());
        assert_eq!(
            items.iter().map(|s| s.value.as_str()).collect::<Vec<_>>(),
            ["amount"]
        );
        assert_eq!(items[0].description.as_deref(), Some("real"));
        assert_eq!(items[0].span, reedline::Span::new(37, 39));
        assert!(items[0].append_whitespace);

        // Columns reflect earlier pipeline stages in table order.
        let narrowed = values(
            &repl,
            &rt,
            "from 'tests/data/orders.csv' | project amount, orderId | where ",
        );
        assert_eq!(&narrowed[..2], ["amount", "orderId"]);
        assert!(!narrowed.contains(&"customerId".to_string()));
        assert!(!narrowed.contains(&"timestamp".to_string()));

        // Mid-line completion replaces the word at the cursor without adding a second space.
        let mid = "from 'tests/data/orders.csv' | where am > 10";
        let items = suggestions_at(&repl, &rt, mid, 39);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].value, "amount");
        assert_eq!(items[0].span, reedline::Span::new(37, 39));
        assert!(!items[0].append_whitespace);
    }

    #[test]
    fn completes_columns_from_a_saved_let_table() {
        let rt = runtime();
        let mut repl = repl_with_tables(&[]);
        let checked = rt.block_on(
            repl.check_input("let big = from 'tests/data/orders.csv' | where amount > 100;"),
        );
        assert!(!checked.failed(), "{:?}", checked.diagnostics);

        let after_project = values(&repl, &rt, "from big | project ");
        assert_eq!(
            &after_project[..4],
            ["orderId", "customerId", "amount", "timestamp"]
        );
        assert_eq!(values(&repl, &rt, "from big | where am"), ["amount"]);

        // Tabular lets appear after `from`, not in scalar expressions; scalar lets vice versa.
        let checked = rt.block_on(repl.check_input("let cutoff = 100;"));
        assert!(!checked.failed(), "{:?}", checked.diagnostics);
        assert!(values(&repl, &rt, "from b").contains(&"big".to_string()));
        assert!(!values(&repl, &rt, "from cut").contains(&"cutoff".to_string()));
        assert_eq!(
            values(&repl, &rt, "from big | where amount > cut"),
            ["cutoff"]
        );
        assert!(!values(&repl, &rt, "from big | where b").contains(&"big".to_string()));

        // Redefining a saved let in the current input uses the new definition's columns.
        assert_eq!(
            values(
                &repl,
                &rt,
                "let big = from 'tests/data/customers.csv'; from big | where na",
            ),
            ["name"]
        );
    }

    #[test]
    fn a_failing_source_gives_no_columns() {
        let rt = runtime();
        let repl = repl_with_tables(&[]);
        assert!(
            values(
                &repl,
                &rt,
                "from 'tests/data/does-not-exist.csv' | where am"
            )
            .is_empty()
        );
        assert!(
            values(
                &repl,
                &rt,
                "from 'tests/data/does-not-exist.csv' | project-away "
            )
            .is_empty()
        );
        let after_project = values(
            &repl,
            &rt,
            "from 'tests/data/does-not-exist.csv' | project ",
        );
        for col in ["orderId", "customerId", "amount", "timestamp"] {
            assert!(!after_project.contains(&col.to_string()));
        }
        assert!(values(&repl, &rt, "from nope | where am").is_empty());
        assert!(values(&repl, &rt, "from 'tests/data/orders.xyz' | where am").is_empty());
    }

    #[test]
    fn completes_operators_functions_lets_tables_and_files() {
        let rt = runtime();
        let repl = repl_with_tables(&[("orders", "tests/data/orders.csv")]);

        // Operators after `|`, including hyphenated ones.
        assert_eq!(values(&repl, &rt, "from orders | whe"), ["where"]);
        let line = "from orders | project-a";
        let items = suggestions_at(&repl, &rt, line, line.len());
        assert_eq!(
            items.iter().map(|s| s.value.as_str()).collect::<Vec<_>>(),
            ["project-away"]
        );
        assert_eq!(items[0].span, reedline::Span::new(14, 23));
        assert_eq!(
            values(&repl, &rt, "from orders | project-"),
            ["project-away", "project-rename"]
        );

        // Functions insert plain names (no `$0` snippets) without trailing whitespace,
        // and aggregates only appear inside `summarize`.
        let line = "from orders | extend y = strl";
        let items = suggestions_at(&repl, &rt, line, line.len());
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].value, "strlen");
        assert!(!items[0].append_whitespace);
        assert!(!values(&repl, &rt, "from orders | where s").contains(&"sum".to_string()));
        assert!(values(&repl, &rt, "from orders | summarize s").contains(&"sum".to_string()));
        assert!(values(&repl, &rt, "from orders | count ").is_empty());

        // Host tables appear in table positions (`from`, `join`, `union`) and yield columns,
        // but do not appear inside `read_csv(`.
        assert_eq!(values(&repl, &rt, "from ord"), ["orders"]);
        assert_eq!(values(&repl, &rt, "from orders | where am"), ["amount"]);
        assert_eq!(
            values(&repl, &rt, "from 'tests/data/customers.csv' | join ord"),
            ["orders"]
        );
        assert_eq!(
            values(&repl, &rt, "from 'tests/data/customers.csv' | union ord"),
            ["orders"]
        );
        assert!(values(&repl, &rt, "from read_csv(ord").is_empty());

        // File completion inside a path string.
        let line = "from 'tests/data/ord";
        let items = suggestions_at(&repl, &rt, line, line.len());
        assert_eq!(
            items.iter().map(|s| s.value.as_str()).collect::<Vec<_>>(),
            ["orders.csv", "orders.parquet"]
        );
        assert_eq!(items[0].span, reedline::Span::new(17, 20));
        assert!(!items[0].append_whitespace);

        // Quoted directory completion appends `/` without trailing whitespace, and directories
        // sort before files.
        let line = "from 'tests/";
        let items = suggestions_at(&repl, &rt, line, line.len());
        let data_dir = items.iter().find(|s| s.value == "data/").unwrap();
        assert!(!data_dir.append_whitespace);
        let line = "from 'tests/data/";
        let items = suggestions_at(&repl, &rt, line, line.len());
        let first_file = items
            .iter()
            .position(|s| !s.value.ends_with('/'))
            .expect("at least one data file");
        let last_dir = items
            .iter()
            .rposition(|s| s.value.ends_with('/'))
            .expect("at least one subdirectory");
        assert!(last_dir < first_file, "directories should precede files");

        // Unquoted file completion quotes files (with trailing whitespace) and leaves
        // directories open without trailing whitespace.
        let mut data_repl = repl_with_tables(&[]);
        data_repl.set_base_dir(workspace_root().join("tests/data"));
        let items = suggestions_at(&data_repl, &rt, "from ", 5);
        let events = items.iter().find(|s| s.value == "'events/").unwrap();
        assert!(!events.append_whitespace);
        let orders_file = items.iter().find(|s| s.value == "'orders.csv'").unwrap();
        assert!(orders_file.append_whitespace);
        let events_idx = items.iter().position(|s| s.value == "'events/").unwrap();
        let orders_idx = items
            .iter()
            .position(|s| s.value == "'orders.csv'")
            .unwrap();
        assert!(events_idx < orders_idx);
    }

    #[test]
    fn host_table_shadowed_by_saved_let_is_not_duplicated_or_offered() {
        let rt = runtime();
        let mut repl = repl_with_tables(&[("orders", "tests/data/orders.csv")]);

        // A tabular let of the same name shadows the host table without duplicating it.
        let checked =
            rt.block_on(repl.check_input("let orders = from 'tests/data/customers.csv';"));
        assert!(!checked.failed(), "{:?}", checked.diagnostics);
        assert_eq!(values(&repl, &rt, "from ord"), ["orders"]);
        assert_eq!(values(&repl, &rt, "from orders | where na"), ["name"]);

        // A non-string scalar let also shadows the host table, so `from ord` offers nothing.
        let checked = rt.block_on(repl.check_input("let orders = 42;"));
        assert!(!checked.failed(), "{:?}", checked.diagnostics);
        assert!(values(&repl, &rt, "from ord").is_empty());
    }

    #[test]
    fn transitive_redefinition_drops_dependent_saved_lets() {
        let rt = runtime();
        let mut repl = repl_with_tables(&[]);
        let checked = rt.block_on(repl.check_input("let a = from 'tests/data/orders.csv';"));
        assert!(!checked.failed(), "{:?}", checked.diagnostics);
        let checked = rt.block_on(repl.check_input("let b = from a | project amount;"));
        assert!(!checked.failed(), "{:?}", checked.diagnostics);

        assert_eq!(values(&repl, &rt, "from b | where am"), ["amount"]);
        // Redefining `a` in the current input removes both `a` and `b` from the prefix.
        assert!(
            values(
                &repl,
                &rt,
                "let a = from 'tests/data/customers.csv'; from b | where am",
            )
            .is_empty()
        );
        assert_eq!(
            values(
                &repl,
                &rt,
                "let a = from 'tests/data/customers.csv'; from a | where na",
            ),
            ["name"]
        );
    }

    #[test]
    fn handles_multiline_and_multibyte_utf8_offsets() {
        let rt = runtime();
        let mut repl = repl_with_tables(&[]);
        let checked = rt.block_on(repl.check_input("let greeting = 'héllo 🚀';"));
        assert!(!checked.failed(), "{:?}", checked.diagnostics);

        // Multi-line buffer maps offsets cleanly across newlines and saved lets.
        let multi = "from 'tests/data/orders.csv'\n| where am";
        let items = suggestions_at(&repl, &rt, multi, multi.len());
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].value, "amount");
        assert_eq!(items[0].span, reedline::Span::new(37, 39));

        // Multi-byte UTF-8 before the cursor preserves byte spans.
        let utf8 = "from 'tests/data/orders.csv' | where customerId == 'café' and am";
        let items = suggestions_at(&repl, &rt, utf8, utf8.len());
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].value, "amount");
        assert_eq!(&utf8[items[0].span.start..items[0].span.end], "am");

        // A cursor byte offset landing mid-codepoint does not panic.
        let mid_char = utf8.find('é').unwrap() + 1;
        let _ = suggestions_at(&repl, &rt, utf8, mid_char);
    }

    #[test]
    fn menu_passes_full_buffer_for_mid_line_and_mid_word_completion() {
        use reedline::{Editor, Menu, UndoBehavior};

        let rt = runtime();
        let repl = repl_with_tables(&[]);
        let mut completer = DsCompleter {
            context: Arc::new(Mutex::new(repl.completion_context())),
            runtime: rt.handle().clone(),
        };
        let mut menu = completion_menu();

        // Mid-line completion before ` > 10` sees the following space and avoids a double space.
        let mut editor = Editor::default();
        let mid = "from 'tests/data/orders.csv' | where am > 10";
        editor.edit_buffer(
            |buf| {
                buf.set_buffer(mid.to_string());
                buf.set_insertion_point(39);
            },
            UndoBehavior::CreateUndoPoint,
        );
        menu.update_values(&mut editor, &mut completer);
        menu.replace_in_buffer(&mut editor);
        assert_eq!(
            editor.get_buffer(),
            "from 'tests/data/orders.csv' | where amount > 10"
        );

        // Mid-word completion replaces the full word instead of leaving the suffix behind.
        let mut editor = Editor::default();
        let mid_word = "from 'tests/data/orders.csv' | where amou > 10";
        editor.edit_buffer(
            |buf| {
                buf.set_buffer(mid_word.to_string());
                buf.set_insertion_point(39);
            },
            UndoBehavior::CreateUndoPoint,
        );
        menu.update_values(&mut editor, &mut completer);
        menu.replace_in_buffer(&mut editor);
        assert_eq!(
            editor.get_buffer(),
            "from 'tests/data/orders.csv' | where amount > 10"
        );
    }

    #[test]
    fn completes_meta_commands_at_the_start_only() {
        let rt = runtime();
        let mut repl = repl_with_tables(&[("orders", "tests/data/orders.csv")]);
        let checked = rt.block_on(repl.check_input("let big = from orders;"));
        assert!(!checked.failed(), "{:?}", checked.diagnostics);

        assert_eq!(values(&repl, &rt, ".sc"), [".schema"]);
        assert_eq!(values(&repl, &rt, ".").len(), COMMANDS.len());
        let line = ".schema ord";
        let items = suggestions_at(&repl, &rt, line, line.len());
        assert_eq!(
            items.iter().map(|s| s.value.as_str()).collect::<Vec<_>>(),
            ["orders"]
        );
        assert_eq!(items[0].span, reedline::Span::new(8, 11));
        assert_eq!(values(&repl, &rt, ".schema b"), ["big"]);
        assert_eq!(
            values(&repl, &rt, ".schema 'tests/data/ord"),
            ["orders.csv", "orders.parquet"]
        );
        assert_eq!(values(&repl, &rt, ".format c"), ["csv"]);
        assert!(values(&repl, &rt, ".help x").is_empty());
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
