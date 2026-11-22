//! The REPL's session logic, with no terminal dependency (D-048): saved lets, the combined
//! program and its span mapping, and the meta commands.

use std::collections::HashSet;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use datascript_compiler::Analysis;
use datascript_compiler::hir::Column;
use datascript_diagnostics::{Diagnostic, FileId, SourceFile, Span, did_you_mean};
use datascript_engine::{ExecContext, QueryResult, RunOutput, Session};
use datascript_syntax::ast::{AstNode, Stmt};
use datascript_syntax::{SyntaxKind, lex, literal, parse};

use crate::args::{RunFormat, TableArg};
use crate::script::Sources;

/// The file id of the combined program. It is never in the source map: every span in it is
/// mapped back to the input it came from before anyone sees it.
const PROGRAM_FILE: FileId = FileId(u32::MAX);

/// What goes between two statements of the combined program. The newline ends a line comment
/// that the statement before may end with, so the `;` is not swallowed by it.
const GLUE: &str = "\n;\n";

/// The meta commands, as the user types them.
pub const COMMANDS: &[&str] = &[".help", ".schema", ".tables", ".format", ".quit", ".exit"];

/// One piece of the combined program.
#[derive(Debug, Clone)]
struct Segment {
    /// Where it starts in the combined text.
    start: u32,
    /// The input it came from, and where in that input it starts.
    file: FileId,
    local: u32,
    /// The length of the text that maps one to one; `None` for glue, all of which maps to
    /// `local`.
    len: Option<u32>,
}

/// One program checked and run on behalf of the REPL: the saved lets followed by the new input
/// (or by `from <source>` for `.schema`). It maps every offset in its text back to the input it
/// came from.
#[derive(Debug, Clone)]
pub struct ReplProgram {
    text: String,
    segments: Vec<Segment>,
    /// The input this program is for: location-less diagnostics belong to it.
    input: FileId,
}

impl ReplProgram {
    fn new(input: FileId) -> Self {
        ReplProgram {
            text: String::new(),
            segments: Vec::new(),
            input,
        }
    }

    /// Appends `text`, which starts at offset `local` of `file`.
    fn push(&mut self, text: &str, file: FileId, local: u32) {
        self.segments.push(Segment {
            start: self.len(),
            file,
            local,
            len: Some(len32(text)),
        });
        self.text.push_str(text);
    }

    /// Appends text the user never wrote. Every offset in it maps to `local` in `file`.
    fn push_glue(&mut self, text: &str, file: FileId, local: u32) {
        self.segments.push(Segment {
            start: self.len(),
            file,
            local,
            len: None,
        });
        self.text.push_str(text);
    }

    fn len(&self) -> u32 {
        len32(&self.text)
    }

    /// The combined text as a source file.
    fn source(&self) -> SourceFile {
        SourceFile::new("<repl>", self.text.clone())
    }

    /// The input file and local offset of `offset` in the combined text.
    fn map_offset(&self, offset: u32) -> (FileId, u32) {
        let index = self.segments.partition_point(|s| s.start <= offset);
        let Some(segment) = index.checked_sub(1).map(|i| &self.segments[i]) else {
            return (self.input, 0);
        };
        match segment.len {
            Some(len) => (
                segment.file,
                segment.local + (offset - segment.start).min(len),
            ),
            None => (segment.file, segment.local),
        }
    }

    fn map_span(&self, span: Span) -> Span {
        if span.file != PROGRAM_FILE {
            return span;
        }
        let (file, start) = self.map_offset(span.start);
        let (end_file, end) = self.map_offset(span.end);
        // A span cannot cross inputs; one that would (it never should) shrinks to its start.
        if end_file == file {
            Span::new(file, start, end)
        } else {
            Span::empty(file, start)
        }
    }

    /// Rewrites every span of `d` (primary, labels, fix edits) to its input file and offset.
    fn map_diagnostic(&self, mut d: Diagnostic) -> Diagnostic {
        for label in d.primary.iter_mut().chain(d.secondary.iter_mut()) {
            label.span = self.map_span(label.span);
        }
        for help in &mut d.help {
            for edit in help.suggestion.iter_mut().flat_map(|s| s.edits.iter_mut()) {
                edit.span = self.map_span(edit.span);
            }
        }
        if d.file == Some(PROGRAM_FILE) {
            d.file = Some(self.input);
        }
        d
    }

