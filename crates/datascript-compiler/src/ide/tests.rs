//! Completion, hover and signature help, one test per context. `‸` marks the cursor.

use std::sync::Arc;

use arrow_schema::{DataType, Field, Schema};
use datascript_diagnostics::{FileId, SourceFile};

use super::*;
use crate::{Capabilities, NoSchemas, SchemaLookup, SchemaProvider, SourceError, SourceRef, check};

/// Knows `orders.csv` and `data/orders.csv`.
struct Orders;

impl SchemaProvider for Orders {
    fn schema_of(&self, source: &SourceRef) -> SchemaLookup {
        match source.path() {
            Some("orders.csv" | "data/orders.csv") => {
                SchemaLookup::Ready(Arc::new(Schema::new(vec![
                    Field::new("orderId", DataType::Utf8, true),
                    Field::new("customerId", DataType::Utf8, true),
                    Field::new("amount", DataType::Float64, true),
                    Field::new("order date", DataType::Utf8, true),
                ])))
            }
            _ => SchemaLookup::Failed(SourceError::NotFound { nearby: Vec::new() }),
        }
    }
}

fn split(marked: &str) -> (SourceFile, usize) {
    let offset = marked.find('‸').expect("a cursor mark");
    (SourceFile::new("q.ds", marked.replace('‸', "")), offset)
}

fn analyze(src: &SourceFile, schemas: &dyn SchemaProvider) -> crate::Analysis {
    check(src, FileId(0), schemas, &Capabilities::full())
}

fn completions_with(marked: &str, schemas: &dyn SchemaProvider) -> Completions {
    let (src, offset) = split(marked);
    let analysis = analyze(&src, schemas);
    complete(&src, &analysis, offset)
}

fn completions(marked: &str) -> Completions {
    completions_with(marked, &Orders)
}

fn labels(marked: &str) -> Vec<String> {
    completions(marked)
        .items
        .into_iter()
        .map(|i| i.label)
        .collect()
}

fn labels_of(c: &Completions, kind: CompletionKind) -> Vec<&str> {
    c.items
        .iter()
        .filter(|i| i.kind == kind)
        .map(|i| i.label.as_str())
        .collect()
}

#[track_caller]
fn assert_has(labels: &[String], expected: &[&str]) {
    for e in expected {
        assert!(labels.iter().any(|l| l == e), "missing `{e}` in {labels:?}");
    }
}

#[track_caller]
fn assert_lacks(labels: &[String], unexpected: &[&str]) {
    for e in unexpected {
        assert!(
            !labels.iter().any(|l| l == e),
            "unexpected `{e}` in {labels:?}"
        );
    }
}

const COLUMNS: [&str; 4] = ["orderId", "customerId", "amount", "['order date']"];

#[test]
fn statement_starts() {
    for marked in ["‸", "fr‸", "let x = 1;\n‸", "from 'orders.csv';\nf‸"] {
        assert_eq!(labels(marked), ["from", "let"], "{marked}");
    }
    let c = completions("let x = 1;\nfr‸");
    assert_eq!((c.start, c.end), (11, 13));
}

#[test]
fn operators_after_a_pipe() {
    for marked in [
        "from 'orders.csv' | ‸",
        "from 'orders.csv' | wh‸",
        "from 'orders.csv'\n| ‸\n| take 5",
        "from 'orders.csv' | project-aw‸",
        "from 'orders.csv' | project-‸",
    ] {
        let c = completions(marked);
        assert_eq!(
            labels_of(&c, CompletionKind::Operator).len(),
            datascript_syntax::OPERATOR_NAMES.len(),
            "{marked}"
        );
        assert_eq!(
            c.items.len(),
            datascript_syntax::OPERATOR_NAMES.len(),
            "{marked}"
        );
    }
    // The hyphenated name being typed is replaced whole.
    let c = completions("from 'orders.csv' | project-aw‸");
    assert_eq!((c.start, c.end), (20, 30));
    let where_op = completions("from t | ‸")
        .items
        .into_iter()
        .find(|i| i.label == "where")
        .unwrap();
    assert_eq!(
        where_op.detail.as_deref(),
        Some("Keeps the rows where the predicate is true.")
    );
}

