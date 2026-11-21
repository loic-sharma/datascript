//! Renderer tests: the documented examples, edge cases, and the JSON form.

use datascript_diagnostics::json::JsonError;
use datascript_diagnostics::{
    Applicability, Diagnostic, FileId, JsonReport, Renderer, SourceMap, Span, Suggestion, TextEdit,
    codes, render_json,
};

/// The span of the `nth` (0-based) occurrence of `needle` in the file's text.
fn find(sources: &SourceMap, file: FileId, needle: &str, nth: usize) -> Span {
    let text = sources.get(file).unwrap().text();
    let start = text
        .match_indices(needle)
        .nth(nth)
        .unwrap_or_else(|| panic!("`{needle}` not found"))
        .0 as u32;
    Span::new(file, start, start + needle.len() as u32)
}

fn plain(d: &Diagnostic, sources: &SourceMap) -> String {
    Renderer::plain().render(d, sources)
}

fn terminal(d: &Diagnostic, sources: &SourceMap) -> String {
    Renderer::terminal(false).render(d, sources)
}

// --- The three examples in DIAGNOSTICS.md -------------------------------------------------------

fn unknown_column() -> (Diagnostic, SourceMap) {
    let mut sources = SourceMap::new();
    let f = sources.add(
        "top_customers.ds",
        "'orders.csv'\n| where amount > 0\n| summarize revenue = sum(amout) by customerId\n",
    );
    let d = Diagnostic::error(codes::DS1001, "unknown column `amout`")
        .primary(
            find(&sources, f, "amout", 0),
            "no column named `amout` here",
        )
        .help_with(
            "a column with a similar name exists: `amount`",
            Suggestion::replace(find(&sources, f, "amout", 0), "amount"),
        )
        .note("columns available at this point: orderId, customerId, amount, timestamp");
    (d, sources)
}

fn assignment_not_comparison() -> (Diagnostic, SourceMap) {
    let mut sources = SourceMap::new();
    let f = sources.add("query.ds", "'orders.csv'\n| where status = \"shipped\"\n");
    let eq = find(&sources, f, "=", 0);
    let d = Diagnostic::error(codes::DS0110, "`=` is assignment, not comparison")
        .primary(eq, "assignment is not allowed here")
        .help_with(
            "use `==` to compare values: `status == \"shipped\"`",
            Suggestion::replace(eq, "=="),
        );
    (d, sources)
}

fn cannot_add() -> (Diagnostic, SourceMap) {
    let mut sources = SourceMap::new();
    let f = sources.add("query.ds", "'products.csv'\n| extend total = price + 1\n");
    let d = Diagnostic::error(codes::DS2003, "cannot add `string` and `long`")
        .primary(
            find(&sources, f, "price", 0),
            "this is `string` (column `price` was read from products.csv)",
        )
        .secondary(find(&sources, f, "1", 0), "this is `long`")
        .help("if `price` holds numbers, convert it first: `toreal(price) + 1`");
    (d, sources)
}

/// The fenced code blocks in a doc section that start with `error[`.
fn documented_examples(doc: &str, section: &str) -> Vec<String> {
    let doc = doc.replace('\r', "");
    let body = doc.split(section).nth(1).expect("section exists");
    let body = body.split("\n## ").next().unwrap();
    body.split("```")
        .skip(1)
        .step_by(2)
        .map(|block| block.trim_start_matches('\n').to_string())
        .filter(|block| block.starts_with("error["))
        .collect()
}

#[test]
fn terminal_output_matches_diagnostics_md() {
    let doc = include_str!("../../../docs/DIAGNOSTICS.md");
    let documented = documented_examples(doc, "## What a good diagnostic looks like");
    let rendered: Vec<String> = [unknown_column(), assignment_not_comparison(), cannot_add()]
        .iter()
        .map(|(d, sources)| terminal(d, sources))
        .collect();
    assert_eq!(rendered, documented);
}

#[test]
fn plain_output_matches_testing_md() {
    // The `---- expected` section of the `mode: check` example.
    let doc = include_str!("../../../docs/TESTING.md").replace('\r', "");
    let block = doc
        .split("```")
        .find(|block| block.contains("mode: check"))
        .expect("TESTING.md has a `mode: check` example");
    let expected = block
        .split("---- expected\n")
        .nth(1)
        .expect("the example has an expected section");

    let mut sources = SourceMap::new();
    let f = sources.add(
        "query",
        "from '../../data/orders.csv' | summarize sum(amout)\n",
    );
    let d = Diagnostic::error(codes::DS1001, "unknown column `amout`")
        .primary(
            find(&sources, f, "amout", 0),
            "no column named `amout` here",
        )
        .help("a column with a similar name exists: `amount`")
        .note("columns available at this point: orderId, customerId, amount, timestamp");
    assert_eq!(plain(&d, &sources), expected);
}

#[test]
fn snapshots_of_documented_examples() {
    for (name, (d, sources)) in [
        ("unknown_column", unknown_column()),
        ("assignment_not_comparison", assignment_not_comparison()),
        ("cannot_add", cannot_add()),
    ] {
        insta::assert_snapshot!(format!("{name}_plain"), plain(&d, &sources));
        insta::assert_snapshot!(format!("{name}_terminal"), terminal(&d, &sources));
    }
}

