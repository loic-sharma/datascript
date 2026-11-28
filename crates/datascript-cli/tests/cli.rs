//! `ds` end to end: exit codes, formats, stdin, `--table`, colors and help.
//!
//! Queries read the shared fixtures in `tests/data` (TESTING.md). Tests that use `-e` or stdin
//! run in that directory, because those programs resolve paths against the current directory.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;
use predicates::str::contains;

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/data")
}

/// The absolute path of `tests/data/orders.csv`, for scripts written outside `tests/data`.
fn orders_path() -> String {
    data_dir()
        .join("orders.csv")
        .canonicalize()
        .unwrap()
        .display()
        .to_string()
        // On Windows, drop the verbatim `\\?\` prefix `canonicalize` adds, and use forward
        // slashes: backslashes would be read as escapes inside a DataScript string literal.
        .trim_start_matches(r"\\?\")
        .replace('\\', "/")
}

fn repo_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// `ds` running in `tests/data`, with `NO_COLOR` unset so the environment cannot leak in.
fn ds_process() -> std::process::Command {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_ds"));
    cmd.current_dir(data_dir()).env_remove("NO_COLOR");
    cmd
}

/// `ds` as an [`assert_cmd::Command`], built from [`ds_process`].
fn ds() -> Command {
    Command::from_std(ds_process())
}

const TWO_ROWS: &str = "from 'orders.csv' | take 2 | project orderId, amount";

const TWO_QUERIES: &str = "let o = 'orders.csv';
from o | take 2 | project orderId;

from o | summarize n = count()";

// ---- help ----

#[test]
fn help_has_examples() {
    for args in [
        &["--help"][..],
        &["check", "--help"],
        &["run", "--help"],
        &["explain", "--help"],
    ] {
        ds().args(args)
            .assert()
            .success()
            .stdout(contains("Examples:\n  ds "));
    }
}

#[test]
fn no_subcommand_starts_the_repl() {
    // With empty piped input the REPL has nothing to do (tests/repl.rs covers it).
    ds().write_stdin("").assert().success().stdout("");
}

#[test]
fn repl_flags_conflict_with_subcommands() {
    ds().args(["--max-rows", "5", "run", "-e", "from 'orders.csv'"])
        .assert()
        .code(2)
        .stderr(contains("put them after the command"));
    ds().args(["--table", "o=orders.csv", "check", "-"])
        .write_stdin("from o")
        .assert()
        .code(2);
}

// ---- run: formats ----

#[test]
fn run_prints_a_table() {
    ds().args(["run", "-e", TWO_ROWS])
        .assert()
        .success()
        .stdout(
            "orderId | amount\n\
         string  | real\n\
         --------+-------\n\
         o-0001  |  183.5\n\
         o-0002  |   88.5\n",
        );
}

#[test]
fn run_a_script_file_resolves_paths_against_its_directory() {
    ds().current_dir(std::env::temp_dir())
        .arg("run")
        .arg(repo_dir().join("examples/orders.ds"))
        .assert()
        .success()
        .stdout(contains("orderId | amount\n").and(contains("o-0010  | 107.5")));
}

#[test]
fn run_the_readme_example() {
    ds().arg("run")
        .arg(repo_dir().join("examples/top_customers.ds"))
        .assert()
        .success()
        .stdout(
            contains("customerId | month                | revenue\n")
                .and(contains("string     | datetime             | real\n")),
        )
        .stderr(
            contains("info[DS5001]: charts are not drawn on the command line")
                .and(contains("this `timechart` is printed as a table"))
                // `on customerId` merges the key into one column (D-019).
                .and(contains("DS1007").not()),
        );
}

// ---- run: charts (DS5001) ----

const TWO_CHARTS: &str = "from 'orders.csv' | count | render card;
from 'orders.csv' | count | render table;
from 'orders.csv' | summarize n = count() by customerId | top 2 by n | render barchart";

#[test]
fn two_charts_give_one_ds5001_for_any_format() {
    for format in ["table", "json", "csv"] {
        let mut cmd = ds();
        cmd.args(["run", "-e", TWO_CHARTS, "--format", format]);
        if format == "csv" {
            cmd.args(["--statement", "3"]);
        }
        let output = cmd.output().unwrap();
        assert!(output.status.success(), "{format}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert_eq!(stderr.matches("DS5001").count(), 1, "{format}: {stderr}");
        if format == "csv" {
            assert!(
                stderr.contains("this `barchart` is printed as a table"),
                "{stderr}"
            );
            assert!(!stderr.contains("other chart"), "{stderr}");
        } else {
            assert!(
                stderr.contains("this `card` is printed as a table"),
                "{stderr}"
            );
            assert!(
                stderr.contains("= note: 1 other chart is not drawn either"),
                "{stderr}"
            );
        }
        assert!(!String::from_utf8(output.stdout).unwrap().contains("DS5001"));
    }
}

#[test]
fn render_table_gives_no_ds5001() {
    ds().args(["run", "-e", "from 'orders.csv' | count | render table"])
        .assert()
        .success()
        .stdout(contains("Count"))
        .stderr("");
}

#[test]
fn run_reads_stdin() {
    ds().args(["run", "-"])
        .write_stdin(TWO_ROWS)
        .assert()
        .success()
        .stdout(contains("o-0002  |   88.5"));
}

#[test]
fn run_csv() {
    ds().args(["run", "-e", TWO_ROWS, "--format", "csv"])
        .assert()
        .success()
        .stdout("orderId,amount\no-0001,183.5\no-0002,88.5\n");
}

#[test]
fn run_ndjson() {
    ds().args(["run", "-e", TWO_ROWS, "--format", "ndjson"])
        .assert()
        .success()
        .stdout(
            "{\"orderId\":\"o-0001\",\"amount\":183.5}\n{\"orderId\":\"o-0002\",\"amount\":88.5}\n",
        );
}

#[test]
fn run_json_has_one_object_per_query_statement() {
    let output = ds()
        .args(["run", "-e", TWO_QUERIES, "--format", "json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        value,
        serde_json::json!([
            {
                "statement": 2,
                "columns": [{"name": "orderId", "type": "string"}],
                "rows": [{"orderId": "o-0001"}, {"orderId": "o-0002"}],
            },
            {
                "statement": 4,
                "columns": [{"name": "n", "type": "long"}],
                "rows": [{"n": 50}],
            },
        ])
    );
}

#[test]
fn run_arrow_writes_an_ipc_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.arrow");
    ds().args(["run", "-e", TWO_ROWS, "--format", "arrow", "--output"])
        .arg(&path)
        .assert()
        .success()
        .stdout("");
    let file = std::fs::File::open(&path).unwrap();
    let reader = arrow_ipc::reader::FileReader::try_new(file, None).unwrap();
    assert_eq!(reader.schema().field(0).name(), "orderId");
    let rows: usize = reader.map(|b| b.unwrap().num_rows()).sum();
    assert_eq!(rows, 2);
}

#[test]
fn run_arrow_to_a_pipe() {
    let output = ds()
        .args(["run", "-e", TWO_ROWS, "--format", "arrow"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stdout.starts_with(b"ARROW1"));
}

#[test]
fn run_output_writes_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.csv");
    ds().args(["run", "-e", TWO_ROWS, "--format", "csv", "-o"])
        .arg(&path)
        .assert()
        .success()
        .stdout("");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "orderId,amount\no-0001,183.5\no-0002,88.5\n"
    );
}

#[test]
fn run_output_writes_table_json_and_ndjson() {
    let dir = tempfile::tempdir().unwrap();

    let table_path = dir.path().join("out.txt");
    ds().args(["run", "-e", TWO_ROWS, "--format", "table", "--output"])
        .arg(&table_path)
        .assert()
        .success()
        .stdout("");
    assert_eq!(
        std::fs::read_to_string(&table_path).unwrap(),
        "orderId | amount\n\
         string  | real\n\
         --------+-------\n\
         o-0001  |  183.5\n\
         o-0002  |   88.5\n"
    );

    let json_path = dir.path().join("out.json");
    ds().args(["run", "-e", TWO_ROWS, "--format", "json", "--output"])
        .arg(&json_path)
        .assert()
        .success()
        .stdout("");
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&json_path).unwrap()).unwrap();
    assert_eq!(
        json,
        serde_json::json!([
            {
                "statement": 1,
                "columns": [
                    {"name": "orderId", "type": "string"},
                    {"name": "amount", "type": "real"},
                ],
                "rows": [
                    {"orderId": "o-0001", "amount": 183.5},
                    {"orderId": "o-0002", "amount": 88.5},
                ],
            }
        ])
    );

    let ndjson_path = dir.path().join("out.ndjson");
    ds().args(["run", "-e", TWO_ROWS, "--format", "ndjson", "--output"])
        .arg(&ndjson_path)
        .assert()
        .success()
        .stdout("");
    assert_eq!(
        std::fs::read_to_string(&ndjson_path).unwrap(),
        "{\"orderId\":\"o-0001\",\"amount\":183.5}\n{\"orderId\":\"o-0002\",\"amount\":88.5}\n"
    );
}

