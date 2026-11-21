//! Tests for the runner itself, with a fake driver, in a scratch directory.

use std::fs;
use std::path::{Path, PathBuf};

use datascript_tests::runner::{discover, prepare};
use datascript_tests::{CaseInput, Driver, DriverConfig, DriverResult, Drivers, Mode};

/// Supports `check` only. Echoes the query upper-cased, plus the absolute path of the case's
/// directory so path normalization is exercised.
struct Echo;

impl Driver for Echo {
    fn engine(&self) -> &str {
        "datafusion"
    }

    fn supports(&self, mode: Mode) -> bool {
        mode == Mode::Check
    }

    fn check(&self, input: &CaseInput<'_>) -> DriverResult {
        assert_eq!(input.config.target_partitions, 1);
        Ok(format!(
            "{}dir: {}\n",
            input.query.to_uppercase(),
            input.base_dir().join("data.csv").display()
        ))
    }
}

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("nested/deeper")).unwrap();
    dir.canonicalize().unwrap()
}

fn drivers() -> Drivers {
    Drivers::new(DriverConfig::default()).with(Echo)
}

fn run(path: &Path, root: &Path, bless: bool) -> Result<(), String> {
    let prepared = prepare(path.to_path_buf(), root, &drivers(), bless);
    assert!(!prepared.ignored, "{path:?} was ignored");
    (prepared.run)()
}

#[test]
fn discovers_recursively_and_sorted() {
    let root = scratch("discover");
    for p in [
        "b.dstest",
        "a.dstest",
        "nested/deeper/c.dstest",
        "nested/notes.md",
    ] {
        fs::write(root.join(p), "").unwrap();
    }
    let found: Vec<_> = discover(&root)
        .into_iter()
        .map(|p| {
            p.strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    assert_eq!(found, ["a.dstest", "b.dstest", "nested/deeper/c.dstest"]);
}

#[test]
fn passing_case_with_normalized_path() {
    let root = scratch("pass");
    let case = root.join("nested/ok.dstest");
    fs::write(
        &case,
        "mode: check\n---- query\nt | take 1\n---- expected\nT | TAKE 1\ndir: <test>/data.csv\n",
    )
    .unwrap();
    run(&case, &root, false).unwrap();
}

#[test]
fn mismatch_shows_file_line_and_diff() {
    let root = scratch("mismatch");
    let case = root.join("bad.dstest");
    fs::write(
        &case,
        "mode: check\n---- query\nx\n---- expected\nY\ndir: <test>/data.csv\n",
    )
    .unwrap();
    let err = run(&case, &root, false).unwrap_err();
    assert_eq!(
        err,
        "bad.dstest:4: output does not match\n\
         --- expected\n+++ actual\n@@ -1,2 +1,2 @@\n-Y\n+X\n dir: <test>/data.csv\n\n\
         run with DS_BLESS=1 to accept the actual output"
    );
}

#[test]
fn missing_expected_section_asks_for_bless() {
    let root = scratch("missing");
    let case = root.join("new.dstest");
    fs::write(&case, "mode: check\n---- query\nx\n").unwrap();
    let err = run(&case, &root, false).unwrap_err();
    assert!(
        err.starts_with("new.dstest: no `---- expected` section; run with DS_BLESS=1"),
        "{err}"
    );
}

#[test]
fn malformed_file_fails_naming_file_and_line() {
    let root = scratch("malformed");
    let case = root.join("nested/broken.dstest");
    fs::write(&case, "# ok\nmode: check\nordr: any\n---- query\nx\n").unwrap();
    let prepared = prepare(case, &root, &drivers(), false);
    assert_eq!(prepared.kind, "invalid");
    assert!(!prepared.ignored);
    assert_eq!(
        (prepared.run)().unwrap_err(),
        "nested/broken.dstest:3: unknown header key `ordr` (did you mean `order`?)"
    );
}

#[test]
fn unknown_engine_names_the_header_line() {
    let root = scratch("engine");
    let case = root.join("e.dstest");
    fs::write(&case, "mode: check\nengine: datafusoin\n---- query\nx\n").unwrap();
    let prepared = prepare(case, &root, &drivers(), false);
    assert_eq!(
        (prepared.run)().unwrap_err(),
        "e.dstest:2: unknown engine `datafusoin` (did you mean `datafusion`?)"
    );
}

#[test]
fn unsupported_modes_and_skip_are_ignored() {
    let root = scratch("ignored");
    let run_case = root.join("run.dstest");
    fs::write(&run_case, "mode: run\n---- query\nx\n---- expected\ny\n").unwrap();
    let prepared = prepare(run_case, &root, &drivers(), false);
    assert!(prepared.ignored);
    assert_eq!(prepared.kind, "run");

    let skipped = root.join("skip.dstest");
    fs::write(&skipped, "mode: check\nskip: known bug\n---- query\nx\n").unwrap();
    assert!(prepare(skipped, &root, &drivers(), false).ignored);
}

#[test]
fn bless_rewrites_expected_and_is_idempotent() {
    let root = scratch("bless");
    let case = root.join("b.dstest");
    let original = "# Description stays.\nmode: check\n\n---- query\nq  \n---- expected\nstale\n";
    fs::write(&case, original).unwrap();

    run(&case, &root, true).unwrap();
    let blessed = fs::read_to_string(&case).unwrap();
    assert_eq!(
        blessed,
        "# Description stays.\nmode: check\n\n---- query\nq  \n---- expected\nQ  \ndir: <test>/data.csv\n"
    );

    run(&case, &root, true).unwrap();
    assert_eq!(fs::read_to_string(&case).unwrap(), blessed);
    run(&case, &root, false).unwrap();
}
