//! The REPL's session logic, with no terminal dependency (D-048): saved lets, the combined
//! program and its span mapping, and the meta commands.

use std::collections::HashSet;
use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use datascript_compiler::Analysis;
use datascript_compiler::hir::Column;
use datascript_compiler::ide::{self, Completions};
use datascript_compiler::ir::RenderSpec;
use datascript_diagnostics::{Diagnostic, FileId, SourceFile, Span, did_you_mean};
use datascript_engine::{ExecContext, Session, catch_panic};
use datascript_syntax::ast::{AstNode, Stmt};
use datascript_syntax::{SyntaxKind, lex, literal, parse};

use crate::args::{RunFormat, TableArg};
use crate::output::{self, Frontend, Layout, OutputError, Sink};
use crate::script::Sources;

/// The file id of the combined program. It is never in the source map: every span in it is
/// mapped back to the input it came from before anyone sees it.
const PROGRAM_FILE: FileId = FileId(u32::MAX);

/// What goes between two statements of the combined program. The newline ends a line comment
/// that the statement before may end with, so the `;` is not swallowed by it.
const GLUE: &str = "\n;\n";

/// The meta commands, as the user types them.
pub const COMMANDS: &[&str] = &[
    ".help",
    ".schema",
    ".tables",
    ".format",
    ".max-rows",
    ".quit",
    ".exit",
];

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

/// The second phase of a submission: the queries ran, and their results were written as they
/// arrived (M2-11).
#[derive(Debug, Default)]
pub struct Ran {
    /// Diagnostics from running, mapped, without the ones [`Checked`] already has. Includes
    /// `DS5001` for charts.
    pub diagnostics: Vec<Diagnostic>,
    /// The statement of each query that started, mapped to the input. Only tests read it.
    #[cfg_attr(not(test), expect(dead_code))]
    pub statements: Vec<Span>,
    /// Writing the results failed (not for a closed pipe): the message to show.
    pub write_error: Option<String>,
}

impl Ran {
    pub fn failed(&self) -> bool {
        self.diagnostics.iter().any(Diagnostic::is_error)
    }
}

/// What a [`ReplSession::run_task`] future reports, before spans are mapped to the input.
#[derive(Debug, Default)]
pub struct RunReport {
    diagnostics: Vec<Diagnostic>,
    /// The `render` specs of the queries, for `DS5001`.
    charts: Vec<(RenderSpec, Option<Span>)>,
    statements: Vec<Span>,
    write_error: Option<String>,
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
    /// Rows shown per table; 0 shows every row (`--max-rows`, `.max-rows`).
    max_rows: usize,
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
            max_rows: 100,
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
    #[cfg(test)]
    pub fn format(&self) -> RunFormat {
        self.format
    }

    /// Rows shown per table, as `--max-rows` or `.max-rows` set it; 0 shows every row.
    #[cfg(test)]
    pub fn max_rows(&self) -> usize {
        self.max_rows
    }

    /// Sets the rows shown per table (from `--max-rows`).
    pub fn set_max_rows(&mut self, max_rows: usize) {
        self.max_rows = max_rows;
    }

    /// The saved lets, in order.
    #[cfg(test)]
    pub fn saved(&self) -> &[SavedLet] {
        &self.saved
    }

    /// Sets the working directory used to resolve relative paths in queries.
    #[cfg(test)]
    pub fn set_base_dir(&mut self, dir: impl Into<PathBuf>) {
        self.base_dir = dir.into();
    }

