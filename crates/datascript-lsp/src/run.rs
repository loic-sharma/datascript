//! Running a document: the `datascript/run` request and the Run CodeLens (M3-05, D-054).
//!
//! `datascript/run` runs the server's current text of an open document, unsaved edits included,
//! and returns the text `ds run --format table` prints for it: the check's warnings, the `DS5001`
//! line for a chart, the tables, and the runtime error that stopped the program, rendered as the
//! CLI renders them on a terminal. The printing is [`datascript_engine::printer`], the CLI's own.

use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use datascript_diagnostics::{Diagnostic, FileId, Renderer, SourceFile, SourceMap, Span};
use datascript_engine::printer::{self, Format, Frontend, Layout, Sink};
use datascript_engine::{ExecContext, Session};
use datascript_syntax::ast::{AstNode, Stmt};
use serde::{Deserialize, Serialize};
use tower_lsp_server::ls_types::{CodeLens, Command, TextDocumentIdentifier, Uri};

use crate::convert;

/// The method name of the run request.
pub const RUN_METHOD: &str = "datascript/run";

/// The client command a "▶ Run" CodeLens invokes, with the document URI and the 1-based
/// statement number as arguments.
pub const RUN_STATEMENT_COMMAND: &str = "datascript.runStatement";

/// The client command a "▶ Run file" CodeLens invokes, with the document URI as argument.
pub const RUN_FILE_COMMAND: &str = "datascript.runFile";

/// Rows per table when the client does not say, as `ds run --max-rows` defaults to.
const DEFAULT_MAX_ROWS: usize = 100;

/// The params of `datascript/run`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunParams {
    /// An open document. Its text as the server has it runs, saved or not.
    pub text_document: TextDocumentIdentifier,
    /// The query statement to run (1-based, counting query statements only), with the `let`s
    /// it needs, like `ds run --statement`. Without it the whole document runs.
    #[serde(default)]
    pub statement: Option<usize>,
    /// Rows per table, like `ds run --max-rows`; 0 shows every row. Default 100.
    #[serde(default)]
    pub max_rows: Option<usize>,
    /// Whether to color diagnostics with ANSI escapes, as `ds run` does on a color terminal.
    #[serde(default)]
    pub color: bool,
}

/// The result of `datascript/run`.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunResult {
    /// What `ds run --format table` prints for this run on a terminal, standard error and
    /// standard output in the order they are written. Lines end in `\n`.
    pub output: String,
    /// Whether the program failed: errors from checking, or a runtime error. `ds run` exits
    /// with 1 then.
    pub failed: bool,
}

/// What a run produced, for the server to reply with and to publish.
#[derive(Debug, Default)]
pub(crate) struct Ran {
    pub result: RunResult,
    /// The runtime error that stopped the program, if one did.
    pub runtime_error: Option<Diagnostic>,
}

/// Why a run did not start.
#[derive(Debug)]
pub(crate) enum RunRefused {
    /// `statement` is 0 or past the last query statement.
    StatementOutOfRange { asked: usize, queries: usize },
}

impl RunRefused {
    pub fn message(&self) -> String {
        match self {
            RunRefused::StatementOutOfRange { asked, queries } => format!(
                "statement {asked} is out of range: the document has {}",
                printer::count(*queries, "query statement", "query statements")
            ),
        }
    }
}

/// Runs `file` (file id `id`) as `ds run --format table` would and returns what it prints.
pub(crate) async fn run(
    session: &Session,
    file: &SourceFile,
    id: FileId,
    ctx: &ExecContext,
    params: &RunParams,
) -> Result<Ran, RunRefused> {
    let mut sources = SourceMap::new();
    let mapped = sources.add(file.name(), file.text());
    debug_assert_eq!(mapped, id, "the document is the only file of its run");
    let renderer = Renderer::terminal(params.color);
    let mut ran = Ran::default();

    let mut run = session.start(file, id, ctx).await;
    if run.has_errors() {
        ran.result.output = renderer.render_all(&run.diagnostics, &sources);
        ran.result.failed = true;
        return Ok(ran);
    }
    if let Some(asked) = params.statement {
        let queries = run.queries().len();
        if asked == 0 || asked > queries {
            return Err(RunRefused::StatementOutOfRange { asked, queries });
        }
        run.select(asked - 1);
    }

    let mut diagnostics = std::mem::take(&mut run.diagnostics);
    diagnostics.extend(printer::chart_not_drawn(printer::charts_of(run.queries())));
    let mut output = renderer.render_all(&diagnostics, &sources);

    let buffer = SharedBuffer::default();
    let mut sink = Sink::writer(Box::new(buffer.clone()));
    let layout = Layout {
        format: Format::Table,
        max_rows: params.max_rows.unwrap_or(DEFAULT_MAX_ROWS),
        frontend: Frontend::Run,
    };
    let line_of = |span: Span| Some(file.line_col(span.start).line);
    // Writing to memory cannot fail.
    let streamed = printer::stream_results(&mut sink, layout, &mut run, &line_of)
        .await
        .unwrap_or_default();
    drop(run);
    output.push_str(&String::from_utf8_lossy(&buffer.take()));
    if let Some(error) = streamed.error {
        output.push_str(&renderer.render(&error, &sources));
        ran.result.failed = true;
        ran.runtime_error = Some(error);
    }
    ran.result.output = output;
    Ok(ran)
}

/// A `Write` over shared memory, so the text can be read back after the sink is done.
#[derive(Debug, Clone, Default)]
struct SharedBuffer(Arc<Mutex<Vec<u8>>>);

impl SharedBuffer {
    fn take(&self) -> Vec<u8> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

impl Write for SharedBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The Run CodeLenses of `file`: "▶ Run" over each query statement, and "▶ Run file" after it over
/// the first one when there are several. `let` statements get none. Each lens's range covers its
/// whole statement, so a client can find the statement at the cursor.
pub(crate) fn code_lenses(file: &SourceFile, id: FileId, uri: &Uri) -> Vec<CodeLens> {
    let parse = datascript_syntax::parse(file.text(), id);
    let queries: Vec<Span> = parse
        .program()
        .statements()
        .filter_map(|stmt| match stmt {
            Stmt::Query(query) => Some(query.span(id)),
            Stmt::Let(_) => None,
        })
        .collect();
    let uri_arg = serde_json::Value::String(uri.as_str().to_owned());
    let mut lenses = Vec::with_capacity(queries.len() + 1);
    for (index, &span) in queries.iter().enumerate() {
        let range = convert::range_of(file, span);
        lenses.push(CodeLens {
            range,
            command: Some(Command {
                title: "▶ Run".into(),
                command: RUN_STATEMENT_COMMAND.into(),
                arguments: Some(vec![uri_arg.clone(), serde_json::json!(index + 1)]),
            }),
            data: None,
        });
        if index == 0 && queries.len() > 1 {
            lenses.push(CodeLens {
                range,
                command: Some(Command {
                    title: "▶ Run file".into(),
                    command: RUN_FILE_COMMAND.into(),
                    arguments: Some(vec![uri_arg.clone()]),
                }),
                data: None,
            });
        }
    }
    lenses
}