#[test]
fn run_output_to_a_missing_directory_is_a_usage_error() {
    ds().args(["run", "-e", TWO_ROWS, "-o", "no/such/dir/out.csv"])
        .assert()
        .code(2)
        .stderr(contains("cannot write to `no/such/dir/out.csv`"));
}

// ---- run: several queries ----

#[test]
fn run_table_prints_each_query_with_its_line() {
    ds().args(["run", "-e", TWO_QUERIES])
        .assert()
        .success()
        .stdout(
            "== query 1 (line 2) ==\n\
         orderId\n\
         string\n\
         -------\n\
         o-0001\n\
         o-0002\n\
         \n\
         == query 2 (line 4) ==\n\
         n\n\
         long\n\
         ----\n\
         \x20 50\n",
        );
}

#[test]
fn single_result_formats_need_one_query() {
    let dir = tempfile::tempdir().unwrap();
    for format in ["csv", "ndjson", "arrow"] {
        let out = dir.path().join(format!("out.{format}"));
        ds().args(["run", "-e", TWO_QUERIES, "--format", format, "-o"])
            .arg(&out)
            .assert()
            .code(2)
            .stdout("")
            .stderr(
                contains(format!(
                    "`--format {format}` prints one result, but the program has 2 query statements"
                ))
                .and(contains("--statement <n>`: 1 (line 2), 2 (line 4)")),
            );
        assert!(!out.exists(), "{format} should not create an output file");
    }
}

#[test]
fn statement_picks_one_query() {
    ds().args([
        "run",
        "-e",
        TWO_QUERIES,
        "--format",
        "csv",
        "--statement",
        "2",
    ])
    .assert()
    .success()
    .stdout("n\n50\n");
    // A table with one result has no header.
    ds().args(["run", "-e", TWO_QUERIES, "--statement", "1"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("orderId\n"));
}

#[test]
fn statement_out_of_range() {
    ds().args(["run", "-e", TWO_QUERIES, "--statement", "3"])
        .assert()
        .code(2)
        .stderr(contains(
            "`--statement 3` is out of range: the program has 2 query statements",
        ));
    ds().args(["run", "-e", TWO_QUERIES, "--statement", "0"])
        .assert()
        .code(2)
        .stderr(contains("invalid value '0'"));
}

#[test]
fn statement_on_a_program_with_no_query() {
    ds().args(["run", "-e", "let x = 1;", "--statement", "1"])
        .assert()
        .code(2)
        .stdout("")
        .stderr(
            contains("`--statement 1` is out of range: the program has 0 query statements")
                .and(contains("the program has no query to pick")),
        );
}

// ---- run: row limits ----

#[test]
fn max_rows_cuts_tables_with_a_footer() {
    ds().args([
        "run",
        "-e",
        "from 'orders.csv' | project orderId",
        "--max-rows",
        "2",
    ])
    .assert()
    .success()
    .stdout(
        "orderId\n\
             string\n\
             -------\n\
             o-0001\n\
             o-0002\n\
             … 48 more rows (show them with --max-rows 50)\n",
    );
}

#[test]
fn max_rows_defaults_to_100_and_0_shows_everything() {
    let program = "from 'orders.csv' | join kind = inner (from 'orders.csv' | project customerId) on customerId | project orderId";
    let default = ds().args(["run", "-e", program]).output().unwrap();
    let text = String::from_utf8(default.stdout).unwrap();
    assert_eq!(text.lines().count(), 3 + 100 + 1, "{text}");
    let footer = text.lines().last().unwrap();
    assert!(
        footer.starts_with("… ") && footer.contains("more rows"),
        "{text}"
    );

    let all = ds()
        .args(["run", "-e", program, "--max-rows", "0"])
        .output()
        .unwrap();
    let text = String::from_utf8(all.stdout).unwrap();
    assert!(!text.contains("more rows"), "{text}");
    assert!(text.lines().count() > 103);
}

#[test]
fn max_rows_does_not_cut_csv() {
    let output = ds()
        .args([
            "run",
            "-e",
            "from 'orders.csv'",
            "--format",
            "csv",
            "--max-rows",
            "2",
        ])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().lines().count(),
        51
    );
}

// ---- run: streaming (M2-11) ----