    /// Maps diagnostics and drops warnings and infos about saved lets: they were shown when the
    /// let was entered. Errors in saved lets are kept (the let's file was deleted, say).
    fn map_diagnostics(&self, diagnostics: Vec<Diagnostic>) -> Vec<Diagnostic> {
        diagnostics
            .into_iter()
            .map(|d| self.map_diagnostic(d))
            .filter(|d| d.is_error() || d.file_id().is_none_or(|f| f == self.input))
            .collect()
    }

    fn map_result(&self, mut result: QueryResult) -> QueryResult {
        result.statement = result.statement.map(|s| self.map_span(s));
        result.render_span = result.render_span.map(|s| self.map_span(s));
        result.diagnostics = self.map_diagnostics(std::mem::take(&mut result.diagnostics));
        result
    }
}

fn len32(text: &str) -> u32 {
    u32::try_from(text.len()).expect("REPL inputs are smaller than 4 GiB")
}

/// A `let` statement the REPL keeps (D-048), as text in the input it was entered in.
#[derive(Debug, Clone)]
pub struct SavedLet {
    pub name: String,
    pub tabular: bool,
    /// The statement, `;` excluded, in its input.
    span: Span,
}

/// An input ready to be checked: registered, parsed, and built into the program it is checked
/// as. See [`ReplSession::prepare`].
#[derive(Debug)]
pub struct Prepared {
    file: FileId,
    new_lets: Vec<SavedLet>,
    /// For each saved let, whether this input removes it.
    removed: Vec<bool>,
    has_queries: bool,
    program: ReplProgram,
    ctx: ExecContext,
}

/// The first phase of a submission: the input is checked and, if it has no errors, its lets are
/// saved.
#[derive(Debug)]
pub struct Checked {
    /// The input's file, `<repl:N>`.
    pub file: FileId,
    /// Every diagnostic from checking, mapped to the inputs.
    pub diagnostics: Vec<Diagnostic>,
    /// Saved lets removed because they used a let this input redefines, in order.
    pub removed: Vec<String>,
    /// The lets this input redefined that had users in `removed`.
    pub redefined: Vec<String>,
    /// The program to run, if the input checked without errors and has queries.
    run: Option<(ReplProgram, ExecContext)>,
}

impl Checked {
    /// Whether checking found errors (then nothing was saved and nothing runs).
    pub fn failed(&self) -> bool {
        self.diagnostics.iter().any(Diagnostic::is_error)
    }

    /// Whether there are queries to run.
    #[cfg(test)]
    pub fn runs(&self) -> bool {
        self.run.is_some()
    }
}

/// The second phase of a submission: the queries ran.
#[derive(Debug, Default)]
pub struct Ran {
    /// Diagnostics from running, mapped, without the ones [`Checked`] already has. Includes
    /// `DS5001` for charts.
    pub diagnostics: Vec<Diagnostic>,
    /// One result per query, spans mapped to the input.
    pub results: Vec<QueryResult>,
}

impl Ran {
    pub fn failed(&self) -> bool {
        self.diagnostics.iter().any(Diagnostic::is_error)
    }
}

/// A meta command mistake: a plain message with a tip, not a [`Diagnostic`], because there is no
/// program location (D-046, D-048).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mistake {
    pub message: String,
    pub tip: String,
}

impl Mistake {
    fn new(message: impl Into<String>, tip: impl Into<String>) -> Self {
        Mistake {
            message: message.into(),
            tip: tip.into(),
        }
    }
}

impl fmt::Display for Mistake {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "error: {}\n  tip: {}", self.message, self.tip)
    }
}

/// What a meta command did.
#[derive(Debug)]
pub enum MetaOutcome {
    /// Text for standard output (possibly empty).
    Print(String),
    /// Program diagnostics (from `.schema`), against [`ReplSession::sources`].
    Diagnostics(Vec<Diagnostic>),
    Mistake(Mistake),
    Quit,
}

/// The REPL's state: the engine session, the host tables, every input so far (for rendering
/// diagnostics), the saved lets and the output format.
#[derive(Debug)]
pub struct ReplSession {
    session: Arc<Session>,
    tables: Vec<TableArg>,
    sources: Sources,
    saved: Vec<SavedLet>,
    /// Program inputs submitted so far; meta commands don't count.
    inputs: u32,
    format: RunFormat,
    base_dir: PathBuf,
}

