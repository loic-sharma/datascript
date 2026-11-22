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

/// `ds` (no command) running in `tests/data`, reading `input` from a pipe.
fn repl(input: &str) -> assert_cmd::assert::Assert {
    Command::new(env!("CARGO_BIN_EXE_ds"))
        .current_dir(data_dir())
        .env_remove("NO_COLOR")
        .write_stdin(input)
        .assert()
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
    Command::new(env!("CARGO_BIN_EXE_ds"))
        .current_dir(data_dir())
        .args(["--table", "orders=orders.csv"])
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
    Command::new(env!("CARGO_BIN_EXE_ds"))
        .current_dir(dir.path())
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
    Command::new(env!("CARGO_BIN_EXE_ds"))
        .current_dir(data_dir())
        .args(["--table", "t=orders.txt"])
        .write_stdin("")
        .assert()
        .code(2)
        .stderr(contains("DS3002"));
}

#[test]
fn diagnostics_are_plain_even_with_color_always_requested_by_auto() {
    repl("from nope;\n").code(1).stderr(contains("\x1b[").not());
}
