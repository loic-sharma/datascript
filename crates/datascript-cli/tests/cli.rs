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
fn ds() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ds"));
    cmd.current_dir(data_dir()).env_remove("NO_COLOR");
    cmd
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
    Command::new(env!("CARGO_BIN_EXE_ds"))
        .current_dir(std::env::temp_dir())
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
    for format in ["csv", "ndjson", "arrow"] {
        ds().args([
            "run",
            "-e",
            TWO_QUERIES,
            "--format",
            format,
            "-o",
            "/dev/null",
        ])
        .assert()
        .code(2)
        .stdout("")
        .stderr(
            contains(format!(
                "`--format {format}` prints one result, but the program has 2 query statements"
            ))
            .and(contains("--statement <n>`: 1 (line 2), 2 (line 4)")),
        );
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

#[cfg(unix)]
#[test]
fn ctrl_c_stops_a_running_query() {
    use std::time::{Duration, Instant};

    // Each self-join multiplies the rows, so this runs far longer than the test waits.
    let join = "join kind = inner (from 'orders.csv' | project customerId) on customerId";
    let program = format!(
        "from 'orders.csv' | project customerId | {} | summarize n = count()",
        vec![join; 14].join(" | ")
    );
    // The first launch of a freshly linked binary can take seconds on macOS, and a SIGINT that
    // arrives before `main` installs its handler kills the process outright. Launch it once.
    ds().arg("--version").assert().success();
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_ds"))
        .current_dir(data_dir())
        .args(["run", "-e", &program])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_secs(1));
    assert!(
        child.try_wait().unwrap().is_none(),
        "the query finished too soon"
    );
    let sent = Instant::now();
    let kill = std::process::Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(kill.success());
    let output = child.wait_with_output().unwrap();
    assert!(sent.elapsed() < Duration::from_secs(5));
    use std::os::unix::process::ExitStatusExt;
    assert_eq!(
        output.status.code(),
        Some(130),
        "signal {:?}, stderr {}",
        output.status.signal(),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("interrupted"));
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

    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_ds"))
        .args(["lsp", "--stdio"])
        .current_dir(data_dir())
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