#[test]
fn columns_in_incomplete_code() {
    let c = completions("from 'orders.csv' | where am‸");
    assert_eq!(labels_of(&c, CompletionKind::Column), COLUMNS);
    assert_eq!((c.start, c.end), (26, 28));
    let amount = c.items.iter().find(|i| i.label == "amount").unwrap();
    assert_eq!(amount.detail.as_deref(), Some("real"));
    let functions = labels_of(&c, CompletionKind::Function);
    assert!(functions.contains(&"strlen"));
    assert!(!functions.contains(&"sum"), "no aggregates in `where`");
    assert!(!functions.contains(&"read_csv"), "readers are tables");
    let strlen = c.items.iter().find(|i| i.label == "strlen").unwrap();
    assert!(strlen.snippet);
    assert_eq!(strlen.insert, "strlen($0)");
    assert_eq!(
        strlen.detail.as_deref(),
        Some("strlen(source: string) -> long")
    );

    // Columns come from the stage before the cursor's operator.
    let c = completions("from 'orders.csv' | project orderId, amount | where ‸ | take 1");
    assert_eq!(labels_of(&c, CompletionKind::Column), ["orderId", "amount"]);
    let c = completions("from 'orders.csv' | extend x = 1 | project ‸");
    assert_eq!(
        labels_of(&c, CompletionKind::Column),
        [&COLUMNS[..], &["x"]].concat()
    );
    // A pipeline in parentheses, and a tabular `let`.
    let c = completions("(from 'orders.csv' | project amount) | where ‸");
    assert_eq!(labels_of(&c, CompletionKind::Column), ["amount"]);
    let c = completions("let t = from 'orders.csv' | project orderId;\nfrom t | where ‸");
    assert_eq!(labels_of(&c, CompletionKind::Column), ["orderId"]);
    // After a stage with an error in its expression, the columns are still known.
    let c = completions("from 'orders.csv' | where amout > 1 | project ‸");
    assert_eq!(labels_of(&c, CompletionKind::Column), COLUMNS);
}

#[test]
fn aggregates_only_where_allowed() {
    let c = completions("from 'orders.csv' | summarize ‸");
    let functions = labels_of(&c, CompletionKind::Function);
    assert!(functions.contains(&"sum") && functions.contains(&"strlen"));
    assert_eq!(labels_of(&c, CompletionKind::Column), COLUMNS);
    let c = completions("from 'orders.csv' | summarize count() by ‸");
    assert!(!labels_of(&c, CompletionKind::Function).contains(&"sum"));
    assert_eq!(labels_of(&c, CompletionKind::Column), COLUMNS);
    let count = completions("from t | summarize c‸")
        .items
        .into_iter()
        .find(|i| i.label == "count")
        .unwrap();
    assert_eq!(count.insert, "count()");
}

#[test]
fn keywords_after_a_complete_expression() {
    let l = labels("from 'orders.csv' | where amount > 1 an‸");
    assert_has(&l, &["and", "or", "has", "contains", "between", "in"]);
    assert_lacks(&l, &["amount", "strlen"]);
    assert_has(&labels("from 'orders.csv' | summarize count() ‸"), &["by"]);
    assert_eq!(labels("from 'orders.csv' | sort ‸"), ["by"]);
    assert_eq!(
        labels("from 'orders.csv' | sort by amount ‸"),
        ["asc", "desc", "nulls first", "nulls last"]
    );
    assert_eq!(labels("from 'orders.csv' | top 5 ‸"), ["by"]);
    assert_has(&labels("from 'orders.csv' | top 5 by ‸"), &COLUMNS);
    assert_eq!(
        labels("from 'orders.csv' | top 5 by amount ‸"),
        ["asc", "desc", "nulls first", "nulls last"]
    );
    assert_eq!(labels("from 'orders.csv' | count ‸"), Vec::<String>::new());
}

