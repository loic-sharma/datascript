//! The command line.

use std::num::NonZeroUsize;
use std::path::PathBuf;

use clap::{ArgGroup, Args, CommandFactory, Parser, Subcommand, ValueEnum};

const MAIN_EXAMPLES: &str = "\
Examples:
  ds                                      start the interactive REPL
  ds --table orders=data/orders.parquet   the REPL, with a host table
  ds run examples/top_customers.ds
  ds run -e \"from 'orders.csv' | take 10\"
  ds check queries/*.ds
  ds explain report.ds

Exit codes: 0 success, 1 the program has errors, 2 usage or internal error, 130 interrupted.";

const CHECK_EXAMPLES: &str = "\
Examples:
  ds check report.ds
  ds check queries/*.ds --deny-warnings
  ds check report.ds --format json
  ds check report.ds --table orders=data/orders.parquet

Exit codes: 0 no errors, 1 errors (or warnings with --deny-warnings), 2 a script could not be read.";

const RUN_EXAMPLES: &str = "\
Examples:
  ds run report.ds
  ds run -e \"from 'orders.csv' | summarize n = count() by region\"
  cat report.ds | ds run -
  ds run report.ds --format csv --output result.csv
  ds run report.ds --format json --max-rows 0
  ds run report.ds --statement 2 --format ndjson
  ds run -e 'from orders | take 5' --table orders=data/orders.parquet

Relative paths in the program resolve against the script's directory, or against the current
directory for -e and stdin. Paths given to --table resolve against the current directory.";

const EXPLAIN_EXAMPLES: &str = "\
Examples:
  ds explain report.ds
  ds explain -e \"from 'orders.csv' | where amount > 100 | take 5\"";

/// DataScript: a pipe-based query language for local data files.
///
/// With no command, `ds` starts the interactive REPL.
#[derive(Debug, Parser)]
#[command(name = "ds", version, about, after_help = MAIN_EXAMPLES)]
pub struct Cli {
    /// When to color diagnostics. `auto` colors them on a terminal unless `NO_COLOR` is set.
    #[arg(long, value_enum, global = true, default_value_t = ColorChoice::Auto)]
    pub color: ColorChoice,

    /// The REPL's settings, used when there is no command.
    #[command(flatten)]
    pub repl: ReplArgs,

    #[command(subcommand)]
    pub command: Option<Command>,
}

/// The REPL's flags (`ds` with no command).
#[derive(Debug, Args)]
pub struct ReplArgs {
    /// Rows to show per table in the REPL; the rest are counted in a footer. 0 shows every row.
    /// [default: 100]
    #[arg(long, value_name = "N")]
    pub max_rows: Option<usize>,

    #[command(flatten)]
    pub tables: TableArgs,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Check scripts and print their diagnostics, without running them.
    #[command(after_help = CHECK_EXAMPLES)]
    Check(CheckArgs),
    /// Run a script and print the result of each query.
    #[command(after_help = RUN_EXAMPLES)]
    Run(RunArgs),
    /// Print the logical plan of each query in a script.
    #[command(after_help = EXPLAIN_EXAMPLES)]
    Explain(ExplainArgs),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ColorChoice {
    Auto,
    Always,
    Never,
}

#[derive(Debug, Args)]
pub struct CheckArgs {
    /// The scripts to check. `-` reads one from standard input.
    #[arg(required = true, value_name = "FILE")]
    pub files: Vec<PathBuf>,

    /// How to print diagnostics: `human` on standard error, or `json` on standard output.
    #[arg(long, value_enum, default_value_t = CheckFormat::Human)]
    pub format: CheckFormat,

    /// Fail (exit code 1) on warnings too.
    #[arg(long)]
    pub deny_warnings: bool,

    #[command(flatten)]
    pub tables: TableArgs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CheckFormat {
    Human,
    Json,
}

#[derive(Debug, Args)]
pub struct RunArgs {
    #[command(flatten)]
    pub script: ScriptArgs,

    /// How to print results. `csv`, `ndjson` and `arrow` (Arrow IPC file) hold one result:
    /// with several query statements, pick one with `--statement`.
    #[arg(long, value_enum, default_value_t = RunFormat::Table)]
    pub format: RunFormat,

    /// Write results to this file instead of standard output.
    #[arg(long, short = 'o', value_name = "PATH")]
    pub output: Option<PathBuf>,

    /// Rows to show per table (`--format table` only); the rest are counted in a footer.
    /// 0 shows every row.
    #[arg(long, value_name = "N", default_value_t = 100)]
    pub max_rows: usize,

    /// Run only the n-th query statement (1-based), with the `let`s it needs.
    #[arg(long, value_name = "N")]
    pub statement: Option<NonZeroUsize>,

    #[command(flatten)]
    pub tables: TableArgs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum RunFormat {
    Table,
    Csv,
    Json,
    Ndjson,
    Arrow,
}

impl RunFormat {
    /// Whether the format holds exactly one result.
    pub fn single_result(self) -> bool {
        matches!(self, RunFormat::Csv | RunFormat::Ndjson | RunFormat::Arrow)
    }

    /// Whether the output is binary, unfit for a terminal.
    pub fn is_binary(self) -> bool {
        self == RunFormat::Arrow
    }

    /// The name as written on the command line.
    pub fn name(self) -> &'static str {
        match self {
            RunFormat::Table => "table",
            RunFormat::Csv => "csv",
            RunFormat::Json => "json",
            RunFormat::Ndjson => "ndjson",
            RunFormat::Arrow => "arrow",
        }
    }
}

#[derive(Debug, Args)]
pub struct ExplainArgs {
    #[command(flatten)]
    pub script: ScriptArgs,

    #[command(flatten)]
    pub tables: TableArgs,
}

/// Where the program comes from: a file, `-` for standard input, or `-e`.
#[derive(Debug, Args)]
#[command(group(ArgGroup::new("program").required(true).args(["file", "expr"])))]
pub struct ScriptArgs {
    /// The script to read. `-` reads it from standard input.
    #[arg(value_name = "FILE")]
    pub file: Option<PathBuf>,

    /// The program itself, instead of a file.
    #[arg(short = 'e', long = "expr", value_name = "PROGRAM")]
    pub expr: Option<String>,
}

#[derive(Debug, Args)]
pub struct TableArgs {
    /// Register a file as a table, so `from NAME` reads it. Repeatable.
    #[arg(long = "table", value_name = "NAME=PATH", value_parser = parse_table)]
    pub tables: Vec<TableArg>,
}

/// One `--table name=path`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableArg {
    pub name: String,
    pub path: String,
}

fn parse_table(value: &str) -> Result<TableArg, String> {
    match value.split_once('=') {
        Some((name, path)) if !name.is_empty() && !path.is_empty() => Ok(TableArg {
            name: name.to_string(),
            path: path.to_string(),
        }),
        _ => Err(format!(
            "expected NAME=PATH, such as `orders=data/orders.csv`, but found `{value}`"
        )),
    }
}

impl ReplArgs {
    /// Whether any REPL flag was given, which is a mistake before a command.
    pub fn given(&self) -> bool {
        self.max_rows.is_some() || !self.tables.tables.is_empty()
    }
}

/// Prints a usage error the way clap does, with a tip, and exits with code 2.
pub fn usage_error(message: &str, tip: &str) -> ! {
    let mut command = Cli::command();
    command
        .error(
            clap::error::ErrorKind::ArgumentConflict,
            format!("{message}\n\n  tip: {tip}"),
        )
        .exit()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_split_at_the_first_equals_sign() {
        assert_eq!(
            parse_table("t=a=b.csv"),
            Ok(TableArg {
                name: "t".into(),
                path: "a=b.csv".into()
            })
        );
        assert!(parse_table("t").is_err());
        assert!(parse_table("=a.csv").is_err());
        assert!(parse_table("t=").is_err());
    }
}
