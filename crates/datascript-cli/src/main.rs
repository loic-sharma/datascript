//! The `ds` command-line tool: `ds check`, `ds run`, `ds explain`, and the REPL when there is
//! no command (ARCHITECTURE.md § Frontends).
//!
//! - [`repl`]: the interactive REPL, or the same loop over piped standard input.
//! - [`args`]: the command line, with examples in every `--help`.
//! - [`script`]: loading a script from a file, `-e` or stdin, and reporting diagnostics.
//! - [`output`]: printing results as a table, CSV, JSON, NDJSON or Arrow IPC.
//!
//! Exit codes: 0 success, 1 the program has errors (or warnings with `--deny-warnings`), 2 usage
//! or internal error, 130 interrupted by Ctrl-C.

mod args;
mod output;
mod repl;
mod script;

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;
use datascript_compiler::SourceRef;
use datascript_compiler::ir::ChartKind;
use datascript_compiler::source::format_of_path;
use datascript_datafusion::DataFusionEngine;
use datascript_diagnostics::codes::{DS3002, DS5001};
use datascript_diagnostics::{Diagnostic, JsonReport, Severity};
use datascript_engine::{QueryResult, Session, SystemClock};

use crate::args::{
    CheckArgs, CheckFormat, Cli, ColorChoice, Command, ExplainArgs, RunArgs, TableArg,
};
use crate::output::{Destination, OutputError};
use crate::script::{Reporter, Sources};

/// The program has errors.
const EXIT_ERRORS: u8 = 1;
/// A usage error, an unreadable script or an internal error.
const EXIT_USAGE: u8 = 2;
/// Interrupted by Ctrl-C (128 + SIGINT, as shells report it).
const EXIT_INTERRUPTED: u8 = 130;

fn main() -> ExitCode {
    let cli = Cli::parse();
    let Some(command) = cli.command else {
        // The REPL handles Ctrl-C per submission instead of exiting (M1-12).
        return repl::main(cli);
    };
    if cli.repl.given() {
        args::usage_error(
            "`--table` and `--max-rows` before the command are settings of the REPL",
            "put them after the command, such as `ds run report.ds --table orders=orders.csv`",
        );
    }
    if let Command::Lsp(_) = command {
        return lsp();
    }
    // A worker thread runs the program, so the main thread stays free to notice Ctrl-C even
    // while a query is busy computing.
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("error: cannot start the async runtime: {e}");
            return ExitCode::from(EXIT_USAGE);
        }
    };
    runtime.block_on(async {
        let work = tokio::spawn(run(command, cli.color));
        tokio::select! {
            joined = work => joined.unwrap_or_else(|e| {
                eprintln!("error: internal error: {e}");
                eprintln!("  = note: this is a bug in DataScript; please report it");
                ExitCode::from(EXIT_USAGE)
            }),
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\ninterrupted: stopped before the program finished");
                let _ = std::io::stderr().flush();
                // Exiting drops the running query, which cancels it.
                std::process::exit(i32::from(EXIT_INTERRUPTED));
            }
        }
    })
}

async fn run(command: Command, color: ColorChoice) -> ExitCode {
    let reporter = Reporter::new(color);
    let code = match command {
        Command::Check(args) => check(args, &reporter).await,
        Command::Run(args) => run_script(args, &reporter).await,
        Command::Explain(args) => explain(args, &reporter).await,
        Command::Lsp(_) => unreachable!("`main` serves the LSP before this"),
    };
    ExitCode::from(code)
}

/// `ds lsp`: serves the LSP over stdio until the client exits. Ctrl-C is not caught here: the
/// editor owns the process, and stdout carries only the protocol.
fn lsp() -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("error: cannot start the async runtime: {e}");
            return ExitCode::from(EXIT_USAGE);
        }
    };
    let session = Session::new(Arc::new(DataFusionEngine::default()), Arc::new(SystemClock));
    runtime.block_on(datascript_lsp::serve_stdio(Arc::new(session)));
    ExitCode::SUCCESS
}

/// A session over the DataFusion engine with the `--table` host tables, or the diagnostic for a
/// table whose format cannot be told.
pub(crate) fn session(tables: &[TableArg]) -> Result<Session, Diagnostic> {
    let mut engine = DataFusionEngine::default();
    for table in tables {
        if format_of_path(&table.path).is_none() {
            return Err(unknown_table_format(table));
        }
        // The engine resolves a host table's relative path against the script's directory, but
        // on the command line the user means the current directory (D-044).
        let path = std::path::absolute(&table.path).unwrap_or_else(|_| PathBuf::from(&table.path));
        engine = engine.with_table(
            table.name.clone(),
            SourceRef::for_path(path.to_string_lossy()),
        );
    }
    Ok(Session::new(Arc::new(engine), Arc::new(SystemClock)))
}