impl ReplSession {
    /// A REPL over `session`, whose host tables are `tables`. Relative paths resolve against
    /// the current directory.
    pub fn new(session: Session, tables: Vec<TableArg>) -> Self {
        ReplSession {
            session: Arc::new(session),
            tables,
            sources: Sources::new(),
            saved: Vec::new(),
            inputs: 0,
            format: RunFormat::Table,
            base_dir: PathBuf::from("."),
        }
    }

    /// Every input so far, for rendering diagnostics.
    pub fn sources(&self) -> &Sources {
        &self.sources
    }

    /// The text of an input.
    pub fn source(&self, file: FileId) -> &SourceFile {
        self.sources
            .map()
            .get(file)
            .expect("REPL files are in the REPL's source map")
    }

    /// The output format `.format` set.
    pub fn format(&self) -> RunFormat {
        self.format
    }

    /// The saved lets, in order.
    #[cfg(test)]
    pub fn saved(&self) -> &[SavedLet] {
        &self.saved
    }

    /// Names worth completing besides the language's own: saved lets and host tables.
    pub fn completion_names(&self) -> Vec<String> {
        self.saved
            .iter()
            .map(|l| l.name.clone())
            .chain(self.tables.iter().map(|t| t.name.clone()))
            .collect()
    }

    /// Checks a program input and, if it has no errors, saves its lets (D-048). An input that
    /// redefines a saved let removes it and every saved let that uses it.
    ///
    /// The same as [`prepare`](Self::prepare), awaiting [`check_task`](Self::check_task) and
    /// [`commit`](Self::commit), which a frontend calls itself to check on another thread.
    pub async fn check_input(&mut self, text: &str) -> Checked {
        let prepared = self.prepare(text);
        let analysis = self.check_task(&prepared).await;
        self.commit(prepared, analysis)
    }

    /// Registers the input as `<repl:N>` and builds the program it is checked as: the saved lets
    /// it does not redefine (or use through one it redefines), then the input.
    pub fn prepare(&mut self, text: &str) -> Prepared {
        self.session.invalidate_schemas();
        self.inputs += 1;
        let file = self
            .sources
            .add_snippet(&format!("<repl:{}>", self.inputs), text);
        let parsed = parse(text, file);
        let mut new_lets = Vec::new();
        let mut has_queries = false;
        for statement in parsed.program().statements() {
            match statement {
                Stmt::Let(stmt) => {
                    if let Some(name) = stmt.name().and_then(|n| n.text()) {
                        new_lets.push(SavedLet {
                            name,
                            tabular: stmt.is_tabular(),
                            span: stmt.span(file),
                        });
                    }
                }
                Stmt::Query(_) => has_queries = true,
            }
        }
        let defined: HashSet<&str> = new_lets.iter().map(|l| l.name.as_str()).collect();
        let removed = self.removed_by(&defined);
        let mut program = ReplProgram::new(file);
        for (saved, _) in self.saved.iter().zip(&removed).filter(|(_, r)| !**r) {
            self.push_saved(&mut program, saved);
        }
        program.push(text, file, 0);
        Prepared {
            file,
            new_lets,
            removed,
            has_queries,
            program,
            ctx: self.session.exec_context(&self.base_dir),
        }
    }