#[test]
fn tables_after_from() {
    let prelude = "let big = from 'orders.csv';\nlet p = 'orders.csv';\nlet n = 1;\n";
    for tail in [
        "from ‸",
        "from b‸",
        "from (from ‸",
        "from 'orders.csv' | join (from ‸",
    ] {
        let c = completions(&format!("{prelude}{tail}"));
        assert_eq!(labels_of(&c, CompletionKind::Table), ["big"], "{tail}");
        assert_eq!(labels_of(&c, CompletionKind::Variable), ["p"], "{tail}");
        assert_eq!(
            labels_of(&c, CompletionKind::Function),
            ["read_csv", "read_json", "read_parquet"],
            "{tail}"
        );
        assert_eq!(c.items.len(), 5, "only tables and paths: {tail}");
        assert_eq!(
            c.files,
            Some(FileCompletion {
                dir: String::new(),
                quote: None
            })
        );
    }
    let c = completions(&format!("{prelude}from ‸"));
    let read_csv = c.items.iter().find(|i| i.label == "read_csv").unwrap();
    assert_eq!(read_csv.insert, "read_csv('$1')");
    let p = c.items.iter().find(|i| i.label == "p").unwrap();
    assert_eq!(p.detail.as_deref(), Some("'orders.csv'"));
    // A `let` defined later is not in scope.
    assert_eq!(
        labels("from ‸;\nlet later = from 'orders.csv'"),
        ["read_csv", "read_json", "read_parquet"]
    );
}

#[test]
fn tables_on_the_right_of_join_and_union() {
    let prelude = "let big = from 'orders.csv';\nlet p = 'orders.csv';\n";
    let c = completions(&format!("{prelude}from 'orders.csv' | join ‸"));
    assert_eq!(labels_of(&c, CompletionKind::Keyword), ["kind ="]);
    assert_eq!(labels_of(&c, CompletionKind::Table), ["big"]);
    assert_eq!(labels_of(&c, CompletionKind::Variable), ["p"]);
    assert!(
        labels_of(&c, CompletionKind::Function).is_empty(),
        "no `from`, so no readers"
    );
    assert!(c.files.is_some());
    let l = labels("from 'orders.csv' | join kind=‸");
    assert_eq!(l.len(), 8);
    assert_has(&l, &["inner", "leftouter", "rightanti"]);
    let c = completions(&format!("{prelude}from 'orders.csv' | join kind=inner ‸"));
    assert_eq!(labels_of(&c, CompletionKind::Table), ["big"]);
    assert_eq!(
        labels(&format!("{prelude}from 'orders.csv' | join big ‸")),
        ["on"]
    );
    assert_has(
        &labels(&format!("{prelude}from 'orders.csv' | join big on ‸")),
        &COLUMNS,
    );
    for tail in ["union ‸", "union big, ‸"] {
        let c = completions(&format!("{prelude}from 'orders.csv' | {tail}"));
        assert_eq!(labels_of(&c, CompletionKind::Table), ["big"], "{tail}");
        assert!(c.files.is_some());
    }
}

#[test]
fn render_kinds_properties_and_values() {
    let l = labels("from 'orders.csv' | render ‸");
    assert_eq!(l.len(), 9);
    assert_has(&l, &["table", "timechart", "barchart", "card"]);
    assert_eq!(labels("from 'orders.csv' | render barchart ‸"), ["with"]);
    let c = completions("from 'orders.csv' | render barchart with (‸");
    let props = labels_of(&c, CompletionKind::Property);
    assert_eq!(props.len(), 10);
    let xcolumn = c.items.iter().find(|i| i.label == "xcolumn").unwrap();
    assert_eq!(xcolumn.insert, "xcolumn = $0");
    assert_eq!(
        labels("from 'orders.csv' | render table with (‸"),
        ["title"]
    );
    assert_lacks(
        &labels("from 'orders.csv' | render piechart with (title = 'x', ‸"),
        &["series", "kind"],
    );
    assert_eq!(
        labels("from 'orders.csv' | render barchart with (xcolumn = ‸"),
        COLUMNS
    );
    // `ycolumns = a, ` may go on with columns or start the next property.
    assert_has(
        &labels("from 'orders.csv' | project amount | render barchart with (ycolumns = amount, ‸"),
        &["amount", "series", "title"],
    );
    assert_eq!(
        labels("from 'orders.csv' | render barchart with (legend = h‸"),
        ["hidden", "visible"]
    );
    assert_eq!(
        labels("from 'orders.csv' | render barchart with (kind = ‸"),
        ["default", "stacked", "stacked100", "unstacked"]
    );
    assert!(labels("from 'orders.csv' | render barchart with (title = ‸").is_empty());
}