    /// A snapshot of the REPL's state for Tab completion: the shared [`Session`] (so completion
    /// reuses its schema cache), the saved lets, the host tables, and the working directory.
    pub fn completion_context(&self) -> CompletionContext {
        let saved = self
            .saved
            .iter()
            .map(|l| SavedLetText {
                name: l.name.clone(),
                text: self.source(l.span.file).text()[l.span.range()].to_string(),
                names: self.names_in(l),
            })
            .collect();
        CompletionContext {
            session: self.session.clone(),
            saved,
            tables: self.tables.clone(),
            base_dir: self.base_dir.clone(),
            next_input: self.inputs + 1,
        }
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

    /// Runs the queries of a checked input, writing results to `out`.
    pub async fn run_checked(&self, checked: &Checked, out: Box<dyn Write + Send>) -> Ran {
        match self.run_task(checked, out) {
            Some(task) => self.finish_run(checked, task.await),
            None => Ran::default(),
        }
    }

    /// Runs the queries of a checked input, if it has any, writing each result to `out` as it
    /// arrives. Like [`check_task`](Self::check_task), the future can run on another thread;
    /// dropping it cancels the running query.
    pub fn run_task(
        &self,
        checked: &Checked,
        out: Box<dyn Write + Send>,
    ) -> Option<impl Future<Output = RunReport> + Send + 'static> {
        let (program, ctx) = checked.run.as_ref()?;
        let session = self.session.clone();
        let program = program.clone();
        let input = self.source(checked.file).clone();
        let ctx = ctx.clone();
        let layout = Layout {
            format: self.format.into(),
            max_rows: self.max_rows,
            frontend: Frontend::Repl,
        };
        Some(async move {
            let src = program.source();
            let mut run = session.start(&src, PROGRAM_FILE, &ctx).await;
            let mut report = RunReport {
                diagnostics: std::mem::take(&mut run.diagnostics),
                charts: crate::charts_of(run.queries())
                    .map(|(spec, span)| (spec.clone(), span))
                    .collect(),
                ..RunReport::default()
            };
            let mut sink = Sink::writer(out);
            let line_of = |span: Span| Some(input.line_col(program.map_span(span).start).line);
            let streamed = output::stream_results(&mut sink, layout, &mut run, &line_of).await;
            drop(run);
            match streamed.and_then(|streamed| sink.finish().map(|()| streamed)) {
                Ok(streamed) => {
                    report.diagnostics.extend(streamed.error);
                    report.statements = streamed.statements;
                }
                Err(OutputError::BrokenPipe) => {}
                Err(OutputError::Io { target, error }) => {
                    report.write_error = Some(format!("cannot write to {target}: {error}"));
                }
            }
            report
        })
    }