/// A CSV of `rows` rows with one column `x`: 1 everywhere except the last row, which is 10000.
/// `x * 1000000000000000` overflows `long` (DS4003) on that row only, so a query that fails
/// shows how far the engine read. The engine reads 8192 rows per batch.
fn big_csv(dir: &Path, rows: usize) -> String {
    let mut text = String::from("x\n");
    text.push_str(&"1\n".repeat(rows - 1));
    text.push_str("10000\n");
    std::fs::write(dir.join("big.csv"), text).unwrap();
    "from 'big.csv' | extend y = x * 1000000000000000".to_string()
}

/// Like [`big_csv`], without the overflowing last row, for multi-batch success tests.
fn valid_big_csv(dir: &Path, rows: usize) -> String {
    let mut text = String::from("x\n");
    text.push_str(&"1\n".repeat(rows));
    std::fs::write(dir.join("big.csv"), text).unwrap();
    "from 'big.csv' | extend y = x * 1000000000000000".to_string()
}

#[test]
fn closed_stdout_pipe_exits_0_and_stops_reading() {
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;

    // 60,000 rows of `1,1000000000000000\n` (~1.14 MB) exceeds any OS pipe buffer, so `ds`
    // hits BrokenPipe long before reaching the overflowing last row.
    let dir = tempfile::tempdir().unwrap();
    let program = big_csv(dir.path(), 60_000);
    let mut child = ds_process()
        .current_dir(dir.path())
        .args(["run", "-e", &program, "--format", "csv"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut header = String::new();
    stdout.read_line(&mut header).unwrap();
    assert_eq!(header, "x,y\n");
    // Close the read end, as `| head -1` does.
    drop(stdout);
    let output = child.wait_with_output().unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(output.status.code(), Some(0), "{stderr}");
    assert!(stderr.is_empty(), "{stderr}");
}

#[test]
fn ndjson_streams_batch_by_batch_and_stops_on_mid_stream_failure() {
    let dir = tempfile::tempdir().unwrap();
    let ok_program = valid_big_csv(dir.path(), 10_000);
    let ok = ds()
        .current_dir(dir.path())
        .args(["run", "-e", &ok_program, "--format", "ndjson"])
        .output()
        .unwrap();
    assert!(
        ok.status.success(),
        "{}",
        String::from_utf8_lossy(&ok.stderr)
    );
    let stdout = String::from_utf8(ok.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 10_000);
    assert_eq!(
        stdout.lines().next().unwrap(),
        r#"{"x":1,"y":1000000000000000}"#
    );

    let program = big_csv(dir.path(), 20_000);
    let output = ds()
        .current_dir(dir.path())
        .args(["run", "-e", &program, "--format", "ndjson"])
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("DS4003"), "{stderr}");
    assert!(
        stdout.starts_with("{\"x\":1,\"y\":1000000000000000}\n"),
        "{stdout:.100}"
    );
    let lines = stdout.lines().count();
    assert!((8_192..20_000).contains(&lines), "{lines} lines");
    let last: serde_json::Value = serde_json::from_str(stdout.lines().last().unwrap()).unwrap();
    assert_eq!(
        last,
        serde_json::json!({"x": 1, "y": 1_000_000_000_000_000_i64})
    );
}

#[test]
fn json_streams_batch_by_batch_and_leaves_array_unclosed_on_failure() {
    let dir = tempfile::tempdir().unwrap();
    let ok_program = valid_big_csv(dir.path(), 10_000);
    let ok = ds()
        .current_dir(dir.path())
        .args(["run", "-e", &ok_program, "--format", "json"])
        .output()
        .unwrap();
    assert!(
        ok.status.success(),
        "{}",
        String::from_utf8_lossy(&ok.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&ok.stdout).unwrap();
    assert_eq!(value[0]["rows"].as_array().unwrap().len(), 10_000);

    // Mid-stream failure across batches in a single query: earlier batches are written and the
    // JSON array is left unclosed so consumers do not mistake a failed run for valid JSON.
    let program = big_csv(dir.path(), 20_000);
    let output = ds()
        .current_dir(dir.path())
        .args(["run", "-e", &program, "--format", "json"])
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("DS4003"), "{stderr}");
    assert!(
        stdout.starts_with(
            r#"[{"statement":1,"columns":[{"name":"x","type":"long"},{"name":"y","type":"long"}],"rows":[{"x":1,"y":1000000000000000}"#
        ),
        "{stdout:.160}"
    );
    let rows = stdout.matches(r#"{"x":1,"y":1000000000000000}"#).count();
    assert!((8_192..20_000).contains(&rows), "{rows} rows");
    assert!(!stdout.ends_with("]\n"));
    assert!(serde_json::from_str::<serde_json::Value>(&stdout).is_err());

    // Failure on a later query: the earlier query's object stays written, and the outer array is
    // still left unclosed (no trailing `]\n`).
    let failing = big_csv(dir.path(), 10);
    let two_queries = format!("from 'big.csv' | count;\n{failing}");
    let output = ds()
        .current_dir(dir.path())
        .args(["run", "-e", &two_queries, "--format", "json"])
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("DS4003"), "{stderr}");
    assert_eq!(
        stdout,
        r#"[{"statement":1,"columns":[{"name":"Count","type":"long"}],"rows":[{"Count":10}]}"#
    );
    assert!(serde_json::from_str::<serde_json::Value>(&stdout).is_err());
}

#[test]
fn arrow_streams_batch_by_batch_and_stops_on_mid_stream_failure() {
    let dir = tempfile::tempdir().unwrap();
    let ok_program = valid_big_csv(dir.path(), 10_000);
    let out_path = dir.path().join("multi.arrow");
    ds().current_dir(dir.path())
        .args(["run", "-e", &ok_program, "--format", "arrow", "--output"])
        .arg(&out_path)
        .assert()
        .success()
        .stdout("");
    let file = std::fs::File::open(&out_path).unwrap();
    let reader = arrow_ipc::reader::FileReader::try_new(file, None).unwrap();
    let batches: Vec<_> = reader.map(|b| b.unwrap()).collect();
    assert!(batches.len() >= 2, "{} batches", batches.len());
    let rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    assert_eq!(rows, 10_000);

    // Mid-stream failure: earlier batches are flushed to the Arrow IPC stream, the footer is not
    // finished, and the process exits 1 with DS4003.
    let program = big_csv(dir.path(), 20_000);
    let output = ds()
        .current_dir(dir.path())
        .args(["run", "-e", &program, "--format", "arrow"])
        .output()
        .unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("DS4003"), "{stderr}");
    assert!(output.stdout.starts_with(b"ARROW1"));
    // At least one 8192-row batch of two i64 columns (> 128 KiB) was written before the failure.
    assert!(
        output.stdout.len() > 100_000,
        "only {} bytes written",
        output.stdout.len()
    );
    assert!(
        arrow_ipc::reader::FileReader::try_new(std::io::Cursor::new(&output.stdout), None).is_err(),
        "an unfinished Arrow IPC file should not have a valid footer"
    );
}