#[test]
fn reader_options_and_values() {
    let l = labels("from read_csv('orders.csv', ‸");
    assert_has(&l, &["delimiter", "header", "compression", "infer_rows"]);
    assert_lacks(&l, &["schema", "format"]);
    assert_eq!(
        labels("from read_json('a.json', ‸"),
        ["compression", "format"]
    );
    assert_eq!(
        labels("from read_csv('orders.csv', header = ‸"),
        ["false", "true"]
    );
    assert_eq!(
        labels("from read_csv('orders.csv', compression = ‸"),
        ["'bzip2'", "'gzip'", "'xz'", "'zstd'"]
    );
    assert!(labels("from read_csv('orders.csv', delimiter = ‸").is_empty());
    // The first argument is a path.
    let c = completions("let p = 'orders.csv';\nfrom read_csv(‸");
    assert_eq!(labels_of(&c, CompletionKind::Variable), ["p"]);
    assert!(c.files.is_some());
    // After the call, its options are behind us.
    assert!(
        completions("from read_csv('orders.csv') ‸")
            .items
            .is_empty()
    );
}

#[test]
fn file_names_inside_path_strings() {
    let c = completions("from 'or‸");
    assert_eq!(
        c.files,
        Some(FileCompletion {
            dir: String::new(),
            quote: Some('\'')
        })
    );
    assert_eq!((c.start, c.end), (6, 8));
    assert!(c.items.is_empty());
    let c = completions("from \"data/or‸ders.csv\" | take 1");
    assert_eq!(
        c.files,
        Some(FileCompletion {
            dir: "data/".into(),
            quote: Some('"')
        })
    );
    assert_eq!((c.start, c.end), (11, 21));
    for marked in [
        "from read_csv('sub/‸",
        "from 'orders.csv' | join @'x‸' on a",
        "from 'orders.csv' | union '‸",
    ] {
        assert!(completions(marked).files.is_some(), "{marked}");
    }
    // Other strings are not paths.
    for marked in [
        "from 'orders.csv' | where orderId == 'x‸",
        "from read_csv('orders.csv', delimiter = '‸",
        "let p = 'or‸",
    ] {
        let c = completions(marked);
        assert!(c.files.is_none() && c.items.is_empty(), "{marked}: {c:?}");
    }
    // Before the opening quote, the string is not entered yet.
    assert!(
        completions("from ‸'orders.csv'")
            .files
            .is_some_and(|f| f.quote.is_none())
    );
}

#[test]
fn nothing_in_comments() {
    for marked in ["// from ‸", "from t | /* where ‸ */ take 1", "from t // ‸"] {
        assert!(completions(marked).items.is_empty(), "{marked}");
    }
}

#[test]
fn column_only_operators() {
    let c = completions("from 'orders.csv' | project-away ‸");
    assert_eq!(labels_of(&c, CompletionKind::Column), COLUMNS);
    assert_eq!(c.items.len(), 4, "columns only");
    assert_eq!(
        labels("from 'orders.csv' | project-away amount, ‸"),
        labels("from 'orders.csv' | project-away ‸")
    );
    let l = labels("from 'orders.csv' | distinct ‸");
    assert_has(&l, &COLUMNS);
    assert_has(&l, &["*"]);
    assert_eq!(
        labels("from 'orders.csv' | project-rename total = ‸").len(),
        4
    );
    assert!(labels("from 'orders.csv' | project-rename ‸").is_empty());
}

#[test]
fn let_values_and_scalar_lets() {
    let l = labels("let x = ‸");
    assert_has(&l, &["from", "strlen", "true"]);
    let c = completions("let a = 1;\nlet b = a + ‸");
    assert_eq!(labels_of(&c, CompletionKind::Variable), ["a"]);
    let c = completions(
        "let limit_ = 10;\nlet t = from 'orders.csv';\nfrom 'orders.csv' | where amount > ‸",
    );
    assert_eq!(labels_of(&c, CompletionKind::Variable), ["limit_"]);
    assert!(
        labels_of(&c, CompletionKind::Table).is_empty(),
        "a table is not a value"
    );
    assert!(labels("let ‸").is_empty());
}