    /// Checks a prepared input. The future owns what it needs, so it can run on another thread
    /// while the caller watches for Ctrl-C.
    pub fn check_task(
        &self,
        prepared: &Prepared,
    ) -> impl Future<Output = Analysis> + Send + 'static {
        let session = self.session.clone();
        let src = prepared.program.source();
        let ctx = prepared.ctx.clone();
        async move { session.check(&src, PROGRAM_FILE, &ctx).await }
    }

    /// Maps the diagnostics of a check and, if there are no errors, saves the input's lets in
    /// place of the ones it removes.
    pub fn commit(&mut self, prepared: Prepared, analysis: Analysis) -> Checked {
        let Prepared {
            file,
            new_lets,
            removed,
            has_queries,
            program,
            ctx,
        } = prepared;
        let mut checked = Checked {
            file,
            diagnostics: program.map_diagnostics(analysis.diagnostics),
            removed: Vec::new(),
            redefined: Vec::new(),
            run: None,
        };
        if checked.failed() {
            return checked;
        }
        let defined: HashSet<&str> = new_lets.iter().map(|l| l.name.as_str()).collect();
        let mut kept = Vec::new();
        for (saved, removed) in std::mem::take(&mut self.saved).into_iter().zip(removed) {
            if !removed {
                kept.push(saved);
            } else if defined.contains(saved.name.as_str()) {
                checked.redefined.push(saved.name);
            } else {
                checked.removed.push(saved.name);
            }
        }
        if checked.removed.is_empty() {
            checked.redefined.clear();
        }
        self.saved = kept;
        self.saved.extend(new_lets);
        if has_queries {
            checked.run = Some((program, ctx));
        }
        checked
    }

    /// Runs the queries of a checked input.
    pub async fn run_checked(&self, checked: &Checked) -> Ran {
        match self.run_task(checked) {
            Some(task) => self.finish_run(checked, task.await),
            None => Ran::default(),
        }
    }

    /// Runs the queries of a checked input, if it has any. Like
    /// [`check_task`](Self::check_task), the future can run on another thread; dropping it
    /// cancels the running query.
    pub fn run_task(
        &self,
        checked: &Checked,
    ) -> Option<impl Future<Output = RunOutput> + Send + 'static> {
        let (program, ctx) = checked.run.as_ref()?;
        let session = self.session.clone();
        let src = program.source();
        let ctx = ctx.clone();
        Some(async move { session.run(&src, PROGRAM_FILE, &ctx).await })
    }

    /// Maps what [`run_task`](Self::run_task) returned onto the input, and adds `DS5001` for
    /// charts.
    pub fn finish_run(&self, checked: &Checked, output: RunOutput) -> Ran {
        let Some((program, _)) = &checked.run else {
            return Ran::default();
        };
        let mut diagnostics: Vec<Diagnostic> = program
            .map_diagnostics(output.diagnostics)
            .into_iter()
            .filter(|d| !checked.diagnostics.contains(d))
            .collect();
        let results: Vec<QueryResult> = output
            .results
            .into_iter()
            .map(|r| program.map_result(r))
            .collect();
        // One input is one program run (M1-10).
        diagnostics.extend(crate::chart_not_drawn(&results));
        Ran {
            diagnostics,
            results,
        }
    }

    /// Runs a meta command: `text` starts with `.`.
    pub async fn meta(&mut self, text: &str) -> MetaOutcome {
        let trimmed = text.trim();
        let (name, arg) = match trimmed.split_once(char::is_whitespace) {
            Some((name, arg)) => (name, arg.trim()),
            None => (trimmed, ""),
        };
        let no_argument = |command: &str| {
            MetaOutcome::Mistake(Mistake::new(
                format!("`{command}` takes no argument"),
                format!("type `{command}` on its own"),
            ))
        };
        match name {
            ".help" if arg.is_empty() => MetaOutcome::Print(HELP.to_string()),
            ".quit" | ".exit" if arg.is_empty() => MetaOutcome::Quit,
            ".tables" if arg.is_empty() => MetaOutcome::Print(self.tables_listing()),
            ".help" | ".quit" | ".exit" | ".tables" => no_argument(name),
            ".format" => self.set_format(arg),
            ".schema" => {
                // The argument's offset in the input, for mapping spans onto it.
                let lead = text.len() - text.trim_start().len();
                let rest = &trimmed[name.len()..];
                let offset = lead + name.len() + (rest.len() - rest.trim_start().len());
                self.schema(text, arg, len32(&text[..offset])).await
            }
            _ => MetaOutcome::Mistake(unknown_command(name)),
        }
    }

    fn set_format(&mut self, arg: &str) -> MetaOutcome {
        let format = match arg {
            "" => {
                return MetaOutcome::Print(format!(
                    "the output format is `{}`\n",
                    self.format.name()
                ));
            }
            "table" => RunFormat::Table,
            "csv" => RunFormat::Csv,
            "json" => RunFormat::Json,
            other => {
                let names = ["table", "csv", "json"];
                let tip = match did_you_mean(other, names).single() {
                    Some(best) => format!("did you mean `.format {best}`?"),
                    None => "use `.format table`, `.format csv` or `.format json`".to_string(),
                };
                return MetaOutcome::Mistake(Mistake::new(
                    format!("unknown output format `{other}`"),
                    tip,
                ));
            }
        };
        self.format = format;
        MetaOutcome::Print(String::new())
    }

    /// `.schema <source>`: checks `from <source>` after the saved lets, as a hidden program
    /// whose spans map onto the argument.
    async fn schema(&mut self, text: &str, arg: &str, arg_offset: u32) -> MetaOutcome {
        if arg.is_empty() {
            return MetaOutcome::Mistake(Mistake::new(
                "`.schema` needs a source",
                "give anything that can follow `from`, such as `.schema 'orders.csv'` or \
                 `.schema orders`",
            ));
        }
        self.session.invalidate_schemas();
        let file = self.sources.add_snippet("<repl>", text);
        let mut program = ReplProgram::new(file);
        for saved in &self.saved {
            self.push_saved(&mut program, saved);
        }
        program.push_glue("from ", file, arg_offset);
        program.push(arg, file, arg_offset);
        let ctx = self.session.exec_context(&self.base_dir);
        let analysis = self
            .session
            .check(&program.source(), PROGRAM_FILE, &ctx)
            .await;
        let diagnostics = program.map_diagnostics(analysis.diagnostics);
        if diagnostics.iter().any(Diagnostic::is_error) {
            return MetaOutcome::Diagnostics(diagnostics);
        }
        let columns = analysis
            .hir
            .queries()
            .last()
            .and_then(|q| q.pipeline.columns.clone());
        match columns {
            Some(columns) => MetaOutcome::Print(schema_table(&columns)),
            None => MetaOutcome::Mistake(Mistake::new(
                format!("the columns of `{arg}` are not known"),
                "check that it is a table, such as a file path, a host table or a tabular let",
            )),
        }
    }

    fn tables_listing(&self) -> String {
        if self.tables.is_empty() && self.saved.is_empty() {
            return "no host tables or saved lets yet (pass `--table name=path` when starting \
                    `ds`, or enter a `let`)\n"
                .to_string();
        }
        let tables = self
            .tables
            .iter()
            .map(|t| vec![t.name.clone(), "host table".into(), t.path.clone()]);
        let lets = self.saved.iter().map(|l| {
            let kind = if l.tabular {
                "tabular let"
            } else {
                "scalar let"
            };
            vec![l.name.clone(), kind.into(), String::new()]
        });
        let rows: Vec<Vec<String>> = tables.chain(lets).collect();
        text_table(&["name", "kind", "path"], &rows)
    }

    /// Appends a saved let and the glue after it.
    fn push_saved(&self, program: &mut ReplProgram, saved: &SavedLet) {
        let text = &self.source(saved.span.file).text()[saved.span.range()];
        program.push(text, saved.span.file, saved.span.start);
        program.push_glue(GLUE, saved.span.file, saved.span.end);
    }

    /// Which saved lets go when the input defines `defined`: those it redefines, and every one
    /// that uses a removed let, directly or transitively. A let can only use earlier lets, so
    /// one pass in order finds them all. A use is any name token that matches: this can remove
    /// too much (a column of the same name), never too little.
    fn removed_by(&self, defined: &HashSet<&str>) -> Vec<bool> {
        let mut gone: HashSet<&str> = HashSet::new();
        self.saved
            .iter()
            .map(|saved| {
                let removed = defined.contains(saved.name.as_str())
                    || self
                        .names_in(saved)
                        .iter()
                        .any(|n| gone.contains(n.as_str()));
                if removed {
                    gone.insert(&saved.name);
                }
                removed
            })
            .collect()
    }

    /// The name tokens in a saved let, its own name included.
    fn names_in(&self, saved: &SavedLet) -> Vec<String> {
        let text = &self.source(saved.span.file).text()[saved.span.range()];
        lex(text, saved.span.file)
            .tokens
            .iter()
            .filter_map(|t| match t.kind {
                SyntaxKind::IDENT => Some(t.text(text).to_string()),
                SyntaxKind::QUOTED_IDENT => {
                    let span = Span::new(saved.span.file, t.start, t.end());
                    literal::quoted_ident(t.text(text), span).ok()
                }
                _ => None,
            })
            .collect()
    }
}