/// `DS3002` for `--table name=path` whose path has no known extension.
fn unknown_table_format(table: &TableArg) -> Diagnostic {
    let stem = std::path::Path::new(&table.path)
        .file_stem()
        .map_or_else(|| "data".into(), |s| s.to_string_lossy());
    Diagnostic::error(
        DS3002,
        format!("cannot tell the format of `{}`", table.path),
    )
    .help(format!(
        "rename the file so its name ends in a known extension, such as `{stem}.csv`, and pass \
         `--table {}=<new path>`",
        table.name
    ))
    .note(
        "formats come from the extension: .csv, .tsv, .parquet, .json, .ndjson and .jsonl, and \
         CSV, TSV and JSON may end in .gz, .bz2, .xz or .zst",
    )
}

async fn check(args: CheckArgs, reporter: &Reporter) -> u8 {
    let session = match session(&args.tables.tables) {
        Ok(session) => session,
        Err(d) => return reporter.fail_usage(&Sources::new(), &d),
    };
    let mut sources = Sources::new();
    let mut all = Vec::new();
    let mut unreadable = false;
    for path in &args.files {
        let script = match sources.load_path(path) {
            Ok(script) => script,
            Err(d) => {
                unreadable = true;
                all.push(d);
                continue;
            }
        };
        let ctx = session.exec_context(&script.base_dir);
        let analysis = session
            .check(sources.file(&script), script.file, &ctx)
            .await;
        all.extend(analysis.diagnostics);
    }
    match args.format {
        CheckFormat::Human => reporter.report(&sources, &all),
        CheckFormat::Json => {
            println!("{}", JsonReport::new(&all, sources.map()).to_json_pretty());
        }
    }
    check_exit_code(unreadable, &all, args.deny_warnings)
}

/// 2 if a script could not be read, 1 for errors (and warnings with `--deny-warnings`), else 0.
fn check_exit_code(unreadable: bool, diagnostics: &[Diagnostic], deny_warnings: bool) -> u8 {
    let fails = |d: &Diagnostic| d.is_error() || (deny_warnings && d.severity == Severity::Warning);
    if unreadable {
        EXIT_USAGE
    } else if diagnostics.iter().any(fails) {
        EXIT_ERRORS
    } else {
        0
    }
}

async fn explain(args: ExplainArgs, reporter: &Reporter) -> u8 {
    let mut sources = Sources::new();
    let script = match sources.load(&args.script) {
        Ok(script) => script,
        Err(d) => return reporter.fail_usage(&sources, &d),
    };
    let session = match session(&args.tables.tables) {
        Ok(session) => session,
        Err(d) => return reporter.fail_usage(&sources, &d),
    };
    let ctx = session.exec_context(&script.base_dir);
    match session
        .explain(sources.file(&script), script.file, &ctx)
        .await
    {
        Ok(text) => match output::write_stdout(text.as_bytes()) {
            Ok(()) => 0,
            Err(e) => e.exit_code(),
        },
        Err(diagnostics) => {
            reporter.report(&sources, &diagnostics);
            EXIT_ERRORS
        }
    }
}

