//! Writes the fixture datasets to `tests/data/`.
//!
//! ```bash
//! cargo run -p datascript-fixtures
//! ```

use std::process::ExitCode;

fn main() -> ExitCode {
    let dir = datascript_fixtures::data_dir();
    for fixture in datascript_fixtures::generate() {
        let path = dir.join(fixture.path);
        let written = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&path, &fixture.bytes));
        if let Err(err) = written {
            eprintln!("error: could not write `{}`: {err}", path.display());
            return ExitCode::FAILURE;
        }
        println!("wrote {} ({} bytes)", fixture.path, fixture.bytes.len());
    }
    ExitCode::SUCCESS
}
