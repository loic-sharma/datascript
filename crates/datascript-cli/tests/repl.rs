//! The REPL end to end, in piped mode: standard input is not a terminal, so `ds` reads inputs
//! from it with the same submit rule and exits 1 if any input had an error (M1-12). The line
//! editor itself (Ctrl-C, Ctrl-D, highlighting) needs a terminal and is checked by hand.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;
use predicates::str::contains;

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/data")
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

/// `ds` (no command) running in `tests/data`, reading `input` from a pipe.
fn repl(input: &str) -> assert_cmd::assert::Assert {
    ds().write_stdin(input).assert()
}

#[test]
fn lets_are_kept_across_inputs() {
    repl(
        "let big = from 'orders.csv' | where amount > 150;\n\
         from big\n\
         | summarize n = count()\n\
         \n\
         let cutoff = 250;\n\
         from big | where amount > cutoff | count;\n",
    )
    .success()
    .stdout(contains("n\nlong\n----\n  10\n"))
    .stdout(contains("Count\nlong\n-----\n    1\n"))
    .stderr("");
}

#[test]
fn several_queries_in_one_input_get_headers_with_input_lines() {
    // A line ending in `;` submits, so the second query is on the first line or the input
    // runs on until a blank line.
    repl("let o = 'orders.csv';\nfrom o | take 1 | project orderId; from o\n| count\n\n")
        .success()
        .stdout(contains("== query 1 (line 1) ==\norderId\n"))
        .stdout(contains("== query 2 (line 1) ==\nCount\n"));
    // Each line ending in `;` is its own input, with one result and no header.
    repl("from 'orders.csv' | take 1 | project orderId;\nfrom 'orders.csv' | count;\n")
        .success()
        .stdout(contains("== query").not());
}

#[test]
fn an_error_points_at_the_input_it_is_in() {
    repl("let a = 1;\nfrom 'orders.csv' | where amout > a;\n")
        .code(1)
        .stderr(contains("error[DS1001]: unknown column `amout`"))
        .stderr(contains("--> <repl:2>:1:27"));
}

#[test]
fn a_failed_input_saves_nothing_but_later_inputs_still_run() {
    repl("let a = nope;\nlet b = 1;\nfrom 'orders.csv' | where amount > b | take 1 | project orderId;\nlet c = a;\n")
        .code(1)
        .stdout(contains("o-0001"))
        .stderr(contains("--> <repl:1>:1:9"))
        .stderr(contains("--> <repl:4>:1:9"));
}

#[test]
fn redefining_a_let_names_the_removed_users() {
    repl("let x = 1;\nlet y = x + 1;\nlet x = 2;\nlet z = y;\n")
        .code(1)
        .stderr(contains(
            "info: removed `y`, which used `x`; enter it again to use the new definition",
        ))
        .stderr(contains("--> <repl:4>:1:9"));
}

#[test]
fn two_lets_of_one_name_in_one_input_are_ds1003() {
    repl("let x = 1; let x = 2;\n")
        .code(1)
        .stderr(contains("error[DS1003]"));
}

#[test]
fn schema_prints_columns_and_types() {
    repl(".schema 'orders.csv'\n")
        .success()
        .stdout(contains("column     | type\n"))
        .stdout(contains("amount     | real\n"))
        .stdout(contains("timestamp  | datetime\n"));
}

#[test]
fn schema_of_a_tabular_let() {
    repl("let small = from 'orders.csv' | project orderId;\n.schema small\n")
        .success()
        .stdout(contains(
            "column  | type\n--------+-------\norderId | string\n",
        ));
}

#[test]
fn schema_errors_are_diagnostics_on_the_argument() {
    repl(".schema 'ordrs.csv'\n")
        .code(1)
        .stderr(contains("error[DS3001]: file `ordrs.csv` not found"))
        .stderr(contains("--> <repl>:1:9"))
        .stderr(contains("a file with a similar name exists: `orders.csv`"));
}