/// An unknown meta command, with did-you-mean over the command names.
fn unknown_command(name: &str) -> Mistake {
    let suggestions = did_you_mean(name, COMMANDS.iter().copied());
    let tip = match suggestions.names() {
        [] => "`.help` lists the commands".to_string(),
        [best] => format!("a command with a similar name exists: `{best}`"),
        names => format!(
            "commands with similar names exist: {}",
            names
                .iter()
                .map(|n| format!("`{n}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    Mistake::new(format!("unknown command `{name}`"), tip)
}

/// `.schema`'s table: one row per column.
fn schema_table(columns: &[Column]) -> String {
    let rows: Vec<Vec<String>> = columns
        .iter()
        .map(|c| {
            let ty = match &c.arrow_only {
                Some(arrow) => format!("arrow:{arrow}"),
                None => c.ty.to_string(),
            };
            vec![c.name.clone(), ty]
        })
        .collect();
    text_table(&["column", "type"], &rows)
}

/// A table laid out like a result, without the type row: meta commands print text, not data.
fn text_table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let width = |i: usize| {
        rows.iter()
            .map(|r| r[i].chars().count())
            .chain([headers[i].chars().count()])
            .max()
            .unwrap_or(0)
    };
    let widths: Vec<usize> = (0..headers.len()).map(width).collect();
    let line = |cells: &mut dyn Iterator<Item = &str>| {
        let padded: Vec<String> = cells
            .zip(&widths)
            .map(|(cell, &w)| format!("{cell:<w$}"))
            .collect();
        format!("{}\n", padded.join(" | ").trim_end())
    };
    let mut out = line(&mut headers.iter().copied());
    let rule: Vec<String> = widths.iter().map(|&w| "-".repeat(w)).collect();
    out.push_str(&rule.join("-+-"));
    out.push('\n');
    for row in rows {
        out.push_str(&line(&mut row.iter().map(String::as_str)));
    }
    out
}

const HELP: &str = "\
Enter DataScript statements. Enter runs the input when it is complete and ends with `;`, or on a
blank line. Lets are kept for the rest of the session; entering `let x = ...` again replaces `x`.

Commands:
  .help               show this help
  .schema <source>    the columns of a file, host table or tabular let
  .tables             the host tables and saved lets
  .format [table|csv|json]
                      set the output format, or show it
  .quit, .exit        leave the REPL

Keys:
  Ctrl-C              cancel the running query, or discard the current input
  Ctrl-D              leave the REPL (on an empty prompt)
  Tab                 complete operator, function, let, table and command names
";

#[cfg(test)]
mod tests {
    use std::path::Path;

    use datascript_datafusion::DataFusionEngine;
    use datascript_diagnostics::codes::{DS1003, DS3001};
    use datascript_engine::FixedClock;

    use super::*;

    fn repl() -> ReplSession {
        let engine = Arc::new(DataFusionEngine::default());
        ReplSession::new(Session::new(engine, Arc::new(FixedClock(0))), Vec::new())
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    /// Submits an input through both phases, like the frontends do.
    fn submit(repl: &mut ReplSession, text: &str) -> (Checked, Ran) {
        runtime().block_on(async {
            let checked = repl.check_input(text).await;
            let ran = repl.run_checked(&checked).await;
            (checked, ran)
        })
    }

    fn saved_names(repl: &ReplSession) -> Vec<&str> {
        repl.saved().iter().map(|l| l.name.as_str()).collect()
    }

    /// The (file name, line, column) of a diagnostic's primary span.
    fn location(repl: &ReplSession, d: &Diagnostic) -> (String, u32, u32) {
        let span = d.primary.as_ref().expect("a primary span").span;
        let src = repl.source(span.file);
        let lc = src.line_col(span.start);
        (src.name().to_string(), lc.line, lc.column)
    }

    fn fixture(name: &str) -> String {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/data")
            .join(name)
            .display()
            .to_string()
    }

    /// `'…/orders.csv'`, quoted for a program.
    fn orders() -> String {
        format!("'{}'", fixture("orders.csv"))
    }

    #[test]
    fn lets_are_kept_across_inputs() {
        let mut repl = repl();
        let (checked, _) = submit(&mut repl, "let n = 2;");
        assert!(!checked.failed() && !checked.runs());
        let (checked, ran) = submit(
            &mut repl,
            &format!("let m = n + 1;\nfrom {} | where amount > m", orders()),
        );
        assert!(!checked.failed(), "{:?}", checked.diagnostics);
        assert!(!ran.failed(), "{:?}", ran.diagnostics);
        assert_eq!(ran.results.len(), 1);
        assert_eq!(saved_names(&repl), ["n", "m"]);
    }

    #[test]
    fn an_error_in_the_new_input_points_at_its_own_snippet() {
        let mut repl = repl();
        submit(&mut repl, "let a = 1;\nlet b = 2;");
        let (checked, _) = submit(&mut repl, "let c = a + nope;");
        assert!(checked.failed());
        let d = &checked.diagnostics[0];
        assert_eq!(location(&repl, d), ("<repl:2>".to_string(), 1, 13));
    }

    #[test]
    fn an_error_in_a_saved_let_points_at_that_let() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.csv");
        std::fs::write(&path, "x\n1\n").unwrap();
        let mut repl = repl();
        submit(&mut repl, "let one = 1;");
        let (checked, _) = submit(&mut repl, &format!("let t =\n  from '{}';", path.display()));
        assert!(!checked.failed(), "{:?}", checked.diagnostics);
        std::fs::remove_file(&path).unwrap();
        let (checked, _) = submit(&mut repl, "from t | take 1;");
        assert!(checked.failed());
        let d = checked
            .diagnostics
            .iter()
            .find(|d| d.code == DS3001)
            .unwrap();
        // Line 2, column 8 of the let's own snippet: the path string.
        assert_eq!(location(&repl, d), ("<repl:2>".to_string(), 2, 8));
        // The failed input saved nothing and removed nothing.
        assert_eq!(saved_names(&repl), ["one", "t"]);
    }

    #[test]
    fn a_failed_input_saves_nothing() {
        let mut repl = repl();
        let (checked, _) = submit(&mut repl, "let a = 1;\nlet b = nope;");
        assert!(checked.failed());
        assert!(repl.saved().is_empty());
        let (checked, _) = submit(&mut repl, "let c = a;");
        assert!(checked.failed(), "`a` must not have been saved");
    }

    #[test]
    fn redefining_a_let_removes_its_users_and_says_so() {
        let mut repl = repl();
        submit(&mut repl, "let x = 1;");
        submit(&mut repl, "let y = x + 1;");
        submit(&mut repl, "let z = y * 2;\nlet other = 5;");
        submit(
            &mut repl,
            &format!("let w = from {} | where amount > z;", orders()),
        );
        let (checked, _) = submit(&mut repl, "let x = 10;");
        assert!(!checked.failed(), "{:?}", checked.diagnostics);
        assert_eq!(checked.redefined, ["x"]);
        assert_eq!(checked.removed, ["y", "z", "w"]);
        assert_eq!(saved_names(&repl), ["other", "x"]);
        // A let with no users is replaced silently.
        let (checked, _) = submit(&mut repl, "let other = 6;");
        assert!(checked.removed.is_empty() && checked.redefined.is_empty());
        assert_eq!(saved_names(&repl), ["x", "other"]);
    }

    #[test]
    fn a_failed_redefinition_removes_nothing() {
        let mut repl = repl();
        submit(&mut repl, "let x = 1;\nlet y = x;");
        let (checked, _) = submit(&mut repl, "let x = nope;");
        assert!(checked.failed());
        assert_eq!(saved_names(&repl), ["x", "y"]);
    }

    #[test]
    fn two_lets_of_one_name_in_one_input_are_still_ds1003() {
        let mut repl = repl();
        let (checked, _) = submit(&mut repl, "let x = 1;\nlet x = 2;");
        assert!(checked.diagnostics.iter().any(|d| d.code == DS1003));
        assert!(repl.saved().is_empty());
    }

    #[test]
    fn a_file_created_after_an_error_is_found_next_time() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("late.csv");
        let query = format!("from '{}' | count;", path.display());
        let mut repl = repl();
        let (checked, _) = submit(&mut repl, &query);
        assert!(checked.diagnostics.iter().any(|d| d.code == DS3001));
        std::fs::write(&path, "x\n1\n2\n").unwrap();
        let (checked, ran) = submit(&mut repl, &query);
        assert!(!checked.failed(), "{:?}", checked.diagnostics);
        assert_eq!(ran.results.len(), 1);
    }

    #[test]
    fn result_lines_count_from_the_input() {
        let mut repl = repl();
        submit(&mut repl, "let a = 1;\nlet b = 2;");
        let o = orders();
        let (checked, ran) = submit(
            &mut repl,
            &format!("from {o} | take 1;\n\nfrom {o} | take 2;"),
        );
        let src = repl.source(checked.file);
        let lines: Vec<u32> = ran
            .results
            .iter()
            .map(|r| src.line_col(r.statement.unwrap().start).line)
            .collect();
        assert_eq!(lines, [1, 3]);
    }

    #[test]
    fn render_table_says_nothing_and_two_charts_say_it_once() {
        let mut repl = repl();
        let o = orders();
        let (_, ran) = submit(&mut repl, &format!("from {o} | take 3 | render table;"));
        assert!(ran.diagnostics.is_empty(), "{:?}", ran.diagnostics);
        let (_, ran) = submit(
            &mut repl,
            &format!(
                "from {o} | project customerId, amount | render barchart;\n\
                 from {o} | project timestamp, amount | render linechart;"
            ),
        );
        let charts: Vec<_> = ran
            .diagnostics
            .iter()
            .filter(|d| d.code == datascript_diagnostics::codes::DS5001)
            .collect();
        assert_eq!(charts.len(), 1);
        assert_eq!(location(&repl, charts[0]).0, "<repl:2>");
    }

    #[test]
    fn meta_commands_do_not_count_as_inputs() {
        let mut repl = repl();
        runtime().block_on(repl.meta(".tables"));
        let (checked, _) = submit(&mut repl, "from nope;");
        assert_eq!(repl.source(checked.file).name(), "<repl:1>");
    }

    #[test]
    fn unknown_commands_get_did_you_mean() {
        let mut repl = repl();
        let MetaOutcome::Mistake(m) = runtime().block_on(repl.meta(".shema 'x.csv'")) else {
            panic!("expected a mistake");
        };
        assert_eq!(m.message, "unknown command `.shema`");
        assert_eq!(m.tip, "a command with a similar name exists: `.schema`");
        let MetaOutcome::Mistake(m) = runtime().block_on(repl.meta(".frobnicate")) else {
            panic!("expected a mistake");
        };
        assert_eq!(m.tip, "`.help` lists the commands");
    }

    #[test]
    fn schema_lists_columns_and_maps_errors_onto_the_argument() {
        let mut repl = repl();
        let path = fixture("orders.csv");
        let MetaOutcome::Print(text) = runtime().block_on(repl.meta(&format!(".schema '{path}'")))
        else {
            panic!("expected a table");
        };
        assert!(text.contains("column") && text.contains("type"), "{text}");
        submit(&mut repl, &format!("let things = from {};", orders()));
        let MetaOutcome::Print(text) = runtime().block_on(repl.meta(".schema things")) else {
            panic!("expected a table");
        };
        assert!(text.contains("amount") && text.contains("real"), "{text}");
        let MetaOutcome::Diagnostics(ds) = runtime().block_on(repl.meta(".schema  thing")) else {
            panic!("expected diagnostics");
        };
        assert_eq!(location(&repl, &ds[0]), ("<repl>".to_string(), 1, 10));
    }

    #[test]
    fn format_is_set_and_shown() {
        let mut repl = repl();
        let rt = runtime();
        assert!(
            matches!(rt.block_on(repl.meta(".format csv")), MetaOutcome::Print(s) if s.is_empty())
        );
        assert_eq!(repl.format(), RunFormat::Csv);
        let MetaOutcome::Print(s) = rt.block_on(repl.meta(".format")) else {
            panic!()
        };
        assert_eq!(s, "the output format is `csv`\n");
        let MetaOutcome::Mistake(m) = rt.block_on(repl.meta(".format jsno")) else {
            panic!()
        };
        assert_eq!(m.tip, "did you mean `.format json`?");
    }

    #[test]
    fn text_tables_align_without_a_type_row() {
        let rows = vec![
            vec![
                "orders".to_string(),
                "host table".to_string(),
                "o.csv".to_string(),
            ],
            vec!["n".to_string(), "scalar let".to_string(), String::new()],
        ];
        assert_eq!(
            text_table(&["name", "kind", "path"], &rows),
            "name   | kind       | path\n\
             -------+------------+------\n\
             orders | host table | o.csv\n\
             n      | scalar let |\n"
        );
    }

    #[test]
    fn spans_map_back_through_glue() {
        let mut program = ReplProgram::new(FileId(2));
        program.push("let a = 1", FileId(0), 4);
        program.push_glue(GLUE, FileId(0), 13);
        program.push("from t", FileId(2), 0);
        assert_eq!(program.map_offset(2), (FileId(0), 6));
        assert_eq!(program.map_offset(10), (FileId(0), 13));
        let input_start = len32("let a = 1") + len32(GLUE);
        assert_eq!(program.map_offset(input_start + 5), (FileId(2), 5));
        let span = program.map_span(Span::new(PROGRAM_FILE, 4, input_start + 2));
        assert_eq!(span, Span::empty(FileId(0), 8));
    }
}
