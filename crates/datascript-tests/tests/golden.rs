//! Runs every `.dstest` case under `tests/cases/`. See TESTING.md.

fn main() -> std::process::ExitCode {
    datascript_tests::runner::main(datascript_tests::Drivers::current())
}