#[test]
fn schema_needs_a_source() {
    repl(".schema\n")
        .code(1)
        .stderr(contains("error: `.schema` needs a source\n  tip: "));
}

#[test]
fn tables_lists_host_tables_and_lets() {
    ds().args(["--table", "orders=orders.csv"])
        .write_stdin("let n = 5;\nlet t = from orders;\n.tables\n")
        .assert()
        .success()
        .stdout(
            "name   | kind        | path\n\
             -------+-------------+-----------\n\
             orders | host table  | orders.csv\n\
             n      | scalar let  |\n\
             t      | tabular let |\n",
        );
}

#[test]
fn tables_with_nothing_says_so() {
    repl(".tables\n")
        .success()
        .stdout(contains("no host tables or saved lets yet"));
}

#[test]
fn format_csv_prints_every_result_under_its_header() {
    repl(
        ".format csv\n\
         from 'orders.csv' | take 1\n\
         | project orderId; from 'orders.csv'\n\
         | take 1 | project amount;\n\
         .format\n",
    )
    .success()
    .stdout(
        "== query 1 (line 1) ==\norderId\no-0001\n\n\
         == query 2 (line 2) ==\namount\n183.5\nthe output format is `csv`\n",
    );
}

#[test]
fn format_json() {
    repl(".format json\nfrom 'orders.csv' | take 1 | project orderId;\n")
        .success()
        .stdout(contains(r#""rows":[{"orderId":"o-0001"}]"#));
}

#[test]
fn unknown_meta_command_gets_did_you_mean() {
    repl(".shema 'orders.csv'\n").code(1).stderr(
        "error: unknown command `.shema`\n  tip: a command with a similar name exists: `.schema`\n",
    );
}

#[test]
fn max_rows_cuts_tables_and_the_footer_names_the_command() {
    repl(
        ".max-rows 2\n\
         from 'orders.csv' | project orderId;\n\
         .max-rows 0\n\
         from 'orders.csv' | count;\n",
    )
    .success()
    .stdout(contains(
        "o-0002\n… 48 more rows (show them with .max-rows 50)\n",
    ))
    .stdout(contains("--max-rows").not());
}

#[test]
fn a_cut_table_of_a_large_result_stops_reading_and_names_the_command() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("big.csv"),
        format!("x\n{}", "1\n".repeat(30_000)),
    )
    .unwrap();
    ds().current_dir(dir.path())
        .write_stdin(".max-rows 1\nfrom 'big.csv';\n")
        .assert()
        .success()
        .stdout(
            contains("… at least ").and(contains(" more rows (show them all with .max-rows 0)\n")),
        );
}

#[test]
fn bad_format_argument() {
    repl(".format xml\n")
        .code(1)
        .stderr(contains("error: unknown output format `xml`"));
}

#[test]
fn quit_stops_reading() {
    repl(".quit\nfrom nope;\n").success().stderr("");
    repl(".exit\n").success();
}

#[test]
fn help_lists_commands_and_keys() {
    repl(".help\n")
        .success()
        .stdout(contains(".schema <source>"))
        .stdout(contains("Ctrl-D"));
}

#[test]
fn an_error_exits_1_but_keeps_earlier_lets() {
    repl("let n = 1;\nfrom 'missing.csv';\nlet m = n;\n")
        .code(1)
        .stderr(contains("DS3001"))
        .stderr(contains("<repl:3>").not());
}

#[test]
fn incomplete_input_at_a_blank_line_gets_the_parser_diagnostic() {
    repl("from 'orders.csv' | where amount > (1\n\nfrom 'orders.csv' | count;\n")
        .code(1)
        .stdout(contains("Count"))
        .stderr(contains("--> <repl:1>"));
}

#[test]
fn charts_give_one_ds5001_per_input() {
    repl(
        "from 'orders.csv' | project customerId, amount | render barchart;\n\
         from 'orders.csv' | project customerId, amount | render piechart;\n",
    )
    .success()
    .stderr(contains("info[DS5001]").count(2));
    repl("from 'orders.csv' | take 1 | render table;\n")
        .success()
        .stderr("");
}