#[test]
fn csv_is_written_batch_by_batch_before_the_query_finishes() {
    let dir = tempfile::tempdir().unwrap();
    let program = big_csv(dir.path(), 20_000);
    let output = ds()
        .current_dir(dir.path())
        .args(["run", "-e", &program, "--format", "csv"])
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    // The last batch fails, but the batches before it were already written.
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("DS4003"), "{stderr}");
    assert!(
        stdout.starts_with("x,y\n1,1000000000000000\n"),
        "{stdout:.100}"
    );
    let lines = stdout.lines().count();
    assert!((8_193..20_000).contains(&lines), "{lines} lines");
}

#[test]
fn a_cut_table_stops_reading_and_says_at_least() {
    let dir = tempfile::tempdir().unwrap();
    let program = big_csv(dir.path(), 30_000);
    // Read in full, the query fails on its last row.
    ds().current_dir(dir.path())
        .args(["run", "-e", &program, "--max-rows", "0"])
        .assert()
        .code(1)
        .stderr(contains("DS4003"));
    // Cut at 3 rows, it stops reading before that row: counting gives up after a few thousand
    // rows, and dropping the stream cancels the rest.
    let output = ds()
        .current_dir(dir.path())
        .args(["run", "-e", &program, "--max-rows", "3"])
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.starts_with("x    | y\n"), "{stdout}");
    assert_eq!(stdout.lines().count(), 3 + 3 + 1, "{stdout}");
    let footer = stdout.lines().last().unwrap();
    assert!(
        footer.starts_with("… at least ")
            && footer.ends_with(" more rows (show them all with --max-rows 0)"),
        "{footer}"
    );
}

#[test]
fn a_runtime_error_keeps_the_results_already_printed() {
    let dir = tempfile::tempdir().unwrap();
    let failing = big_csv(dir.path(), 10);
    let program = format!("from 'big.csv' | count;\n{failing}");
    let output = ds()
        .current_dir(dir.path())
        .args(["run", "-e", &program])
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("DS4003"), "{stderr}");
    // The second query fails on its first batch, so nothing of it is printed.
    assert_eq!(
        stdout,
        "== query 1 (line 1) ==\n\
         Count\n\
         long\n\
         -----\n\
         \x20  10\n"
    );
}

#[test]
fn a_failed_run_creates_no_output_file() {
    let dir = tempfile::tempdir().unwrap();
    let program = big_csv(dir.path(), 10);
    let out = dir.path().join("out.csv");
    ds().current_dir(dir.path())
        .args(["run", "-e", &program, "--format", "csv", "--output"])
        .arg(&out)
        .assert()
        .code(1);
    assert!(!out.exists());
}

// ---- run: errors ----

#[test]
fn program_errors_exit_1() {
    ds().args(["run", "-e", "from 'orders.csv' | project nope"])
        .assert()
        .code(1)
        .stdout("")
        .stderr(contains("error[DS1001]: unknown column `nope`"));
}

#[test]
fn a_missing_script_is_a_usage_error_with_did_you_mean() {
    ds().args(["run", "../../examples/top_customer.ds"])
        .assert()
        .code(2)
        .stderr(
            contains("error[DS3001]: file `../../examples/top_customer.ds` not found").and(
                contains("a file with a similar name exists: `../../examples/top_customers.ds`"),
            ),
        );
}

#[test]
fn a_directory_is_not_a_script() {
    ds().args(["run", "."])
        .assert()
        .code(2)
        .stderr(contains("error[DS3008]: cannot read `.`").and(contains("this is a directory")));
}

#[test]
fn an_empty_program_warns_and_succeeds() {
    ds().args(["run", "-e", ""])
        .assert()
        .success()
        .stdout("")
        .stderr(contains("warning[DS0004]: program has no query to run"));
    ds().args(["run", "-"])
        .write_stdin("// nothing here\n")
        .assert()
        .success()
        .stderr(contains("--> <stdin>"));
}

#[test]
fn file_and_expr_together_are_a_usage_error() {
    ds().args(["run", "x.ds", "-e", "from 'orders.csv'"])
        .assert()
        .code(2);
    ds().args(["run"]).assert().code(2);
}

// ---- --table ----

#[test]
fn table_registers_a_host_table() {
    ds().args([
        "run",
        "-e",
        "from o | take 1 | project orderId",
        "--table",
        "o=orders.csv",
    ])
    .assert()
    .success()
    .stdout("orderId\nstring\n-------\no-0001\n");
}

#[test]
fn table_paths_resolve_against_the_current_directory() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("q.ds");
    std::fs::write(&script, "from o | take 1 | project orderId").unwrap();
    ds().arg("run")
        .arg(&script)
        .args(["--table", "o=orders.csv", "--format", "csv"])
        .assert()
        .success()
        .stdout("orderId\no-0001\n");
}

#[test]
fn table_with_an_unknown_format() {
    ds().args(["run", "-e", "from o", "--table", "o=orders.txt"])
        .assert()
        .code(2)
        .stderr(
            contains("error[DS3002]: cannot tell the format of `orders.txt`")
                .and(contains("such as `orders.csv`")),
        );
}

#[test]
fn table_without_equals_is_a_usage_error() {
    ds().args(["run", "-e", "from o", "--table", "orders.csv"])
        .assert()
        .code(2)
        .stderr(contains("expected NAME=PATH"));
}

#[test]
fn table_works_for_check_and_explain() {
    ds().args(["check", "-", "--table", "o=orders.csv"])
        .write_stdin("from o | take 1")
        .assert()
        .success();
    ds().args([
        "explain",
        "-e",
        "from o | take 1",
        "--table",
        "o=orders.csv",
    ])
    .assert()
    .success()
    .stdout(contains("Limit: 1"));
}

#[test]
fn several_table_flags_and_a_glob_table() {
    ds().args([
        "run",
        "-e",
        "from o | join c on customerId | sort by orderId asc | take 2 | project orderId, name",
        "--table",
        "o=orders.csv",
        "--table",
        "c=customers.csv",
        "--format",
        "csv",
    ])
    .assert()
    .success()
    .stdout(contains("orderId,name\no-0001,").and(contains("\no-0002,")));

    // A `--table` whose path is a glob reads every matching file, resolved against the current
    // directory even when the script sits elsewhere.
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("q.ds");
    std::fs::write(&script, "from ev | count").unwrap();
    ds().arg("run")
        .arg(&script)
        .args(["--table", "ev=events/*.ndjson", "--format", "csv"])
        .assert()
        .success()
        .stdout("Count\n24\n");
}

