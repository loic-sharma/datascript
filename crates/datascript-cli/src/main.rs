//! The `ds` command-line tool.
//!
//! The walking skeleton (M1-15) has `ds check`, `ds run` and `ds explain` over one script file.
//! M1-11 adds the rest (formats, `-e`, stdin, `--table`, colors, several files).

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use datascript_datafusion::DataFusionEngine;
use datascript_diagnostics::{Diagnostic, FileId, Renderer, SourceFile, SourceMap};
use datascript_engine::{Session, SystemClock, table::format_table};

/// DataScript: a pipe-based query language for local data files.
#[derive(Debug, Parser)]
#[command(name = "ds", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Check a script and print its diagnostics, without running it.
    Check {
        /// The script to check.
        file: PathBuf,
    },
    /// Run a script and print each query's result as a table.
    Run {
        /// The script to run.
        file: PathBuf,
    },
    /// Print the logical plan of each query in a script.
    Explain {
        /// The script to explain.
        file: PathBuf,
    },
}

/// Exit codes: 0 success, 1 the program has errors, 2 usage or internal error.
const EXIT_ERRORS: u8 = 1;
const EXIT_USAGE: u8 = 2;

fn main() -> ExitCode {
    let cli = Cli::parse();
    let runtime = match tokio::runtime::Builder::new_current_thread().build() {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("error: cannot start the async runtime: {e}");
            return ExitCode::from(EXIT_USAGE);
        }
    };
    runtime.block_on(run(cli.command))
}

/// A loaded script, with what is needed to report diagnostics against it.
struct Script {
    sources: SourceMap,
    src: SourceFile,
    file: FileId,
    base_dir: PathBuf,
}

impl Script {
    fn load(path: &Path) -> Result<Script, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read `{}`: {e}", path.display()))?;
        let name = path.display().to_string();
        let mut sources = SourceMap::new();
        let file = sources.add(name.clone(), text.clone());
        let base_dir = match path.parent() {
            Some(dir) if !dir.as_os_str().is_empty() => dir.to_path_buf(),
            _ => PathBuf::from("."),
        };
        Ok(Script {
            sources,
            src: SourceFile::new(name, text),
            file,
            base_dir,
        })
    }

    /// Prints diagnostics to stderr. Returns whether any is an error.
    fn report(&self, diagnostics: &[Diagnostic]) -> bool {
        if !diagnostics.is_empty() {
            let stderr = std::io::stderr();
            let renderer = Renderer::terminal(stderr.is_terminal());
            eprint!("{}", renderer.render_all(diagnostics, &self.sources));
        }
        diagnostics.iter().any(Diagnostic::is_error)
    }
}

async fn run(command: Command) -> ExitCode {
    let path = match &command {
        Command::Check { file } | Command::Run { file } | Command::Explain { file } => file,
    };
    let script = match Script::load(path) {
        Ok(script) => script,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::from(EXIT_USAGE);
        }
    };
    let session = Session::new(Arc::new(DataFusionEngine::default()), Arc::new(SystemClock));
    let ctx = session.exec_context(&script.base_dir);
    let failed = match command {
        Command::Check { .. } => {
            let analysis = session.check(&script.src, script.file, &ctx).await;
            script.report(&analysis.diagnostics)
        }
        Command::Explain { .. } => match session.explain(&script.src, script.file, &ctx).await {
            Ok(text) => {
                print!("{text}");
                false
            }
            Err(diagnostics) => script.report(&diagnostics),
        },
        Command::Run { .. } => {
            let output = session.run(&script.src, script.file, &ctx).await;
            let failed = script.report(&output.diagnostics);
            for (i, result) in output.results.iter().enumerate() {
                if i > 0 {
                    println!();
                }
                print!("{}", format_table(&result.schema, &result.batches));
            }
            failed
        }
    };
    if failed {
        ExitCode::from(EXIT_ERRORS)
    } else {
        ExitCode::SUCCESS
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
}