#[test]
fn table_with_an_unknown_format_is_a_usage_error() {
    ds().args(["--table", "t=orders.txt"])
        .write_stdin("")
        .assert()
        .code(2)
        .stderr(contains("DS3002"));
}

#[test]
fn diagnostics_are_plain_even_with_color_always() {
    ds().args(["--color", "always"])
        .write_stdin("from nope;\n")
        .assert()
        .code(1)
        .stderr(contains("--> <repl:1>").and(contains("\x1b[").not()));
}

#[test]
fn startup_max_rows_flag_sets_initial_limit() {
    ds().args(["--max-rows", "2"])
        .write_stdin("from 'orders.csv' | project orderId;\n.max-rows\n")
        .assert()
        .success()
        .stdout(
            contains("o-0002\n… 48 more rows (show them with .max-rows 50)\n")
                .and(contains("tables show 2 rows\n")),
        )
        .stderr("");

    ds().args(["--max-rows", "0"])
        .write_stdin(".max-rows\n")
        .assert()
        .success()
        .stdout("tables show every row\n")
        .stderr("");
}

#[test]
fn runtime_error_points_at_its_input_and_keeps_session() {
    repl(
        "let a = 10;\n\
         let b = 20; from 'orders.csv' | take 1 | extend z = 1 / (tolong(amount) - tolong(amount));\n\
         from 'orders.csv' | take 1 | project orderId, total = a + b;\n",
    )
    .code(1)
    .stderr(
        contains("error[DS4003]")
            .and(contains("--> <repl:2>:"))
            .and(contains("<repl:3>").not()),
    )
    .stdout(
        "orderId | total\n\
         string  | long\n\
         --------+------\n\
         o-0001  |    30\n",
    );
}