// ---- check ----

#[test]
fn check_a_clean_script_prints_nothing() {
    ds().args(["check", "../../examples/orders.ds"])
        .assert()
        .success()
        .stdout("")
        .stderr("");
}

#[test]
fn check_several_files() {
    let dir = tempfile::tempdir().unwrap();
    let good = dir.path().join("good.ds");
    let bad = dir.path().join("bad.ds");
    std::fs::write(&good, format!("from '{}' | take 1", orders_path())).unwrap();
    // `orders.csv` is not next to the script.
    std::fs::write(&bad, "from 'orders.csv'").unwrap();
    ds().arg("check")
        .arg(&good)
        .arg(&bad)
        .assert()
        .code(1)
        .stderr(contains("bad.ds:1:6").and(contains("good.ds").not()));
}

#[test]
fn check_a_missing_file_exits_2_but_still_checks_the_others() {
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("bad.ds");
    std::fs::write(&bad, "from 'orders.csv'").unwrap();
    ds().args(["check", "missing.ds"])
        .arg(&bad)
        .assert()
        .code(2)
        .stderr(contains("file `missing.ds` not found").and(contains("bad.ds:1:6")));
}

#[test]
fn check_json() {
    let output = ds()
        .args(["check", "-", "--format", "json"])
        .write_stdin("from 'orders.csv' | project nope")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let diagnostics = report["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0]["code"], "DS1001");
}

#[test]
fn check_json_of_a_clean_script() {
    let output = ds()
        .args(["check", "-", "--format", "json"])
        .write_stdin("from 'orders.csv' | take 1")
        .output()
        .unwrap();
    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["diagnostics"], serde_json::json!([]));
}

#[test]
fn check_an_empty_program_is_clean() {
    ds().args(["check", "-"])
        .write_stdin("")
        .assert()
        .success()
        .stderr("");
}

#[test]
fn check_deny_warnings_fails_on_warnings_but_not_info() {
    let warning_query = "from 'orders.csv' | sort by amount";
    ds().args(["check", "-"])
        .write_stdin(warning_query)
        .assert()
        .success()
        .stderr(contains("warning[DS2103]"));
    ds().args(["check", "-", "--deny-warnings"])
        .write_stdin(warning_query)
        .assert()
        .code(1)
        .stderr(contains("warning[DS2103]"));

    // Only an info diagnostic (`DS1007`, renamed join column): `--deny-warnings` still exits 0.
    let info_query =
        "from 'orders.csv' | join (from 'orders.csv' | project customerId, amount) on customerId";
    ds().args(["check", "-", "--deny-warnings"])
        .write_stdin(info_query)
        .assert()
        .success()
        .stderr(contains("info[DS1007]"));
}

