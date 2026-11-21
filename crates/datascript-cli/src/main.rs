//! The `ds` command-line tool.

use clap::Parser;

/// DataScript: a pipe-based query language for local data files.
#[derive(Debug, Parser)]
#[command(name = "ds", version, about)]
struct Cli {}

fn main() {
    let _cli = Cli::parse();
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