#[test]
fn saved_let_whose_file_is_removed_points_at_repl_1() {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::process::Stdio;

    let dir = tempfile::tempdir().unwrap();
    let tmp = dir.path().join("tmp.csv");
    std::fs::write(&tmp, "x\n1\n").unwrap();
    // ~610 KiB of CSV output exceeds OS pipe buffers (4–64 KiB), so `ds` deterministically
    // blocks inside `<repl:3>` after `<repl:1>` and `<repl:2>` have succeeded and before
    // `<repl:4>` starts.
    let pad_row = format!("{}\n", "a".repeat(60));
    let pad_body = pad_row.repeat(10_000);
    std::fs::write(dir.path().join("pad.csv"), format!("s\n{pad_body}")).unwrap();

    let mut child = ds_process()
        .current_dir(dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let mut stdin = child.stdin.take().unwrap();
    stdin
        .write_all(
            b"let t = from 'tmp.csv';\n\
              .format csv\n\
              from t | take 1;\n\
              from 'pad.csv';\n\
              from t | take 1;\n",
        )
        .unwrap();
    drop(stdin);

    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    // `<repl:2>` proves `<repl:1>` succeeded, saved `t`, and read `tmp.csv` before removal.
    stdout.read_line(&mut line).unwrap();
    assert_eq!(line, "x\n");
    line.clear();
    stdout.read_line(&mut line).unwrap();
    assert_eq!(line, "1\n");
    // Reading `pad.csv`'s header proves `<repl:3>` is now blocked writing to stdout, so
    // `<repl:2>`'s handle is closed and `<repl:4>` has not started yet.
    line.clear();
    stdout.read_line(&mut line).unwrap();
    assert_eq!(line, "s\n");

    std::fs::remove_file(&tmp).unwrap();

    // Drain the rest of stdout so `<repl:3>` finishes and `<repl:4>` runs.
    let mut rest = String::new();
    stdout.read_to_string(&mut rest).unwrap();
    drop(stdout);
    // Only `pad.csv`'s 10,000 rows follow — nothing from `<repl:4>`.
    assert_eq!(rest.len(), pad_body.len());
    assert_eq!(rest, pad_body);

    let output = child.wait_with_output().unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    // Exactly one error: `<repl:4>` failed because `<repl:1>`'s file was removed.
    assert_eq!(stderr.matches("error[").count(), 1, "{stderr}");
    assert!(stderr.contains("error[DS3001]"), "{stderr}");
    assert!(stderr.contains("--> <repl:1>:1:14"), "{stderr}");
}

#[test]
fn max_rows_meta_command_query_and_bad_argument() {
    repl(".max-rows\n.max-rows 1\n.max-rows\n.max-rows 0\n.max-rows\n")
        .success()
        .stdout("tables show 100 rows\ntables show 1 row\ntables show every row\n")
        .stderr("");

    repl(".max-rows lots\n").code(1).stderr(
        "error: `.max-rows` needs a whole number, found `lots`\n  \
         tip: use `.max-rows 20`, or `.max-rows 0` to show every row\n",
    );

    repl(".max-rows -1\n")
        .code(1)
        .stderr(contains("`.max-rows` needs a whole number, found `-1`"));
}

#[test]
fn extra_arguments_to_no_arg_commands() {
    for cmd in [".help", ".tables", ".quit", ".exit"] {
        repl(&format!("{cmd} x\n")).code(1).stderr(format!(
            "error: `{cmd}` takes no argument\n  tip: type `{cmd}` on its own\n"
        ));
    }
    // `.quit x` is an error and does not leave the REPL.
    repl(".quit x\n.tables\n")
        .code(1)
        .stdout(contains("no host tables or saved lets yet"))
        .stderr(contains("error: `.quit` takes no argument"));
}

// ---- M2-31: duplicate --table and piped REPL coverage ----

#[test]
fn duplicate_table_flags_last_wins_in_tables_schema_and_queries() {
    ds().args(["--table", "o=orders.csv", "--table", "o=customers.csv"])
        .write_stdin(".tables\n.schema o\nfrom o | take 1 | project name;\n")
        .assert()
        .success()
        .stdout(
            contains(
                "name | kind       | path\n\
                 -----+------------+--------------\n\
                 o    | host table | customers.csv\n",
            )
            .and(contains("orders.csv").not())
            .and(contains("name       | string\n"))
            .and(contains("Ana Silva")),
        )
        .stderr("");
}

#[test]
fn warning_on_saved_let_is_reported_once() {
    repl(
        "let sorted = from 'orders.csv' | sort by orderId;\n\
         from sorted | take 1 | project orderId;\n\
         from sorted | count;\n",
    )
    .success()
    .stdout(contains("o-0001").and(contains("50")))
    .stderr(
        contains("warning[DS2103]")
            .count(1)
            .and(contains("--> <repl:1>:"))
            .and(contains("<repl:2>").not())
            .and(contains("<repl:3>").not()),
    );
}

#[test]
fn schema_on_host_table_and_arrow_only_column() {
    ds().args(["--table", "orders=orders.csv"])
        .write_stdin(".schema orders\n")
        .assert()
        .success()
        .stdout(
            contains("column     | type\n")
                .and(contains("orderId    | string\n"))
                .and(contains("amount     | real\n")),
        )
        .stderr("");

    repl(".schema 'types.parquet'\n")
        .success()
        .stdout(contains("bin").and(contains("arrow:Binary")))
        .stderr("");
}

#[test]
fn format_switches_to_table_handles_csv_mid_stream_error_and_rejects_unsupported() {
    // Switching from `csv` back to `table`.
    repl(
        ".format csv\n\
         from 'orders.csv' | take 1 | project orderId;\n\
         .format table\n\
         from 'orders.csv' | take 1 | project orderId;\n",
    )
    .success()
    .stdout(
        "orderId\n\
         o-0001\n\
         orderId\n\
         string\n\
         -------\n\
         o-0001\n",
    )
    .stderr("");

    // `ndjson` and `arrow` are CLI-only formats and rejected by `.format`.
    for fmt in ["ndjson", "arrow"] {
        repl(&format!(".format {fmt}\n"))
            .code(1)
            .stderr(contains(format!("error: unknown output format `{fmt}`")));
    }

    // Mid-stream runtime failure under `csv` keeps flushed batches and lets later inputs run.
    let dir = tempfile::tempdir().unwrap();
    let mut text = String::from("x\n");
    text.push_str(&"1\n".repeat(19_999));
    text.push_str("10000\n");
    std::fs::write(dir.path().join("big.csv"), text).unwrap();

    let output = ds()
        .current_dir(dir.path())
        .write_stdin(
            ".format csv\n\
             from 'big.csv' | extend y = x * 1000000000000000;\n\
             from 'big.csv' | take 1;\n",
        )
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("DS4003"), "{stderr}");
    assert!(stderr.contains("--> <repl:1>:"), "{stderr}");
    assert!(!stderr.contains("<repl:2>"), "{stderr}");
    assert!(
        stdout.starts_with("x,y\n1,1000000000000000\n"),
        "{stdout:.100}"
    );
    assert!(stdout.ends_with("\nx\n1\n"), "{stdout}");
    let lines = stdout.lines().count();
    assert!((8_195..20_005).contains(&lines), "{lines} lines");
}

