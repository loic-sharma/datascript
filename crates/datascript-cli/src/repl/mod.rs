//! The REPL: `ds` with no command (M1-12, D-048).
//!
//! - [`session`]: the session logic (saved lets, span mapping, meta commands), with no terminal
//!   dependency.
//! - [`input`]: the submit rule.
//! - [`editor`]: the interactive line editor (reedline), with highlighting, completion and
//!   history.
//!
//! When standard input is not a terminal, the same loop reads inputs from it with no editor and
//! exits 1 if any input had an error.

mod editor;
mod input;
mod session;

use std::io::{self, IsTerminal, Read, Write};
use std::process::ExitCode;

use crate::args::Cli;
use crate::output;
use crate::script::{Reporter, Sources};
use crate::{EXIT_ERRORS, EXIT_USAGE};

pub use session::{Checked, MetaOutcome, Ran, ReplSession};

/// Runs the REPL, interactive on a terminal, piped otherwise.
pub fn main(cli: Cli) -> ExitCode {
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
    let interactive = io::stdin().is_terminal();
    let reporter = if interactive {
        Reporter::new(cli.color)
    } else {
        Reporter::plain()
    };
    let session = match crate::session(&cli.repl.tables.tables) {
        Ok(session) => session,
        Err(d) => return ExitCode::from(reporter.fail_usage(&Sources::new(), &d)),
    };
    let repl = ReplSession::new(session, cli.repl.tables.tables.clone());
    let printer = Printer {
        reporter,
        max_rows: cli.repl.max_rows.unwrap_or(100),
    };
    let code = if interactive {
        editor::run(repl, printer, &runtime, cli.color)
    } else {
        piped(repl, &printer, &runtime)
    };
    ExitCode::from(code)
}

/// Reads every input from standard input and submits them in order.
fn piped(mut repl: ReplSession, printer: &Printer, runtime: &tokio::runtime::Runtime) -> u8 {
    let mut text = String::new();
    if let Err(e) = io::stdin().read_to_string(&mut text) {
        eprintln!("error: cannot read standard input: {e}");
        return EXIT_USAGE;
    }
    let mut failed = false;
    for input in input::split_inputs(&text) {
        match runtime.block_on(submit(&mut repl, printer, &input)) {
            Step::Continue { failed: f } => failed |= f,
            Step::Quit => break,
        }
    }
    if failed { EXIT_ERRORS } else { 0 }
}

/// What the loop does after an input.
enum Step {
    Continue { failed: bool },
    Quit,
}

/// Submits one input with no Ctrl-C handling, for piped mode.
async fn submit(repl: &mut ReplSession, printer: &Printer, text: &str) -> Step {
    if input::is_meta(text) {
        let outcome = repl.meta(text).await;
        return printer.meta(repl, outcome);
    }
    let checked = repl.check_input(text).await;
    if printer.checked(repl, &checked) {
        return Step::Continue { failed: true };
    }
    let ran = repl.run_checked(&checked).await;
    Step::Continue {
        failed: printer.ran(repl, &checked, &ran),
    }
}

/// Prints what the REPL produced: results on standard output, diagnostics and messages on
/// standard error.
#[derive(Debug)]
pub struct Printer {
    reporter: Reporter,
    max_rows: usize,
}

impl Printer {
    /// Prints the outcome of checking. Returns whether it failed.
    fn checked(&self, repl: &ReplSession, checked: &Checked) -> bool {
        self.reporter.report(repl.sources(), &checked.diagnostics);
        if !checked.removed.is_empty() {
            eprintln!("{}", removed_line(&checked.removed, &checked.redefined));
        }
        checked.failed()
    }

    /// Prints the outcome of running. Returns whether it failed.
    fn ran(&self, repl: &ReplSession, checked: &Checked, ran: &Ran) -> bool {
        self.reporter.report(repl.sources(), &ran.diagnostics);
        let mut out = io::stdout().lock();
        let written = output::write_repl_results(
            &mut out,
            repl.format(),
            &ran.results,
            repl.source(checked.file),
            self.max_rows,
        )
        .and_then(|()| out.flush());
        if let Err(e) = written
            && e.kind() != io::ErrorKind::BrokenPipe
        {
            eprintln!("error: cannot write to standard output: {e}");
        }
        ran.failed()
    }

    fn meta(&self, repl: &ReplSession, outcome: MetaOutcome) -> Step {
        match outcome {
            MetaOutcome::Print(text) => {
                let _ = output::write_stdout(text.as_bytes());
                Step::Continue { failed: false }
            }
            MetaOutcome::Diagnostics(diagnostics) => {
                self.reporter.report(repl.sources(), &diagnostics);
                Step::Continue {
                    failed: diagnostics.iter().any(|d| d.is_error()),
                }
            }
            MetaOutcome::Mistake(mistake) => {
                eprintln!("{mistake}");
                Step::Continue { failed: true }
            }
            MetaOutcome::Quit => Step::Quit,
        }
    }
}

/// "info: removed `y` and `z`, which used `x`; enter them again to use the new `x`".
fn removed_line(removed: &[String], redefined: &[String]) -> String {
    format!(
        "info: removed {}, which used {}; enter {} again to use the new definition",
        listing(removed),
        listing(redefined),
        if removed.len() == 1 { "it" } else { "them" }
    )
}

/// "`a`", "`a` and `b`", "`a`, `b` and `c`".
fn listing(names: &[String]) -> String {
    let quoted: Vec<String> = names.iter().map(|n| format!("`{n}`")).collect();
    match quoted.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removed_lets_are_named_in_one_line() {
        let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            removed_line(&names(&["y"]), &names(&["x"])),
            "info: removed `y`, which used `x`; enter it again to use the new definition"
        );
        assert_eq!(
            removed_line(&names(&["y", "z", "w"]), &names(&["x"])),
            "info: removed `y`, `z` and `w`, which used `x`; enter them again to use the new \
             definition"
        );
    }
}