#[test]
fn check_json_with_several_files_and_warnings() {
    let dir = tempfile::tempdir().unwrap();
    let orders = orders_path();
    let good = dir.path().join("good.ds");
    let warn = dir.path().join("warn.ds");
    let bad = dir.path().join("bad.ds");
    std::fs::write(&good, format!("from '{orders}' | take 1")).unwrap();
    std::fs::write(&warn, format!("from '{orders}' | sort by amount")).unwrap();
    std::fs::write(&bad, format!("from '{orders}' | project nope")).unwrap();

    // Warnings only: exit 0 without `--deny-warnings`, exit 1 with it.
    let output = ds()
        .args(["check", "--format", "json"])
        .arg(&good)
        .arg(&warn)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["version"], 1);
    let diagnostics = report["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0]["code"], "DS2103");
    assert_eq!(diagnostics[0]["severity"], "warning");
    assert!(
        diagnostics[0]["file"]
            .as_str()
            .unwrap()
            .ends_with("warn.ds")
    );

    let denied = ds()
        .args(["check", "--format", "json", "--deny-warnings"])
        .arg(&good)
        .arg(&warn)
        .output()
        .unwrap();
    assert_eq!(denied.status.code(), Some(1));
    assert!(denied.stderr.is_empty());
    let denied_report: serde_json::Value = serde_json::from_slice(&denied.stdout).unwrap();
    assert_eq!(denied_report["diagnostics"][0]["code"], "DS2103");

    // Several files with both warnings and errors.
    let output = ds()
        .args(["check", "--format", "json"])
        .arg(&good)
        .arg(&warn)
        .arg(&bad)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let diagnostics = report["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 2);
    assert_eq!(diagnostics[0]["code"], "DS2103");
    assert_eq!(diagnostics[0]["severity"], "warning");
    assert!(
        diagnostics[0]["file"]
            .as_str()
            .unwrap()
            .ends_with("warn.ds")
    );
    assert_eq!(diagnostics[1]["code"], "DS1001");
    assert_eq!(diagnostics[1]["severity"], "error");
    assert!(diagnostics[1]["file"].as_str().unwrap().ends_with("bad.ds"));
}

#[test]
fn check_json_with_a_missing_file_exits_2() {
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("bad.ds");
    std::fs::write(&bad, format!("from '{}' | project nope", orders_path())).unwrap();

    let output = ds()
        .args(["check", "missing.ds", "--format", "json"])
        .arg(&bad)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stderr.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["version"], 1);
    let diagnostics = report["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 2);
    assert_eq!(diagnostics[0]["code"], "DS3001");
    assert_eq!(diagnostics[0]["severity"], "error");
    assert!(diagnostics[0]["file"].is_null());
    assert!(diagnostics[0]["primary"].is_null());
    assert!(
        diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("missing.ds")
    );
    assert_eq!(diagnostics[1]["code"], "DS1001");
    assert!(diagnostics[1]["file"].as_str().unwrap().ends_with("bad.ds"));
}

// ---- explain ----

#[test]
fn explain_prints_the_plan() {
    ds().args([
        "explain",
        "-e",
        "from 'orders.csv' | take 1 | project orderId",
    ])
    .assert()
    .success()
    .stdout(contains("Project").and(contains("Limit: 1")));
}

#[test]
fn explain_a_script_file_and_stdin() {
    ds().current_dir(std::env::temp_dir())
        .arg("explain")
        .arg(repo_dir().join("examples/orders.ds"))
        .assert()
        .success()
        .stdout(
            contains("Scan:")
                .and(contains("Filter:"))
                .and(contains("Project:")),
        )
        .stderr("");

    ds().args(["explain", "-"])
        .write_stdin("from 'orders.csv' | where amount > 100 | take 5")
        .assert()
        .success()
        .stdout(
            contains("Limit: 5")
                .and(contains("Filter:"))
                .and(contains("Scan: 'orders.csv'")),
        )
        .stderr("");
}

#[test]
fn explain_several_queries_and_render() {
    let output = ds().args(["explain", "-e", TWO_QUERIES]).output().unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let sections: Vec<&str> = stdout.trim().split("\n\n").collect();
    assert_eq!(sections.len(), 2, "{stdout}");
    assert!(sections[0].contains("Limit: 2"), "{}", sections[0]);
    assert!(sections[1].contains("Aggregate:"), "{}", sections[1]);

    ds().args([
        "explain",
        "-e",
        "from 'orders.csv' | summarize revenue = sum(amount) by customerId | render barchart with (title = 'Rev')",
    ])
    .assert()
    .success()
    .stdout(
        contains("Aggregate:")
            .and(contains("render: barchart x=customerId y=[revenue] title=\"Rev\"\n")),
    )
    .stderr("");
}

#[test]
fn explain_errors_exit_1() {
    ds().args(["explain", "-e", "from 'nope.csv'"])
        .assert()
        .code(1)
        .stderr(contains("error[DS3001]"));
}

// ---- colors ----

const ESC: &str = "\u{1b}[";

#[test]
fn diagnostics_are_plain_ascii_when_not_on_a_terminal() {
    ds().args(["run", "-e", "from 'orders.csv' | project nope"])
        .assert()
        .stderr(contains("--> <expr>:1:29").and(contains(ESC).not()));
}

#[test]
fn color_always_uses_the_terminal_renderer() {
    ds().args([
        "--color",
        "always",
        "run",
        "-e",
        "from 'orders.csv' | project nope",
    ])
    .assert()
    .stderr(contains(ESC).and(contains("┌─")));
    // `--color` works after the subcommand too, and beats NO_COLOR.
    ds().args([
        "run",
        "-e",
        "from 'orders.csv' | project nope",
        "--color",
        "always",
    ])
    .env("NO_COLOR", "1")
    .assert()
    .stderr(contains(ESC));
}

#[test]
fn color_never_is_plain() {
    ds().args([
        "run",
        "-e",
        "from 'orders.csv' | project nope",
        "--color",
        "never",
    ])
    .assert()
    .stderr(contains(ESC).not().and(contains("-->")));
}

// ---- Ctrl-C ----

/// Rewinds a temporary file capturing child stderr and returns its contents.
fn read_temp_file(file: &mut std::fs::File) -> String {
    use std::io::{Read, Seek, SeekFrom};

    file.seek(SeekFrom::Start(0)).unwrap();
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(unix)]
#[test]
fn ctrl_c_stops_a_running_query() {
    use std::io::{BufRead, BufReader};
    use std::os::unix::process::ExitStatusExt;
    use std::process::Stdio;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    // Query 1 flushes a row immediately, proving `ds` has started and installed its SIGINT
    // handler; query 2 then runs a 14-way self-join that would outlast the test.
    let join = "join kind = inner (from 'orders.csv' | project customerId) on customerId";
    let program = format!(
        "from 'orders.csv' | take 1 | project orderId;\n\
         from 'orders.csv' | project customerId | {} | summarize n = count()",
        vec![join; 14].join(" | ")
    );
    let mut stderr_file = tempfile::tempfile().unwrap();
    let mut child = ds_process()
        .args(["run", "-e", &program])
        .stdout(Stdio::piped())
        .stderr(Stdio::from(stderr_file.try_clone().unwrap()))
        .spawn()
        .unwrap();
    let (tx, rx) = mpsc::channel();
    let stdout = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if tx.send(line).is_err() {
                        break;
                    }
                }
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(remaining) {
            Ok(line) if line == "o-0001\n" => break,
            Ok(_) => continue,
            Err(reason) => {
                let _ = child.kill();
                let status = child.wait().unwrap();
                let _ = reader.join();
                let stderr = read_temp_file(&mut stderr_file);
                panic!(
                    "did not see first row on stdout ({reason}); status={status:?}, stderr={stderr}"
                );
            }
        }
    }
    let sent = Instant::now();
    let kill = std::process::Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(kill.success());
    let status = child.wait().unwrap();
    drop(rx);
    reader.join().unwrap();
    let stderr = read_temp_file(&mut stderr_file);
    assert!(sent.elapsed() < Duration::from_secs(5));
    assert_eq!(
        status.code(),
        Some(130),
        "signal {:?}, stderr {stderr}",
        status.signal(),
    );
    assert!(stderr.contains("interrupted"), "{stderr}");
}