#[test]
fn failed_let_redefinition_rolls_back_and_plural_removal_lists_dependents() {
    // A failed redefinition leaves the old `let` and its dependents intact.
    repl(
        "let x = 10;\n\
         let y = x + 5;\n\
         let x = nope;\n\
         from 'orders.csv' | take 1 | project orderId, v = y;\n",
    )
    .code(1)
    .stdout(contains("o-0001").and(contains("15")))
    .stderr(
        contains("--> <repl:3>:")
            .and(contains("info: removed").not())
            .and(contains("<repl:4>").not()),
    );

    // Redefining a `let` with several dependents prints the plural removal notice.
    repl(
        "let x = 1;\n\
         let y = x + 1;\n\
         let z = y * 2;\n\
         let w = z + 3;\n\
         let x = 10;\n",
    )
    .success()
    .stderr(contains(
        "info: removed `y`, `z` and `w`, which used `x`; enter them again to use the new definition",
    ));
}

#[test]
fn describe_meta_command_summarizes_sources_and_reports_errors() {
    // `.describe` on a file path.
    repl(".describe 'orders.csv'\n")
        .success()
        .stdout(contains(
            "column_name | column_type | count | null_percentage",
        ))
        .stdout(contains(
            "orderId     | string      |    50 |             0.0",
        ))
        .stdout(contains(
            "amount      | real        |    49 |             2.0",
        ))
        .stderr("");

    // `.describe` on a saved tabular let and a host table.
    ds().args(["--table", "orders=orders.csv"])
        .write_stdin("let small = from orders | project orderId, amount;\n.describe small\n.describe orders\n")
        .assert()
        .success()
        .stdout(contains("orderId"))
        .stdout(contains("customerId"))
        .stderr("");

    // `.describe` honors `.format csv`.
    repl(".format csv\n.describe 'orders.csv'\n")
        .success()
        .stdout(contains(
            "column_name,column_type,count,null_percentage,approx_unique,min,max,avg,std,q25,q50,q75\n",
        ))
        .stderr("");

    // `.describe` with no argument gives a mistake with a tip.
    repl(".describe\n")
        .code(1)
        .stderr(contains("error: `.describe` needs a source\n  tip: "));

    // `.describe` on a missing file points the diagnostic at the argument.
    repl(".describe 'ordrs.csv'\n")
        .code(1)
        .stderr(contains("error[DS3001]: file `ordrs.csv` not found"))
        .stderr(contains("--> <repl>:1:11"))
        .stderr(contains("a file with a similar name exists: `orders.csv`"));

    // `.help` lists `.describe`.
    repl(".help\n")
        .success()
        .stdout(contains(".describe <source>"));
}