#[test]
fn unknown_schemas_give_no_columns() {
    let c = completions_with("from 'orders.csv' | where ‸", &NoSchemas);
    assert!(labels_of(&c, CompletionKind::Column).is_empty());
    assert!(labels_of(&c, CompletionKind::Function).contains(&"strlen"));
}

// --- Hover ---

fn hover_at(marked: &str) -> Option<String> {
    let (src, offset) = split(marked);
    let analysis = analyze(&src, &Orders);
    hover(&src, &analysis, offset).map(|h| h.markdown)
}

#[test]
fn hover_shows_columns_lets_functions_and_operators() {
    assert_eq!(
        hover_at("from 'orders.csv' | where am‸ount > 1").as_deref(),
        Some("```ds\namount: real\n```\ncolumn")
    );
    assert_eq!(
        hover_at("from 'orders.csv' | project ['order date'‸]").as_deref(),
        Some("```ds\n['order date']: string\n```\ncolumn")
    );
    let strlen = hover_at("from 'orders.csv' | extend n = str‸len(orderId)").unwrap();
    assert!(
        strlen.starts_with("```ds\nstrlen(source: string) -> long\n```\n"),
        "{strlen}"
    );
    assert_eq!(
        hover_at("let n = 5;\nfrom 'orders.csv' | take n‸").as_deref(),
        Some("```ds\nlet n: long\n```")
    );
    assert_eq!(
        hover_at("let n‸ = 5;").as_deref(),
        Some("```ds\nlet n: long\n```")
    );
    let table = hover_at("let t = from 'orders.csv' | project amount;\nfrom t‸").unwrap();
    assert_eq!(table, "```ds\nlet t: table\n```\n\n- `amount`: real");
    let source = hover_at("from 'orders‸.csv' | take 1").unwrap();
    assert!(
        source.starts_with("table `'orders.csv'`\n\n- `orderId`: string"),
        "{source}"
    );
    let op = hover_at("from 'orders.csv' | project-aw‸ay amount").unwrap();
    assert!(
        op.starts_with("```ds\nproject-away\n```\nRemoves columns"),
        "{op}"
    );
    assert_eq!(hover_at("from 'orders.csv' | where 1 > 2‸"), None);
    assert_eq!(hover_at("from 'orders.csv' | where nothing‸ > 2"), None);
}

// --- Signature help ---

fn signature_at(marked: &str) -> Option<SignatureHelp> {
    let (src, offset) = split(marked);
    let analysis = analyze(&src, &Orders);
    signature_help(&src, &analysis, offset)
}

#[test]
fn signature_help_tracks_the_argument() {
    let help = signature_at("from 'orders.csv' | where strlen(‸").unwrap();
    assert_eq!(help.active_argument, 0);
    let sig = &help.signatures[help.active_signature];
    assert_eq!(sig.label, "strlen(source: string) -> long");
    assert_eq!(
        &sig.label[sig.parameters[0].0..sig.parameters[0].1],
        "source: string"
    );
    assert_eq!(sig.active_parameter, Some(0));

    let help = signature_at("from 'orders.csv' | extend s = substring(orderId, 1, ‸)").unwrap();
    assert_eq!(help.active_argument, 2);
    let sig = &help.signatures[help.active_signature];
    let p = sig.parameters[sig.active_parameter.unwrap()];
    assert_eq!(&sig.label[p.0..p.1], "length: integer");

    // The innermost open call wins; a closed one before the cursor does not count.
    let help = signature_at("from 'orders.csv' | where strlen(tolower(orderId‸))").unwrap();
    assert!(help.signatures[0].label.starts_with("tolower("));
    let help = signature_at("from 'orders.csv' | where strlen(tolower(orderId), ‸").unwrap();
    assert!(help.signatures[0].label.starts_with("strlen("));
    assert_eq!(
        help.signatures[0].active_parameter, None,
        "strlen takes one argument"
    );
    assert!(signature_at("from 'orders.csv' | where strlen(orderId)‸ > 1").is_none());
    assert!(signature_at("from 'orders.csv' | where nothing(‸").is_none());
    assert!(signature_at("from 'orders.csv' | where amount in (1, ‸").is_none());
}