async fn run_script(args: RunArgs, reporter: &Reporter) -> u8 {
    let mut sources = Sources::new();
    let script = match sources.load(&args.script) {
        Ok(script) => script,
        Err(d) => return reporter.fail_usage(&sources, &d),
    };
    let session = match session(&args.tables.tables) {
        Ok(session) => session,
        Err(d) => return reporter.fail_usage(&sources, &d),
    };
    let destination = Destination::new(args.output.as_deref());
    if args.format.is_binary() && destination.is_terminal() {
        args::usage_error(
            "refusing to write binary Arrow data to the terminal",
            "send it to a file with `--output result.arrow`, or pipe it to another program",
        );
    }
    let src = sources.file(&script);
    let ctx = session.exec_context(&script.base_dir);

    // Formats that hold one result, and `--statement`, need the number of queries before
    // anything runs. Schemas are cached, so compiling twice reads each source once.
    let mut selected = None;
    if args.statement.is_some() || args.format.single_result() {
        let program = match session.compile(src, script.file, &ctx).await {
            Ok(program) => program,
            Err(diagnostics) => {
                reporter.report(&sources, &diagnostics);
                return EXIT_ERRORS;
            }
        };
        let lines: Vec<u32> = program
            .queries
            .iter()
            .map(|q| src.line_col(q.span.start).line)
            .collect();
        match args.statement {
            Some(n) if n.get() > lines.len() => args::usage_error(
                &format!(
                    "`--statement {n}` is out of range: the program has {}",
                    output::count(lines.len(), "query statement", "query statements")
                ),
                &statement_help(&lines),
            ),
            Some(n) => selected = Some(n.get() - 1),
            None if lines.len() > 1 => args::usage_error(
                &format!(
                    "`--format {}` prints one result, but the program has {} query statements",
                    args.format.name(),
                    lines.len()
                ),
                &format!(
                    "{}, or use `--format table` or `--format json`, which print them all",
                    statement_help(&lines)
                ),
            ),
            None => {}
        }
    }

    let output = match selected {
        Some(index) => session.run_statement(src, script.file, &ctx, index).await,
        None => session.run(src, script.file, &ctx).await,
    };
    let mut diagnostics = output.diagnostics.clone();
    diagnostics.extend(chart_not_drawn(&output.results));
    reporter.report(&sources, &diagnostics);
    let failed = output.diagnostics.iter().any(Diagnostic::is_error);
    let written = output::write_results(
        &destination,
        args.format,
        &output.results,
        src,
        args.max_rows,
    );
    match written {
        Ok(()) if failed => EXIT_ERRORS,
        Ok(()) => 0,
        Err(e) => e.exit_code(),
    }
}

/// `DS5001`, once per program run: the CLI prints charts as tables (LANGUAGE.md §6.3). It points
/// at the first `render` that is not `render table`, with a note counting the others.
pub(crate) fn chart_not_drawn(results: &[QueryResult]) -> Option<Diagnostic> {
    let charts: Vec<&QueryResult> = results
        .iter()
        .filter(|r| {
            r.render
                .as_ref()
                .is_some_and(|spec| spec.kind != ChartKind::Table)
        })
        .collect();
    let first = charts.first()?;
    let kind = first.render.as_ref()?.kind;
    let mut d = Diagnostic::info(DS5001, "charts are not drawn on the command line");
    d = match first.render_span {
        Some(span) => d.primary(span, format!("this `{kind}` is printed as a table")),
        None => d.note(format!("the `{kind}` is printed as a table")),
    };
    if charts.len() > 1 {
        let others = charts.len() - 1;
        d = d.note(format!(
            "{} not drawn either",
            output::count(others, "other chart is", "other charts are")
        ));
    }
    Some(d)
}

/// "pick one with `--statement <n>`: 1 (line 3), 2 (line 8)".
fn statement_help(lines: &[u32]) -> String {
    let choices: Vec<String> = lines
        .iter()
        .enumerate()
        .map(|(i, line)| format!("{} (line {line})", i + 1))
        .collect();
    if choices.is_empty() {
        "the program has no query to pick".to_string()
    } else {
        format!("pick one with `--statement <n>`: {}", choices.join(", "))
    }
}

impl OutputError {
    /// Reports the error and returns the exit code. A closed pipe (`ds run … | head`) is not an
    /// error: the reader has what it wanted.
    fn exit_code(self) -> u8 {
        match self {
            OutputError::BrokenPipe => 0,
            OutputError::Io { target, error } => {
                eprintln!("error: cannot write to {target}: {error}");
                EXIT_USAGE
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn warnings_fail_check_only_when_denied() {
        use datascript_diagnostics::codes::{DS0004, DS1007, DS9001};
        let warning = [Diagnostic::warning(DS0004, "w")];
        let info = [Diagnostic::info(DS1007, "i")];
        let error = [Diagnostic::error(DS9001, "e")];
        assert_eq!(check_exit_code(false, &warning, false), 0);
        assert_eq!(check_exit_code(false, &warning, true), EXIT_ERRORS);
        assert_eq!(check_exit_code(false, &info, true), 0);
        assert_eq!(check_exit_code(false, &error, false), EXIT_ERRORS);
        assert_eq!(check_exit_code(true, &error, false), EXIT_USAGE);
        assert_eq!(check_exit_code(false, &[], true), 0);
    }

    #[test]
    fn statement_help_lists_each_query_with_its_line() {
        assert_eq!(
            statement_help(&[3, 8]),
            "pick one with `--statement <n>`: 1 (line 3), 2 (line 8)"
        );
    }
}