    /// Maps what [`run_task`](Self::run_task) returned onto the input, and adds `DS5001` for
    /// charts.
    pub fn finish_run(&self, checked: &Checked, report: RunReport) -> Ran {
        let Some((program, _)) = &checked.run else {
            return Ran::default();
        };
        let mut diagnostics: Vec<Diagnostic> = program
            .map_diagnostics(report.diagnostics)
            .into_iter()
            .filter(|d| !checked.diagnostics.contains(d))
            .collect();
        // One input is one program run (M1-10).
        let charts = report
            .charts
            .iter()
            .map(|(spec, span)| (spec, span.map(|s| program.map_span(s))));
        diagnostics.extend(crate::chart_not_drawn(charts));
        Ran {
            diagnostics,
            statements: report
                .statements
                .into_iter()
                .map(|s| program.map_span(s))
                .collect(),
            write_error: report.write_error,
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
            ".max-rows" => self.set_max_rows_command(arg),
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

    /// `.max-rows [n]`: sets the rows shown per table, or shows the setting.
    fn set_max_rows_command(&mut self, arg: &str) -> MetaOutcome {
        if arg.is_empty() {
            let shown = match self.max_rows {
                0 => "every row".to_string(),
                1 => "1 row".to_string(),
                n => format!("{n} rows"),
            };
            return MetaOutcome::Print(format!("tables show {shown}\n"));
        }
        match arg.parse::<usize>() {
            Ok(n) => {
                self.max_rows = n;
                MetaOutcome::Print(String::new())
            }
            Err(_) => MetaOutcome::Mistake(Mistake::new(
                format!("`.max-rows` needs a whole number, found `{arg}`"),
                "use `.max-rows 20`, or `.max-rows 0` to show every row",
            )),
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
        let names: Vec<Vec<String>> = self.saved.iter().map(|s| self.names_in(s)).collect();
        removed_lets(
            self.saved
                .iter()
                .zip(&names)
                .map(|(s, ns)| (s.name.as_str(), ns.as_slice())),
            defined,
        )
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

/// A saved `let`'s text and name tokens, detached from [`ReplSession::sources`] so completion
/// can run from a snapshot without borrowing the session mutably.
#[derive(Debug, Clone)]
struct SavedLetText {
    name: String,
    text: String,
    names: Vec<String>,
}

/// A snapshot of the REPL's state for Tab completion.
///
/// Analyses the current buffer as its own `<repl:N>` snippet with the saved lets in front
/// (D-048), reusing the [`Session`]'s schema cache without invalidating it, then calls the
/// compiler's [`ide::complete`] and maps the replacement range back to the current buffer.
#[derive(Debug, Clone)]
pub struct CompletionContext {
    session: Arc<Session>,
    saved: Vec<SavedLetText>,
    tables: Vec<TableArg>,
    base_dir: PathBuf,
    next_input: u32,
}

impl CompletionContext {
    /// The host tables registered with `--table`.
    pub fn tables(&self) -> &[TableArg] {
        &self.tables
    }

    /// The directory relative file paths resolve against.
    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    /// Names of `let` bindings in scope at byte offset `pos` of `line`: kept saved lets plus any
    /// `let` statement in `line` that ends at or before `pos`. Used to skip host tables shadowed
    /// by a non-table `let` until `ide::complete` accepts host tables (M4-04).
    pub fn let_names_before(&self, line: &str, pos: usize) -> HashSet<String> {
        let pos = line.floor_char_boundary(pos.min(line.len()));
        let parsed = parse(line, FileId(0));
        let mut defined = HashSet::new();
        let mut in_line = Vec::new();
        for stmt in parsed.program().statements() {
            if let Stmt::Let(let_stmt) = stmt
                && let Some(name) = let_stmt.name().and_then(|n| n.text())
            {
                let end = usize::from(let_stmt.syntax().text_range().end());
                if end <= pos {
                    in_line.push(name.clone());
                }
                defined.insert(name);
            }
        }
        let defined_refs: HashSet<&str> = defined.iter().map(String::as_str).collect();
        let removed = removed_lets(
            self.saved
                .iter()
                .map(|l| (l.name.as_str(), l.names.as_slice())),
            &defined_refs,
        );
        self.saved
            .iter()
            .zip(removed)
            .filter(|(_, r)| !*r)
            .map(|(l, _)| l.name.clone())
            .chain(in_line)
            .collect()
    }

    /// Computes compiler completions for `line` at byte offset `pos`.
    ///
    /// Schema lookups go through the session's cache without invalidating it, so a file the
    /// session already read is not read again. A failing source or an internal panic yields no
    /// columns and produces no error output.
    pub async fn complete(&self, line: &str, pos: usize) -> Completions {
        let pos = line.floor_char_boundary(pos.min(line.len()));
        let parsed = parse(line, FileId(0));
        let defined: HashSet<String> = parsed
            .program()
            .statements()
            .filter_map(|stmt| match stmt {
                Stmt::Let(let_stmt) => let_stmt.name().and_then(|n| n.text()),
                Stmt::Query(_) => None,
            })
            .collect();
        let defined_refs: HashSet<&str> = defined.iter().map(String::as_str).collect();
        let removed = removed_lets(
            self.saved
                .iter()
                .map(|l| (l.name.as_str(), l.names.as_slice())),
            &defined_refs,
        );

        let mut combined = String::new();
        for (saved, _) in self.saved.iter().zip(removed).filter(|(_, r)| !*r) {
            combined.push_str(&saved.text);
            combined.push_str(GLUE);
        }
        let prefix_len = combined.len();
        combined.push_str(line);

        let src = SourceFile::new(format!("<repl:{}>", self.next_input), combined);
        let ctx = self.session.exec_context(&self.base_dir);
        let analysis = self.session.check(&src, PROGRAM_FILE, &ctx).await;
        let offset = prefix_len + pos;
        match catch_panic(Some(PROGRAM_FILE), "completing", || {
            ide::complete(&src, &analysis, offset)
        }) {
            Ok(mut completions) => {
                completions.start = completions.start.saturating_sub(prefix_len).min(pos);
                completions.end = completions
                    .end
                    .saturating_sub(prefix_len)
                    .clamp(pos, line.len());
                completions
            }
            Err(_) => Completions {
                start: pos,
                end: pos,
                ..Completions::default()
            },
        }
    }
}

/// Which saved lets go when the input defines `defined`: those it redefines, and every one that
/// uses a removed let, directly or transitively.
fn removed_lets<'a>(
    saved: impl IntoIterator<Item = (&'a str, &'a [String])>,
    defined: &HashSet<&str>,
) -> Vec<bool> {
    let mut gone: HashSet<&'a str> = HashSet::new();
    saved
        .into_iter()
        .map(|(name, names)| {
            let removed = defined.contains(name) || names.iter().any(|n| gone.contains(n.as_str()));
            if removed {
                gone.insert(name);
            }
            removed
        })
        .collect()
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
  .max-rows [n]       set the rows shown per table (0 shows every row), or show it
  .quit, .exit        leave the REPL

Keys:
  Ctrl-C              cancel the running query, or discard the current input
  Ctrl-D              leave the REPL (on an empty prompt)
  Tab                 complete column, operator, function, let, table, file and command names
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
            let ran = repl.run_checked(&checked, Box::new(std::io::sink())).await;
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

    /// `path` as a quoted string literal. Forward slashes, so Windows
    /// backslashes are not read as escapes.
    fn quoted(path: &Path) -> String {
        format!("'{}'", path.display().to_string().replace('\\', "/"))
    }

    fn fixture(name: &str) -> String {
        quoted(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/data")
                .join(name),
        )
    }

    /// `'…/orders.csv'`, quoted for a program.
    fn orders() -> String {
        fixture("orders.csv")
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
        assert_eq!(ran.statements.len(), 1);
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
        let (checked, _) = submit(&mut repl, &format!("let t =\n  from {};", quoted(&path)));
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
        let query = format!("from {} | count;", quoted(&path));
        let mut repl = repl();
        let (checked, _) = submit(&mut repl, &query);
        assert!(checked.diagnostics.iter().any(|d| d.code == DS3001));
        std::fs::write(&path, "x\n1\n2\n").unwrap();
        let (checked, ran) = submit(&mut repl, &query);
        assert!(!checked.failed(), "{:?}", checked.diagnostics);
        assert_eq!(ran.statements.len(), 1);
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
            .statements
            .iter()
            .map(|s| src.line_col(s.start).line)
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
        let MetaOutcome::Print(text) = runtime().block_on(repl.meta(&format!(".schema {path}")))
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
    fn max_rows_is_set_and_shown() {
        let mut repl = repl();
        let rt = runtime();
        assert_eq!(repl.max_rows(), 100);
        assert!(
            matches!(rt.block_on(repl.meta(".max-rows 5")), MetaOutcome::Print(s) if s.is_empty())
        );
        assert_eq!(repl.max_rows(), 5);
        let MetaOutcome::Print(s) = rt.block_on(repl.meta(".max-rows")) else {
            panic!()
        };
        assert_eq!(s, "tables show 5 rows\n");
        rt.block_on(repl.meta(".max-rows 0"));
        let MetaOutcome::Print(s) = rt.block_on(repl.meta(".max-rows")) else {
            panic!()
        };
        assert_eq!(s, "tables show every row\n");
        let MetaOutcome::Mistake(m) = rt.block_on(repl.meta(".max-rows lots")) else {
            panic!()
        };
        assert_eq!(m.message, "`.max-rows` needs a whole number, found `lots`");
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

    #[test]
    fn completion_uses_the_session_schema_cache() {
        use std::pin::Pin;
        use std::sync::atomic::{AtomicUsize, Ordering};

        use datascript_compiler::ide::CompletionKind;

        struct CountingEngine {
            inner: DataFusionEngine,
            schema_calls: Arc<AtomicUsize>,
        }

        impl datascript_engine::Engine for CountingEngine {
            fn name(&self) -> &str {
                self.inner.name()
            }

            fn capabilities(&self) -> &datascript_compiler::Capabilities {
                self.inner.capabilities()
            }

            fn schema_of<'a, 'b, 'c, 'r>(
                &'a self,
                source: &'b datascript_compiler::SourceRef,
                ctx: &'c ExecContext,
            ) -> Pin<
                Box<
                    dyn Future<
                            Output = Result<
                                arrow_schema::SchemaRef,
                                datascript_compiler::SourceError,
                            >,
                        > + Send
                        + 'r,
                >,
            >
            where
                'a: 'r,
                'b: 'r,
                'c: 'r,
                Self: 'r,
            {
                self.schema_calls.fetch_add(1, Ordering::SeqCst);
                self.inner.schema_of(source, ctx)
            }

            fn execute<'a, 'b, 'c, 'r>(
                &'a self,
                plan: &'b datascript_compiler::ir::QueryPlan,
                ctx: &'c ExecContext,
            ) -> Pin<
                Box<
                    dyn Future<Output = Result<datascript_engine::ResultStream, Diagnostic>>
                        + Send
                        + 'r,
                >,
            >
            where
                'a: 'r,
                'b: 'r,
                'c: 'r,
                Self: 'r,
            {
                self.inner.execute(plan, ctx)
            }
        }

        let calls = Arc::new(AtomicUsize::new(0));
        let engine = Arc::new(CountingEngine {
            inner: DataFusionEngine::default(),
            schema_calls: calls.clone(),
        });
        let mut repl = ReplSession::new(Session::new(engine, Arc::new(FixedClock(0))), Vec::new());
        repl.set_base_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."));

        let rt = runtime();
        let checked = rt.block_on(
            repl.check_input("let big = from 'tests/data/orders.csv' | where amount > 100;"),
        );
        assert!(!checked.failed(), "{:?}", checked.diagnostics);
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // Completing over the same file or over `big` reuses the cached schema.
        let ctx = repl.completion_context();
        let line = "from 'tests/data/orders.csv' | where am";
        let c = rt.block_on(ctx.complete(line, line.len()));
        assert_eq!((c.start, c.end), (37, 39));
        assert!(
            c.items
                .iter()
                .any(|i| i.kind == CompletionKind::Column && i.label == "amount")
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let line = "from big | project ";
        let c = rt.block_on(ctx.complete(line, line.len()));
        assert_eq!((c.start, c.end), (19, 19));
        let cols: Vec<&str> = c
            .items
            .iter()
            .filter(|i| i.kind == CompletionKind::Column)
            .map(|i| i.label.as_str())
            .collect();
        assert_eq!(cols, ["orderId", "customerId", "amount", "timestamp"]);
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // A new file is read once on the first Tab and cached for the next.
        let line = "from 'tests/data/customers.csv' | where na";
        let c = rt.block_on(ctx.complete(line, line.len()));
        assert!(
            c.items
                .iter()
                .any(|i| i.kind == CompletionKind::Column && i.label == "name")
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);

        let line = "from 'tests/data/customers.csv' | where co";
        let c = rt.block_on(ctx.complete(line, line.len()));
        assert!(
            c.items
                .iter()
                .any(|i| i.kind == CompletionKind::Column && i.label == "country")
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);

        // Empty input after saved lets still maps offsets to 0 and offers statement starters.
        let c = rt.block_on(ctx.complete("", 0));
        assert_eq!((c.start, c.end), (0, 0));
        assert_eq!(
            c.items.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(),
            ["from", "let"]
        );
    }
}