#[test]
fn signature_help_with_trailing_comma() {
    // Before the trailing comma, `written` counts actual arguments, not commas + 1.
    let help = signature_at("from 'orders.csv' | extend x = round(am‸ount,)").unwrap();
    assert_eq!(help.active_argument, 0);
    let sig = &help.signatures[help.active_signature];
    let p = sig.parameters[sig.active_parameter.unwrap()];
    assert_eq!(&sig.label[p.0..p.1], "x: T");

    // After the trailing comma, the cursor is on parameter 1 (`digits`).
    let help = signature_at("from 'orders.csv' | extend x = round(amount, ‸)").unwrap();
    assert_eq!(help.active_argument, 1);
    let sig = &help.signatures[help.active_signature];
    let p = sig.parameters[sig.active_parameter.unwrap()];
    assert_eq!(&sig.label[p.0..p.1], "digits: integer");

    // A 1-arg function still highlights parameter 0 before a trailing comma, and none after it.
    let help = signature_at("from 'orders.csv' | extend n = strlen(ord‸erId,)").unwrap();
    assert_eq!(help.active_argument, 0);
    assert_eq!(
        help.signatures[help.active_signature].active_parameter,
        Some(0)
    );
    let help = signature_at("from 'orders.csv' | extend n = strlen(orderId, ‸)").unwrap();
    assert_eq!(help.active_argument, 1);
    assert_eq!(
        help.signatures[help.active_signature].active_parameter,
        None
    );

    // Variadic `case` with a trailing comma still recognizes `else` before the comma.
    let help =
        signature_at("from 'orders.csv' | extend s = case(amount > 1, 'hi', 'l‸o',)").unwrap();
    assert_eq!(help.active_argument, 2);
    let sig = &help.signatures[help.active_signature];
    let p = sig.parameters[sig.active_parameter.unwrap()];
    assert_eq!(&sig.label[p.0..p.1], "else: T");
}

#[test]
fn completions_after_trailing_commas() {
    let prelude = "let big = from 'orders.csv';\n";

    // Function and reader calls with a trailing comma before `)`.
    let c = completions("from 'orders.csv' | extend s = strcat(orderId, ‸)");
    assert_eq!(labels_of(&c, CompletionKind::Column), COLUMNS);
    assert!(labels_of(&c, CompletionKind::Function).contains(&"tolower"));
    let l = labels("from read_csv('orders.csv', delimiter = ',', ‸)");
    assert_has(&l, &["header", "compression", "infer_rows"]);

    // Operator lists with a trailing comma before `|`.
    let c = completions("from 'orders.csv' | project amount, ‸ | take 5");
    assert_eq!(labels_of(&c, CompletionKind::Column), COLUMNS);
    let c = completions(&format!(
        "{prelude}from 'orders.csv' | union big, ‸ | take 5"
    ));
    assert_eq!(labels_of(&c, CompletionKind::Table), ["big"]);
    assert!(c.files.is_some());
    let c = completions("from 'orders.csv' | project-away amount, ‸ | take 5");
    assert_eq!(labels_of(&c, CompletionKind::Column), COLUMNS);
    assert_eq!(c.items.len(), 4);
    let c = completions("from 'orders.csv' | distinct amount, ‸ | take 5");
    assert_eq!(labels_of(&c, CompletionKind::Column), COLUMNS);
    assert_eq!(c.items.len(), 4);

    // `summarize` trailing commas before `by` and after `by`.
    let c = completions("from 'orders.csv' | summarize sum(amount), ‸ by customerId");
    assert!(labels_of(&c, CompletionKind::Function).contains(&"count"));
    let c = completions("from 'orders.csv' | summarize sum(amount) by customerId, ‸ | take 5");
    assert_eq!(labels_of(&c, CompletionKind::Column), COLUMNS);
    assert!(!labels_of(&c, CompletionKind::Function).contains(&"sum"));

    // `render ... with (...)` with a trailing comma before `)`.
    let l = labels("from 'orders.csv' | render piechart with (title = 'x', ‸)");
    assert_has(&l, &["xcolumn", "ycolumns", "legend"]);
    assert_lacks(&l, &["series", "kind"]);
    assert_has(
        &labels("from 'orders.csv' | project amount | render barchart with (ycolumns = amount, ‸)"),
        &["amount", "series", "title"],
    );
}