#[test]
fn terminal_colors() {
    let (d, sources) = cannot_add();
    let out = Renderer::terminal(true).render(&d, &sources);
    assert!(out.contains("\x1b[1;31merror[DS2003]\x1b[0m"));
    // Without escapes it is exactly the uncolored output.
    let stripped = strip_ansi(&out);
    assert_eq!(stripped, terminal(&d, &sources));
    insta::assert_snapshot!(out.replace('\x1b', "<ESC>"));
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            for c in chars.by_ref() {
                if c == 'm' {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

// --- Edge cases -----------------------------------------------------------------------------------

#[test]
fn no_primary_span() {
    let mut sources = SourceMap::new();
    let f = sources.add("query.ds", "let x = 1;\n");
    let d = Diagnostic::warning(codes::DS0004, "program has no query to run")
        .in_file(f)
        .help("add a query after the `let` statements, e.g. `from 'orders.csv' | take 10`");
    insta::assert_snapshot!("no_primary_plain", plain(&d, &sources));
    insta::assert_snapshot!("no_primary_terminal", terminal(&d, &sources));

    // No file either: header and help only.
    let d = Diagnostic::error(codes::DS9001, "internal compiler error")
        .note("this is a bug in DataScript; please report it");
    insta::assert_snapshot!("no_location_plain", plain(&d, &sources));
}

#[test]
fn multi_edit_suggestion() {
    let mut sources = SourceMap::new();
    let f = sources.add("query.ds", "T | where a = 1 && b = 2\n");
    let suggestion = Suggestion::new(
        vec![
            TextEdit::new(find(&sources, f, "=", 1), "=="),
            TextEdit::new(find(&sources, f, "&&", 0), "and"),
            TextEdit::new(find(&sources, f, "=", 0), "=="),
        ],
        Applicability::MaybeIncorrect,
    );
    let d = Diagnostic::error(codes::DS0111, "`&&` is not an operator in DataScript")
        .primary(find(&sources, f, "&&", 0), "use `and` here")
        .help_with("write it as `a == 1 and b == 2`", suggestion);
    insta::assert_snapshot!("multi_edit_suggestion_plain", plain(&d, &sources));
    insta::assert_snapshot!("multi_edit_suggestion_json", render_json(&[d], &sources));
}

#[test]
fn multi_line_span() {
    let mut sources = SourceMap::new();
    let text = "T\n| summarize total = sum(\n    amount,\n    price)\n| take 3\n";
    let f = sources.add("query.ds", text);
    let start = text.find("sum(").unwrap() as u32;
    let end = text.find("price)").unwrap() as u32 + 6;
    let d = Diagnostic::error(codes::DS2002, "`sum` takes 1 argument, but 2 were given")
        .primary(Span::new(f, start, end), "called with 2 arguments here");
    insta::assert_snapshot!("multi_line_plain", plain(&d, &sources));
}

#[test]
fn span_at_eof_and_empty_span() {
    let mut sources = SourceMap::new();
    let f = sources.add("query.ds", "T | where");
    let d = Diagnostic::error(codes::DS0101, "expected a condition after `where`")
        .primary(Span::empty(f, 9), "the query ends here");
    insta::assert_snapshot!("eof_plain", plain(&d, &sources));

    // EOF after a trailing newline points at the final empty line.
    let g = sources.add("trailing.ds", "T |\n");
    let d = Diagnostic::error(codes::DS0103, "`|` with no operator after it")
        .primary(Span::empty(g, 4), "expected an operator here");
    insta::assert_snapshot!("eof_after_newline_plain", plain(&d, &sources));
}

#[test]
fn tabs_are_expanded() {
    let mut sources = SourceMap::new();
    let f = sources.add("query.ds", "T\n|\twhere\tx = 1\n");
    let d = Diagnostic::error(codes::DS0110, "`=` is assignment, not comparison")
        .primary(find(&sources, f, "=", 0), "assignment is not allowed here");
    let out = plain(&d, &sources);
    // Column counts the tabs as one character each: `|`, tab, `where`, tab, `x`, space.
    assert!(out.contains("--> query.ds:2:11"), "{out}");
    insta::assert_snapshot!("tabs_plain", out);
}

#[test]
fn non_ascii_text() {
    let mut sources = SourceMap::new();
    let f = sources.add("query.ds", "T | where ville == \"Zürich\" and prïce > 1\n");
    let d = Diagnostic::error(codes::DS1001, "unknown column `prïce`").primary(
        find(&sources, f, "prïce", 0),
        "no column named `prïce` here",
    );
    let out = plain(&d, &sources);
    // Columns count characters, not bytes: `ü` is one column.
    assert!(out.contains("--> query.ds:1:33"), "{out}");
    insta::assert_snapshot!("non_ascii_plain", out);
}

#[test]
fn labels_far_apart_and_in_two_files() {
    let mut sources = SourceMap::new();
    let lib = sources.add("lib.ds", "let threshold = \"10\";\n");
    let main = sources.add(
        "query.ds",
        "let a = 1;\n// one\n// two\n// three\nT | where amount > threshold\n",
    );
    let d = Diagnostic::error(codes::DS2003, "cannot compare `long` and `string`")
        .primary(find(&sources, main, "threshold", 0), "this is `string`")
        .secondary(find(&sources, main, "a", 0), "unrelated, but far away")
        .secondary(find(&sources, lib, "\"10\"", 0), "defined as a string here");
    insta::assert_snapshot!("two_files_plain", plain(&d, &sources));
    insta::assert_snapshot!("two_files_terminal", terminal(&d, &sources));
}

#[test]
fn three_labels_on_one_line() {
    let mut sources = SourceMap::new();
    let f = sources.add("query.ds", "T | extend x = a + b + c\n");
    let d = Diagnostic::error(codes::DS2003, "cannot add these values")
        .primary(find(&sources, f, "a", 0), "this is `string`")
        .secondary(find(&sources, f, "b", 0), "this is `long`")
        .secondary(find(&sources, f, "c", 0), "this is `bool`");
    insta::assert_snapshot!("three_labels_plain", plain(&d, &sources));
}

#[test]
fn render_all_separates_with_blank_lines() {
    let (a, sources) = assignment_not_comparison();
    let out = Renderer::plain().render_all(&[a.clone(), a], &sources);
    assert_eq!(out.matches("\n\nerror[DS0110]").count(), 1);
}

// --- JSON ----------------------------------------------------------------------------------------

#[test]
fn json_round_trip() {
    for (d, sources) in [unknown_column(), assignment_not_comparison(), cannot_add()] {
        let report = JsonReport::new(std::slice::from_ref(&d), &sources);
        let text = report.to_json_pretty();
        let parsed: JsonReport = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed, report);
        assert_eq!(parsed.to_diagnostics(&sources).unwrap(), vec![d]);
    }
}

#[test]
fn json_round_trip_without_primary() {
    let mut sources = SourceMap::new();
    let f = sources.add("query.ds", "let x = 1;\n");
    let d = Diagnostic::warning(codes::DS0004, "program has no query to run").in_file(f);
    let report = JsonReport::new(std::slice::from_ref(&d), &sources);
    assert_eq!(report.diagnostics[0].file.as_deref(), Some("query.ds"));
    assert!(report.diagnostics[0].primary.is_none());
    assert_eq!(report.to_diagnostics(&sources).unwrap(), vec![d]);
}

#[test]
fn json_positions_are_utf16() {
    let mut sources = SourceMap::new();
    // `😀` is outside the BMP: 4 bytes, 2 UTF-16 code units, 1 character.
    let text = "T | where note == \"😀\"\n| where 😀x > 1\n";
    let f = sources.add("query.ds", text);
    let span = find(&sources, f, "😀x", 0);
    let d = Diagnostic::error(codes::DS1001, "unknown column `😀x`").primary(span, "");
    let report = JsonReport::new(std::slice::from_ref(&d), &sources);
    let json = &report.diagnostics[0].primary.as_ref().unwrap().span;

    let utf16: Vec<u16> = text.encode_utf16().collect();
    let expected_start = text[..span.start as usize].encode_utf16().count() as u32;
    assert_eq!(json.start, expected_start);
    assert_eq!(json.end, expected_start + 3);
    assert_eq!(
        String::from_utf16(&utf16[json.start as usize..json.end as usize]).unwrap(),
        "😀x"
    );
    // Line 2, `| where ` is 8 code units.
    assert_eq!((json.line, json.column), (2, 9));
    assert_eq!((json.end_line, json.end_column), (2, 12));
    // The plain renderer counts characters instead.
    assert!(plain(&d, &sources).contains("--> query.ds:2:9"));

    let value: serde_json::Value = serde_json::from_str(&report.to_json_pretty()).unwrap();
    assert_eq!(value["version"], 1);
    assert_eq!(value["diagnostics"][0]["primary"]["span"]["endColumn"], 12);
    assert_eq!(report.to_diagnostics(&sources).unwrap(), vec![d]);
}

#[test]
fn json_rejects_bad_input() {
    let (d, sources) = cannot_add();
    let mut report = JsonReport::new(&[d], &sources);
    report.version = 2;
    assert_eq!(
        report.to_diagnostics(&sources),
        Err(JsonError::UnsupportedVersion(2))
    );
    report.version = 1;
    report.diagnostics[0].code = "E0001".into();
    assert!(matches!(
        report.to_diagnostics(&sources),
        Err(JsonError::InvalidCode(_))
    ));
    report.diagnostics[0].code = "DS2003".into();
    report.diagnostics[0].primary.as_mut().unwrap().span.file = "nope.ds".into();
    assert!(matches!(
        report.to_diagnostics(&sources),
        Err(JsonError::UnknownFile(_))
    ));
}

#[test]
fn json_snapshot() {
    let (d, sources) = cannot_add();
    insta::assert_snapshot!(render_json(&[d], &sources));
}