/// Writes one framed JSON-RPC message, as the LSP sends them over stdio.
fn lsp_send(stdin: &mut impl std::io::Write, body: &str) {
    write!(stdin, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
    stdin.flush().unwrap();
}

/// Reads one framed JSON-RPC message.
fn lsp_receive(stdout: &mut impl std::io::BufRead) -> String {
    let mut length = 0;
    loop {
        let mut line = String::new();
        assert_ne!(
            stdout.read_line(&mut line).unwrap(),
            0,
            "the server closed stdout"
        );
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(value) = line.strip_prefix("Content-Length: ") {
            length = value.parse().unwrap();
        }
    }
    let mut body = vec![0; length];
    stdout.read_exact(&mut body).unwrap();
    String::from_utf8(body).unwrap()
}

#[test]
fn lsp_speaks_the_protocol_on_stdio() {
    use std::io::BufReader;
    use std::process::Stdio;

    let mut child = ds_process()
        .args(["lsp", "--stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    lsp_send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"processId":null,"rootUri":null,"capabilities":{}}}"#,
    );
    let reply = lsp_receive(&mut stdout);
    assert!(reply.contains(r#""serverInfo":{"name":"ds""#), "{reply}");
    lsp_send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#,
    );
    lsp_send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"untitled:q","languageId":"datascript","version":1,"text":"from 'orders.csv' | where amount = 1"}}}"#,
    );
    let published = lsp_receive(&mut stdout);
    assert!(
        published.contains("textDocument/publishDiagnostics"),
        "{published}"
    );
    assert!(published.contains(r#""code":"DS0110""#), "{published}");
    lsp_send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":2,"method":"shutdown"}"#,
    );
    assert!(lsp_receive(&mut stdout).contains(r#""id":2"#));
    lsp_send(&mut stdin, r#"{"jsonrpc":"2.0","method":"exit"}"#);
    assert!(child.wait().unwrap().success());
}

// ---- M2-31: CLI plumbing and bug-fix regressions ----

#[test]
fn zero_row_results_are_valid_in_every_format() {
    let query = "from 'orders.csv' | take 0 | project orderId, amount";

    ds().args(["run", "-e", query, "--format", "table"])
        .assert()
        .success()
        .stdout(
            "orderId | amount\n\
             string  | real\n\
             --------+-------\n",
        )
        .stderr("");

    ds().args(["run", "-e", query, "--format", "csv"])
        .assert()
        .success()
        .stdout("orderId,amount\n")
        .stderr("");

    let json_out = ds()
        .args(["run", "-e", query, "--format", "json"])
        .output()
        .unwrap();
    assert!(json_out.status.success());
    assert!(json_out.stderr.is_empty());
    let json: serde_json::Value = serde_json::from_slice(&json_out.stdout).unwrap();
    assert_eq!(
        json,
        serde_json::json!([
            {
                "statement": 1,
                "columns": [
                    {"name": "orderId", "type": "string"},
                    {"name": "amount", "type": "real"},
                ],
                "rows": [],
            }
        ])
    );

    ds().args(["run", "-e", query, "--format", "ndjson"])
        .assert()
        .success()
        .stdout("")
        .stderr("");

    let arrow_out = ds()
        .args(["run", "-e", query, "--format", "arrow"])
        .output()
        .unwrap();
    assert!(arrow_out.status.success());
    assert!(arrow_out.stderr.is_empty());
    let reader =
        arrow_ipc::reader::FileReader::try_new(std::io::Cursor::new(&arrow_out.stdout), None)
            .unwrap();
    assert_eq!(reader.schema().fields().len(), 2);
    assert_eq!(reader.schema().field(0).name(), "orderId");
    assert_eq!(reader.schema().field(1).name(), "amount");
    let rows: usize = reader.map(|b| b.unwrap().num_rows()).sum();
    assert_eq!(rows, 0);
}

#[test]
fn let_only_program_writes_nothing_and_creates_no_output_file() {
    let dir = tempfile::tempdir().unwrap();
    for format in ["table", "csv", "json", "ndjson", "arrow"] {
        let out = dir.path().join(format!("let_only.{format}"));
        ds().args(["run", "-e", "let x = 1;", "--format", format, "--output"])
            .arg(&out)
            .assert()
            .success()
            .stdout("")
            .stderr(contains("warning[DS0004]"));
        assert!(!out.exists(), "{format} should not create an output file");
    }
}

#[test]
fn warnings_go_to_stderr_and_leave_stdout_parseable() {
    // `sort by orderId` without `asc`/`desc` emits `warning[DS2103]`.
    let query = "from 'orders.csv' | sort by orderId | take 2 | project orderId, amount";

    let csv_out = ds()
        .args(["run", "-e", query, "--format", "csv"])
        .output()
        .unwrap();
    assert!(csv_out.status.success());
    assert!(String::from_utf8_lossy(&csv_out.stderr).contains("warning[DS2103]"));
    assert_eq!(
        String::from_utf8(csv_out.stdout).unwrap(),
        "orderId,amount\no-0001,183.5\no-0002,88.5\n"
    );

    let json_out = ds()
        .args(["run", "-e", query, "--format", "json"])
        .output()
        .unwrap();
    assert!(json_out.status.success());
    assert!(String::from_utf8_lossy(&json_out.stderr).contains("warning[DS2103]"));
    let json: serde_json::Value = serde_json::from_slice(&json_out.stdout).unwrap();
    assert_eq!(json[0]["rows"].as_array().unwrap().len(), 2);

    let ndjson_out = ds()
        .args(["run", "-e", query, "--format", "ndjson"])
        .output()
        .unwrap();
    assert!(ndjson_out.status.success());
    assert!(String::from_utf8_lossy(&ndjson_out.stderr).contains("warning[DS2103]"));
    let ndjson_text = String::from_utf8(ndjson_out.stdout).unwrap();
    let lines: Vec<&str> = ndjson_text.lines().collect();
    assert_eq!(lines.len(), 2);
    for line in lines {
        let row: serde_json::Value = serde_json::from_str(line).unwrap();
        assert!(row.is_object());
    }

    let arrow_out = ds()
        .args(["run", "-e", query, "--format", "arrow"])
        .output()
        .unwrap();
    assert!(arrow_out.status.success());
    assert!(String::from_utf8_lossy(&arrow_out.stderr).contains("warning[DS2103]"));
    let reader =
        arrow_ipc::reader::FileReader::try_new(std::io::Cursor::new(&arrow_out.stdout), None)
            .unwrap();
    let rows: usize = reader.map(|b| b.unwrap().num_rows()).sum();
    assert_eq!(rows, 2);
}

#[test]
fn statement_with_json_keeps_line_number_and_skips_failing_sibling() {
    // Statement 2 in `TWO_QUERIES` starts on line 4.
    let output = ds()
        .args([
            "run",
            "-e",
            TWO_QUERIES,
            "--statement",
            "2",
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        json,
        serde_json::json!([
            {
                "statement": 4,
                "columns": [{"name": "n", "type": "long"}],
                "rows": [{"n": 50}],
            }
        ])
    );

    // Selecting statement 2 succeeds even when statement 1 fails at runtime.
    let mixed = "from 'orders.csv' | take 1 | extend z = 1 / (tolong(amount) - tolong(amount));\n\
                 from 'orders.csv' | take 1 | project orderId";
    ds().args(["run", "-e", mixed, "--statement", "2", "--format", "csv"])
        .assert()
        .success()
        .stdout("orderId\no-0001\n")
        .stderr("");
    ds().args(["run", "-e", mixed, "--statement", "1"])
        .assert()
        .code(1)
        .stderr(contains("DS4003"));
}

#[test]
fn runtime_failure_during_count_rest_leaves_partial_table_without_footer() {
    // Batch 1 has 8,192 valid rows; with `--max-rows 2`, 8,190 extra rows remain (< COUNT_BUDGET
    // of 10,000), so `count_rest` reads batch 2 and hits the overflowing row 9,000.
    let dir = tempfile::tempdir().unwrap();
    let program = big_csv(dir.path(), 9_000);
    let output = ds()
        .current_dir(dir.path())
        .args(["run", "-e", &program, "--max-rows", "2"])
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("DS4003"), "{stderr}");
    assert_eq!(
        stdout,
        "x    | y\n\
         long | long\n\
         -----+-----------------\n\
         \x20  1 | 1000000000000000\n\
         \x20  1 | 1000000000000000\n"
    );
    assert!(!stdout.contains("more row"), "{stdout}");
}

#[test]
fn mid_stream_failure_while_writing_to_output_file() {
    let dir = tempfile::tempdir().unwrap();
    let program = big_csv(dir.path(), 20_000);
    let out = dir.path().join("partial.csv");
    ds().current_dir(dir.path())
        .args(["run", "-e", &program, "--format", "csv", "--output"])
        .arg(&out)
        .assert()
        .code(1)
        .stdout("")
        .stderr(contains("DS4003"));
    let written = std::fs::read_to_string(&out).unwrap();
    assert!(
        written.starts_with("x,y\n1,1000000000000000\n"),
        "{written:.100}"
    );
    let lines = written.lines().count();
    assert!((8_193..20_000).contains(&lines), "{lines} lines");
}

/// Spawns `cmd`, reads `prefix_bytes` from stdout, closes the pipe, and asserts exit 0 with
/// clean stderr.
fn assert_broken_pipe_exits_0(cmd: &mut std::process::Command, prefix_bytes: usize) {
    use std::io::Read;
    use std::process::Stdio;

    let mut stderr_file = tempfile::tempfile().unwrap();
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::from(stderr_file.try_clone().unwrap()))
        .spawn()
        .unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let mut buf = vec![0u8; prefix_bytes];
    if let Err(err) = stdout.read_exact(&mut buf) {
        drop(stdout);
        let status = child.wait().unwrap();
        let stderr = read_temp_file(&mut stderr_file);
        panic!(
            "failed to read {prefix_bytes} byte(s) from stdout ({err}); \
             status={status:?}, stderr={stderr}"
        );
    }
    drop(stdout);
    let status = child.wait().unwrap();
    let stderr = read_temp_file(&mut stderr_file);
    assert_eq!(status.code(), Some(0), "{stderr}");
    assert!(stderr.is_empty(), "{stderr}");
}

#[test]
fn broken_pipe_exits_0_for_all_formats_check_json_and_explain() {
    let dir = tempfile::tempdir().unwrap();

    // `table` buffers up to `--max-rows` before writing; use valid rows so it reaches the write.
    let valid = valid_big_csv(dir.path(), 60_000);
    assert_broken_pipe_exits_0(
        ds_process().current_dir(dir.path()).args([
            "run",
            "-e",
            &valid,
            "--format",
            "table",
            "--max-rows",
            "0",
        ]),
        1,
    );

    // `json`, `ndjson` and `arrow` stream batch by batch; 100,000 rows exceed any OS pipe buffer
    // (~1.6 MB even for binary Arrow IPC), so `ds` hits BrokenPipe before the overflowing row.
    let failing = big_csv(dir.path(), 100_000);
    for (format, prefix) in [("json", 1), ("ndjson", 1), ("arrow", 6)] {
        assert_broken_pipe_exits_0(
            ds_process()
                .current_dir(dir.path())
                .args(["run", "-e", &failing, "--format", format]),
            prefix,
        );
    }

    // `ds check --format json` with a >1 MiB diagnostic report (30 copies of a 50-error script).
    let bad_script = dir.path().join("many_errors.ds");
    std::fs::write(&bad_script, "from 'a.csv' | where x = 1;\n".repeat(50)).unwrap();
    let mut check_cmd = ds_process();
    check_cmd.args(["check", "--format", "json"]);
    for _ in 0..30 {
        check_cmd.arg(&bad_script);
    }
    assert_broken_pipe_exits_0(&mut check_cmd, 1);

    // `ds explain` with a >1 MiB logical plan dump.
    let explain_script = dir.path().join("many_queries.ds");
    let one_query = format!(
        "from '{}' | where amount > 0 | extend a = amount + 1, b = amount + 2, \
         c = amount + 3, d = amount + 4 | project orderId, customerId, amount, a, b, c, d;\n",
        orders_path()
    );
    std::fs::write(&explain_script, one_query.repeat(3_000)).unwrap();
    assert_broken_pipe_exits_0(ds_process().arg("explain").arg(&explain_script), 1);
}

#[test]
fn check_json_with_bad_table_emits_json_ds3002_and_exits_2() {
    let output = ds()
        .args(["check", "-", "--format", "json", "--table", "o=orders.txt"])
        .write_stdin("from o")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["version"], 1);
    let diagnostics = report["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0]["code"], "DS3002");
    assert_eq!(diagnostics[0]["severity"], "error");
    assert!(diagnostics[0]["file"].is_null());
    assert!(diagnostics[0]["primary"].is_null());
    assert!(
        diagnostics[0]["message"]
            .as_str()
            .unwrap()
            .contains("orders.txt")
    );
}

#[test]
fn duplicate_table_flags_last_wins_in_run_check_and_explain() {
    let tables = ["--table", "t=orders.csv", "--table", "t=customers.csv"];

    ds().args([
        "run",
        "-e",
        "from t | take 1 | project name",
        "--format",
        "csv",
    ])
    .args(tables)
    .assert()
    .success()
    .stdout("name\nAna Silva\n")
    .stderr("");

    ds().args(["check", "-"])
        .args(tables)
        .write_stdin("from t | project name")
        .assert()
        .success()
        .stderr("");

    ds().args(["explain", "-e", "from t | project name"])
        .args(tables)
        .assert()
        .success()
        .stdout(
            contains("Scan: table `t`")
                .and(contains("name: string"))
                .and(contains("orderId").not()),
        )
        .stderr("");
}

#[test]
fn missing_table_file_unused_exits_0_and_referenced_exits_1_with_ds3008() {
    let missing = ["--table", "ghost=does_not_exist.csv"];

    ds().args([
        "run",
        "-e",
        "from 'orders.csv' | take 1 | project orderId",
        "--format",
        "csv",
    ])
    .args(missing)
    .assert()
    .success()
    .stdout("orderId\no-0001\n")
    .stderr("");

    ds().args(["check", "-"])
        .args(missing)
        .write_stdin("from 'orders.csv' | take 1")
        .assert()
        .success()
        .stderr("");

    // A missing host table file surfaces as DS3008 ("the file registered for this table does not
    // exist") rather than DS3001, because `SourceError::NotFound` on `SourceRef::Table` means an
    // unregistered table name (DS1002).
    ds().args(["run", "-e", "from ghost"])
        .args(missing)
        .assert()
        .code(1)
        .stdout("")
        .stderr(contains("error[DS3008]").and(contains(
            "the file registered for this table does not exist",
        )));
}

#[test]
fn non_utf8_script_file_gives_ds3008_with_exit_2() {
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("bad.ds");
    std::fs::write(&bad, [0xff, 0xfe, 0x00]).unwrap();

    ds().arg("run")
        .arg(&bad)
        .assert()
        .code(2)
        .stdout("")
        .stderr(contains("error[DS3008]").and(contains("not UTF-8 text")));

    ds().arg("check")
        .arg(&bad)
        .assert()
        .code(2)
        .stdout("")
        .stderr(contains("error[DS3008]").and(contains("not UTF-8 text")));
}
