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
    assert!(labels_of(&c, CompletionKind::Keyword).contains(&"by"));
    let l = labels("from 'orders.csv' | summarize sum(amount), ‸");
    assert_has(&l, &["count", "by"]);
    let l = labels("from 'orders.csv' | summarize round(sum(amount), ‸)");
    assert_lacks(&l, &["by"]);
    let c = completions("from 'orders.csv' | summarize sum(amount) by customerId, ‸ | take 5");
    assert_eq!(labels_of(&c, CompletionKind::Column), COLUMNS);
    assert!(!labels_of(&c, CompletionKind::Function).contains(&"sum"));
    assert!(!labels_of(&c, CompletionKind::Keyword).contains(&"by"));

    // `render ... with (...)` with a trailing comma before `)`.
    let l = labels("from 'orders.csv' | render piechart with (title = 'x', ‸)");
    assert_has(&l, &["xcolumn", "ycolumns", "legend"]);
    assert_lacks(&l, &["series", "kind"]);
    assert_has(
        &labels("from 'orders.csv' | project amount | render barchart with (ycolumns = amount, ‸)"),
        &["amount", "series", "title"],
    );
}

#[test]
fn arrow_only_column_in_completion_and_hover() {
    struct Nested;
    impl SchemaProvider for Nested {
        fn schema_of(&self, _: &SourceRef) -> SchemaLookup {
            let item = Arc::new(Field::new("item", DataType::Int64, true));
            SchemaLookup::Ready(Arc::new(Schema::new(vec![
                Field::new("id", DataType::Int64, true),
                Field::new("tags", DataType::List(item), true),
            ])))
        }
    }
    let list_type = DataType::List(Arc::new(Field::new("item", DataType::Int64, true))).to_string();

    let c = completions_with("from 'x.csv' | project ‸", &Nested);
    let tags = c.items.iter().find(|i| i.label == "tags").unwrap();
    assert_eq!(tags.detail.as_deref(), Some(list_type.as_str()));

    let c = completions_with(
        "from 'x.csv' | project id, tags | project-rename t = ‸",
        &Nested,
    );
    let tags = c.items.iter().find(|i| i.label == "tags").unwrap();
    assert_eq!(tags.detail.as_deref(), Some(list_type.as_str()));

    let (src, offset) = split("from 'x.csv' | project ta‸gs");
    let analysis = analyze(&src, &Nested);
    assert_eq!(
        hover(&src, &analysis, offset).map(|h| h.markdown),
        Some(format!("```ds\ntags: {list_type}\n```\ncolumn"))
    );
}

#[test]
fn signature_help_positional_and_multi_signature() {
    // `coalesce` repeats parameters all named `value`: active_parameter must track position.
    let help = signature_at("from 'orders.csv' | extend x = coalesce(‸)").unwrap();
    assert_eq!(help.active_argument, 0);
    let sig = &help.signatures[help.active_signature];
    assert_eq!(sig.parameters.len(), 2);
    assert_eq!(sig.active_parameter, Some(0));
    let p0 = sig.parameters[0];
    let p1 = sig.parameters[1];
    assert_eq!(&sig.label[p0.0..p0.1], "value: T");
    assert_eq!(&sig.label[p1.0..p1.1], "value: T");
    assert!(p1.0 > p0.1);

    let help = signature_at("from 'orders.csv' | extend x = coalesce(orderId, ‸)").unwrap();
    assert_eq!(help.active_argument, 1);
    assert_eq!(
        help.signatures[help.active_signature].active_parameter,
        Some(1)
    );

    let help =
        signature_at("from 'orders.csv' | extend x = coalesce(orderId, customerId, ‸)").unwrap();
    assert_eq!(help.active_argument, 2);
    assert_eq!(
        help.signatures[help.active_signature].active_parameter,
        Some(1)
    );

    // `repeat.min > 0` wraparound and `trailing` (`case`, `strcat`).
    let help = signature_at("from 'orders.csv' | extend s = case(amount > 1, 'a', amount > 0, ‸)")
        .unwrap();
    assert_eq!(help.active_argument, 3);
    assert_eq!(
        help.signatures[help.active_signature].active_parameter,
        Some(1)
    );
    let help =
        signature_at("from 'orders.csv' | extend s = case(amount > 1, 'a', am‸ount > 0, 'b', 'c')")
            .unwrap();
    assert_eq!(help.active_argument, 2);
    assert_eq!(
        help.signatures[help.active_signature].active_parameter,
        Some(0)
    );
    let help =
        signature_at("from 'orders.csv' | extend s = case(amount > 1, 'a', amount > 0, 'b', '‸c')")
            .unwrap();
    assert_eq!(help.active_argument, 4);
    assert_eq!(
        help.signatures[help.active_signature].active_parameter,
        Some(2)
    );
    let help =
        signature_at("from 'orders.csv' | extend s = strcat(orderId, customerId, ‸)").unwrap();
    assert_eq!(help.active_argument, 2);
    assert_eq!(
        help.signatures[help.active_signature].active_parameter,
        Some(0)
    );

    // Multi-signature builtins: `sumif` and `percentile`.
    let help = signature_at("from 'orders.csv' | summarize sumif(‸)").unwrap();
    assert_eq!(help.signatures.len(), 2);
    assert_eq!(help.active_argument, 0);
    for sig in &help.signatures {
        assert_eq!(sig.active_parameter, Some(0));
    }

    let help = signature_at("from 'orders.csv' | summarize sumif(amount, ‸)").unwrap();
    assert_eq!(help.active_argument, 1);
    for sig in &help.signatures {
        assert_eq!(sig.active_parameter, Some(1));
        let p = sig.parameters[1];
        assert_eq!(&sig.label[p.0..p.1], "predicate: bool");
    }

    let help = signature_at("from 'orders.csv' | summarize sumif(amount, amount > 0, ‸)").unwrap();
    assert_eq!(help.active_argument, 2);
    for sig in &help.signatures {
        assert_eq!(sig.active_parameter, None);
    }

    let help = signature_at("from 'orders.csv' | summarize percentile(‸)").unwrap();
    assert_eq!(help.signatures.len(), 3);
    assert_eq!(help.active_argument, 0);
    for sig in &help.signatures {
        assert_eq!(sig.active_parameter, Some(0));
    }

    let help = signature_at("from 'orders.csv' | summarize percentile(amount, ‸)").unwrap();
    assert_eq!(help.active_argument, 1);
    for sig in &help.signatures {
        assert_eq!(sig.active_parameter, Some(1));
        let p = sig.parameters[1];
        assert_eq!(&sig.label[p.0..p.1], "percentage: number");
    }

    let help = signature_at("from 'orders.csv' | summarize percentile(amount, 95, ‸)").unwrap();
    assert_eq!(help.active_argument, 2);
    for sig in &help.signatures {
        assert_eq!(sig.active_parameter, None);
    }
}

#[test]
fn join_on_left_and_right_completion_and_hover() {
    struct JoinTables;
    impl SchemaProvider for JoinTables {
        fn schema_of(&self, source: &SourceRef) -> SchemaLookup {
            match source.path() {
                Some("orders.csv") => SchemaLookup::Ready(Arc::new(Schema::new(vec![
                    Field::new("orderId", DataType::Utf8, true),
                    Field::new("customerId", DataType::Utf8, true),
                    Field::new("amount", DataType::Float64, true),
                ]))),
                Some("customers.csv") => SchemaLookup::Ready(Arc::new(Schema::new(vec![
                    Field::new("id", DataType::Utf8, true),
                    Field::new("customerId", DataType::Int64, true),
                    Field::new("name", DataType::Utf8, true),
                    Field::new("full name", DataType::Utf8, true),
                ]))),
                _ => SchemaLookup::Failed(SourceError::NotFound { nearby: Vec::new() }),
            }
        }
    }

    let hover_join = |marked: &str| {
        let (src, offset) = split(marked);
        let analysis = analyze(&src, &JoinTables);
        hover(&src, &analysis, offset).map(|h| h.markdown)
    };

    // Completion after `$right.` offers only the right table's columns.
    let c = completions_with(
        "from 'orders.csv' | join 'customers.csv' on $left.customerId == $right.‸",
        &JoinTables,
    );
    assert_eq!(
        labels_of(&c, CompletionKind::Column),
        ["id", "customerId", "name", "['full name']"]
    );
    assert_eq!(c.items.len(), 4, "only columns after `$right.`");

    let c = completions_with(
        "from 'orders.csv' | join 'customers.csv' on $left.customerId == $right.na‸",
        &JoinTables,
    );
    assert_eq!(
        labels_of(&c, CompletionKind::Column),
        ["id", "customerId", "name", "['full name']"]
    );
    assert_eq!((c.start, c.end), (71, 73));

    // Completion and hover still resolve `$right.` when editing a multi-key join before `,`.
    let c = completions_with(
        "from 'orders.csv' | join 'customers.csv' on $left.customerId == $right.‸, $left.orderId == $right.id",
        &JoinTables,
    );
    assert_eq!(
        labels_of(&c, CompletionKind::Column),
        ["id", "customerId", "name", "['full name']"]
    );
    assert_eq!(
        hover_join(
            "from 'orders.csv' | join 'customers.csv' on $left.customerId == $right., $left.orderId == $right.i‸d"
        )
        .as_deref(),
        Some("```ds\nid: string\n```\ncolumn")
    );

    // Parenthesized pipeline and tabular `let` on the right side.
    let c = completions_with(
        "from 'orders.csv' | join (from 'customers.csv' | project cid = id, name) on $left.customerId == $right.‸",
        &JoinTables,
    );
    assert_eq!(labels_of(&c, CompletionKind::Column), ["cid", "name"]);

    let c = completions_with(
        "let c = from 'customers.csv' | project cid = id;\nfrom 'orders.csv' | join c on $left.customerId == $right.‸",
        &JoinTables,
    );
    assert_eq!(labels_of(&c, CompletionKind::Column), ["cid"]);

    // Completion after `$left.` offers only the left table's columns.
    let c = completions_with(
        "from 'orders.csv' | join 'customers.csv' on $left.‸",
        &JoinTables,
    );
    assert_eq!(
        labels_of(&c, CompletionKind::Column),
        ["orderId", "customerId", "amount"]
    );
    assert_eq!(c.items.len(), 3, "only columns after `$left.`");

    // Hover resolves `$right.` against the right table and `$left.` against the left table.
    assert_eq!(
        hover_join("from 'orders.csv' | join 'customers.csv' on $left.customerId == $right.na‸me")
            .as_deref(),
        Some("```ds\nname: string\n```\ncolumn")
    );
    assert_eq!(
        hover_join(
            "from 'orders.csv' | join 'customers.csv' on $left.customerId == $right.['full name'‸]"
        )
        .as_deref(),
        Some("```ds\n['full name']: string\n```\ncolumn")
    );
    assert_eq!(
        hover_join(
            "from 'orders.csv' | join 'customers.csv' on $left.customerId == $right.customer‸Id"
        )
        .as_deref(),
        Some("```ds\ncustomerId: long\n```\ncolumn")
    );
    assert_eq!(
        hover_join(
            "from 'orders.csv' | join 'customers.csv' on $left.customer‸Id == $right.customerId"
        )
        .as_deref(),
        Some("```ds\ncustomerId: string\n```\ncolumn")
    );
    // A left-only column or a scalar `let` must not resolve after `$right.`.
    assert_eq!(
        hover_join(
            "from 'orders.csv' | join 'customers.csv' on $left.customerId == $right.order‸Id"
        ),
        None
    );
    assert_eq!(
        hover_join(
            "let x = 1;\nfrom 'orders.csv' | join 'customers.csv' on $left.customerId == $right.x‸"
        ),
        None
    );
}

#[test]
fn extend_sees_columns_from_earlier_items() {
    // Earlier item is visible with its inferred type.
    let c = completions("from 'orders.csv' | extend total = amount * 2, big = ‸");
    assert_eq!(
        labels_of(&c, CompletionKind::Column),
        [&COLUMNS[..], &["total"]].concat()
    );
    let total = c.items.iter().find(|i| i.label == "total").unwrap();
    assert_eq!(total.detail.as_deref(), Some("real"));

    // Chained earlier items are all visible.
    let c =
        completions("from 'orders.csv' | extend total = amount * 2, big = total > 100, tag = ‸");
    assert_eq!(
        labels_of(&c, CompletionKind::Column),
        [&COLUMNS[..], &["total", "big"]].concat()
    );
    assert_eq!(
        c.items
            .iter()
            .find(|i| i.label == "big")
            .unwrap()
            .detail
            .as_deref(),
        Some("bool")
    );

    // Replacing an input column updates its type in place without duplicating it.
    let c = completions("from 'orders.csv' | extend amount = tostring(amount), x = ‸");
    assert_eq!(labels_of(&c, CompletionKind::Column), COLUMNS);
    assert_eq!(
        c.items
            .iter()
            .find(|i| i.label == "amount")
            .unwrap()
            .detail
            .as_deref(),
        Some("string")
    );

    // The item being edited does not see itself.
    let c = completions("from 'orders.csv' | extend total = to‸");
    assert_eq!(labels_of(&c, CompletionKind::Column), COLUMNS);

    // Incomplete expression inside the current item still sees prior items, not itself.
    let c = completions("from 'orders.csv' | extend total = amount * 2, big = 1 + ‸");
    assert_eq!(
        labels_of(&c, CompletionKind::Column),
        [&COLUMNS[..], &["total"]].concat()
    );

    // Syntactically broken `extend` before another stage still sees earlier items.
    let c = completions("from 'orders.csv' | extend total = amount * 2, big = ‸ | take 5");
    assert_eq!(
        labels_of(&c, CompletionKind::Column),
        [&COLUMNS[..], &["total"]].concat()
    );

    // Inside a tabular `let` and a subquery.
    let c = completions("let t = from 'orders.csv' | extend total = amount * 2, big = ‸;");
    assert_eq!(
        labels_of(&c, CompletionKind::Column),
        [&COLUMNS[..], &["total"]].concat()
    );
    let c = completions("from (from 'orders.csv' | extend total = amount * 2, big = ‸)");
    assert_eq!(
        labels_of(&c, CompletionKind::Column),
        [&COLUMNS[..], &["total"]].concat()
    );

    // Hover resolves an earlier `extend` column to its inferred type.
    let (src, offset) = split("from 'orders.csv' | extend total = amount * 2, big = to‸tal > 100");
    let analysis = analyze(&src, &Orders);
    assert_eq!(
        hover(&src, &analysis, offset)
            .map(|h| h.markdown)
            .as_deref(),
        Some("```ds\ntotal: real\n```\ncolumn")
    );
}

#[test]
fn goto_definition_references_and_rename_for_lets() {
    let text = "let cutoff = 100;\n\
                let big = from 'orders.csv' | where amount > cutoff;\n\
                from big | join big on customerId | where amount > cutoff | take 5;\n";
    let src = SourceFile::new("q.ds", text);
    let analysis = analyze(&src, &Orders);

    let cutoff_def = text.find("cutoff").unwrap();
    let cutoff_use_1 = text[cutoff_def + 6..].find("cutoff").unwrap() + cutoff_def + 6;
    let cutoff_use_2 = text.rfind("cutoff").unwrap();

    let big_def = text.find("big").unwrap();
    let big_use_1 = text.find("from big").unwrap() + 5;
    let big_use_2 = text.find("join big").unwrap() + 5;

    // Full analysis: both tabular (`big`) and row-expression (`cutoff`) `let`s resolve and rename.
    assert_eq!(
        goto_definition(&src, Some(&analysis), cutoff_use_2 + 2),
        Some(cutoff_def..cutoff_def + 6)
    );
    // Cursor right at the end of a `let` name before punctuation (`cutoff;`) still resolves.
    assert_eq!(
        goto_definition(&src, Some(&analysis), cutoff_use_1 + 6),
        Some(cutoff_def..cutoff_def + 6)
    );
    assert_eq!(
        prepare_rename(&src, Some(&analysis), cutoff_use_1 + 6),
        Ok(Some(RenameTarget {
            start: cutoff_use_1,
            end: cutoff_use_1 + 6,
            placeholder: "cutoff".into(),
        }))
    );
    assert_eq!(
        references(&src, Some(&analysis), cutoff_def + 1, true),
        vec![
            cutoff_def..cutoff_def + 6,
            cutoff_use_1..cutoff_use_1 + 6,
            cutoff_use_2..cutoff_use_2 + 6,
        ]
    );
    assert_eq!(
        references(&src, Some(&analysis), cutoff_use_1 + 1, false),
        vec![
            cutoff_use_1..cutoff_use_1 + 6,
            cutoff_use_2..cutoff_use_2 + 6,
        ]
    );

    // Tabular `let` (`big`, used only in no-column table positions) works identically with full
    // analysis and in syntax-only mode (`None`).
    for analysis_opt in [Some(&analysis), None] {
        assert_eq!(
            goto_definition(&src, analysis_opt, big_use_2 + 1),
            Some(big_def..big_def + 3)
        );
        assert_eq!(
            references(&src, analysis_opt, big_use_1, true),
            vec![
                big_def..big_def + 3,
                big_use_1..big_use_1 + 3,
                big_use_2..big_use_2 + 3,
            ]
        );
        assert_eq!(
            prepare_rename(&src, analysis_opt, big_use_1 + 1),
            Ok(Some(RenameTarget {
                start: big_use_1,
                end: big_use_1 + 3,
                placeholder: "big".into(),
            }))
        );
        let edits = rename(&src, analysis_opt, big_use_1 + 1, "large_orders").unwrap();
        assert_eq!(
            edits,
            vec![
                RenameEdit {
                    start: big_def,
                    end: big_def + 3,
                    replacement: "large_orders".into(),
                },
                RenameEdit {
                    start: big_use_1,
                    end: big_use_1 + 3,
                    replacement: "large_orders".into(),
                },
                RenameEdit {
                    start: big_use_2,
                    end: big_use_2 + 3,
                    replacement: "large_orders".into(),
                },
            ]
        );
    }

    // Columns, keywords and unknown names do not jump or rename.
    let amount_off = text.find("amount").unwrap();
    assert_eq!(goto_definition(&src, Some(&analysis), amount_off), None);
    assert_eq!(prepare_rename(&src, Some(&analysis), amount_off), Ok(None));
    assert_eq!(
        rename(&src, Some(&analysis), amount_off, "x"),
        Err(RenameError::NotRenamable)
    );
    // Collision with an existing `let` is rejected.
    assert_eq!(
        rename(&src, Some(&analysis), big_def, "cutoff"),
        Err(RenameError::AlreadyDefined("cutoff".into()))
    );
}

#[test]
fn unknown_schema_excludes_ambiguous_row_refs_and_refuses_rename() {
    let text = "let amount = 5;\n\
                from 'orders.csv' | where amount > 1 | take amount\n";
    let src = SourceFile::new("q.ds", text);
    let no_schema_analysis = analyze(&src, &crate::NoSchemas);

    let let_amount = text.find("amount").unwrap();
    let where_amount = text.find("where amount").unwrap() + 6;
    let take_amount = text.rfind("amount").unwrap();

    for (analysis_opt, expected_syntax_only) in [(None, true), (Some(&no_schema_analysis), false)] {
        // Row-expression site with unknown schema is excluded from goto, references, and readonly tokens.
        assert_eq!(goto_definition(&src, analysis_opt, where_amount), None);
        // No-column site (`take amount`) still resolves unambiguously.
        assert_eq!(
            goto_definition(&src, analysis_opt, take_amount),
            Some(let_amount..let_amount + 6)
        );
        assert_eq!(
            references(&src, analysis_opt, let_amount, false),
            vec![take_amount..take_amount + 6]
        );
        let tokens = semantic_tokens(&src, analysis_opt);
        let where_tok = tokens
            .iter()
            .find(|t| t.start == where_amount && t.end == where_amount + 6)
            .unwrap();
        assert!(
            !where_tok
                .modifiers
                .contains(SemanticTokenModifiers::READONLY)
        );
        let take_tok = tokens
            .iter()
            .find(|t| t.start == take_amount && t.end == take_amount + 6)
            .unwrap();
        assert!(
            take_tok
                .modifiers
                .contains(SemanticTokenModifiers::READONLY)
        );

        // Both `prepare_rename` and `rename` refuse from the definition, `take`, or `where` site.
        for offset in [let_amount, take_amount, where_amount] {
            assert_eq!(
                prepare_rename(&src, analysis_opt, offset),
                Err(RenameError::UnknownSchema {
                    syntax_only: expected_syntax_only
                })
            );
            assert_eq!(
                rename(&src, analysis_opt, offset, "threshold"),
                Err(RenameError::UnknownSchema {
                    syntax_only: expected_syntax_only
                })
            );
        }
    }
}

#[test]
fn rename_rejects_name_shadowed_by_in_scope_column() {
    let text = "let cutoff = 100;\n\
                from 'orders.csv' | where amount > cutoff\n";
    let src = SourceFile::new("q.ds", text);
    let analysis = analyze(&src, &Orders);
    let cutoff_def = text.find("cutoff").unwrap();

    // `amount` is an input column at line 2 -> rejected so query meaning cannot silently change.
    assert_eq!(
        rename(&src, Some(&analysis), cutoff_def, "amount"),
        Err(RenameError::ShadowedByColumn {
            name: "amount".into(),
            line: 2,
        })
    );
    // Non-colliding name still succeeds.
    assert!(rename(&src, Some(&analysis), cutoff_def, "threshold").is_ok());

    // Earlier `extend` item column also blocks the rename (D-040).
    let ext_text = "let cutoff = 100;\n\
                    from 'orders.csv' | extend total = amount * 2, big = total > cutoff\n";
    let ext_src = SourceFile::new("ext.ds", ext_text);
    let ext_analysis = analyze(&ext_src, &Orders);
    let ext_def = ext_text.find("cutoff").unwrap();
    assert_eq!(
        rename(&ext_src, Some(&ext_analysis), ext_def, "total"),
        Err(RenameError::ShadowedByColumn {
            name: "total".into(),
            line: 2,
        })
    );

    // No-column positions (`take`) are not shadowed by columns.
    let take_text = "let limit = 5;\nfrom 'orders.csv' | take limit\n";
    let take_src = SourceFile::new("take.ds", take_text);
    let take_analysis = analyze(&take_src, &Orders);
    assert!(rename(&take_src, Some(&take_analysis), 4, "amount").is_ok());
}

#[test]
fn let_resolution_respects_column_shadowing_and_non_let_positions() {
    // Column `amount` shadows `let amount` in row expressions when schemas are known (D-038),
    // but `let amount` is still resolved in `take` (constant count) and in syntax-only mode.
    let text = "let amount = 5;\n\
                let sum = 1;\n\
                from 'orders.csv'\n\
                | where amount > 0\n\
                | extend x = 1, amount = 2, y = amount\n\
                | summarize sum(amount) by customerId\n\
                | take amount\n";
    let src = SourceFile::new("q.ds", text);
    let analysis = analyze(&src, &Orders);

    let let_amount = text.find("amount").unwrap();
    let where_amount = text.find("where amount").unwrap() + 6;
    let extend_y_amount = text.find("y = amount").unwrap() + 4;
    let sum_call = text.find("sum(amount)").unwrap();
    let take_amount = text.rfind("amount").unwrap();

    // With full analysis: `where amount` and `extend y = amount` are columns, `take amount` is the `let`.
    assert_eq!(goto_definition(&src, Some(&analysis), where_amount), None);
    assert_eq!(
        goto_definition(&src, Some(&analysis), extend_y_amount),
        None
    );
    assert_eq!(
        goto_definition(&src, Some(&analysis), take_amount),
        Some(let_amount..let_amount + 6)
    );
    assert_eq!(
        references(&src, Some(&analysis), let_amount, false),
        vec![take_amount..take_amount + 6]
    );

    // Function callee `sum(...)` never resolves to `let sum`.
    assert_eq!(goto_definition(&src, Some(&analysis), sum_call), None);
    assert_eq!(goto_definition(&src, None, sum_call), None);

    // A `let` shadowed by an earlier `extend` item (not an input column): `total + 1` still
    // resolves to `let total`, while `y = total` resolves to the column made by the prior item.
    let ext_text = "let total = 1;\nfrom 'orders.csv' | extend total = total + 1, y = total\n";
    let ext_src = SourceFile::new("ext.ds", ext_text);
    let ext_analysis = analyze(&ext_src, &Orders);
    let total_def = ext_text.find("total").unwrap();
    let total_rhs_1 = ext_text.find("total + 1").unwrap();
    let total_rhs_2 = ext_text.find("y = total").unwrap() + 4;
    assert_eq!(
        goto_definition(&ext_src, Some(&ext_analysis), total_rhs_1),
        Some(total_def..total_def + 5)
    );
    assert_eq!(
        goto_definition(&ext_src, Some(&ext_analysis), total_rhs_2),
        None
    );
    assert_eq!(
        references(&ext_src, Some(&ext_analysis), total_def, false),
        vec![total_rhs_1..total_rhs_1 + 5]
    );

    // Non-`let` positions (`join ... on`, `$left.col`, `project-rename`, `render xcolumn`,
    // `distinct`, self/forward references, duplicate `let`) never resolve to `let`.
    let edge = "let a = a + b;\n\
                let b = 1;\n\
                let b = 2;\n\
                let title = 'Sales';\n\
                from 'orders.csv'\n\
                | join 'orders.csv' on $left.b == $right.b\n\
                | render barchart with (title = title, xcolumn = b, legend = hidden)\n";
    let edge_src = SourceFile::new("edge.ds", edge);
    let b_def = edge.find("let b = 1").unwrap() + 4;
    let b_dup = edge.find("let b = 2").unwrap() + 4;
    let self_a = edge.find("= a").unwrap() + 2;
    let fwd_b = edge.find("+ b").unwrap() + 2;
    let join_b = edge.find("$left.b").unwrap() + 6;
    let render_xcol_b = edge.find("xcolumn = b").unwrap() + 10;
    let title_def = edge.find("let title").unwrap() + 4;
    let title_val = edge.find("title = title").unwrap() + 8;

    assert_eq!(goto_definition(&edge_src, None, self_a), None);
    assert_eq!(goto_definition(&edge_src, None, fwd_b), None);
    assert_eq!(goto_definition(&edge_src, None, join_b), None);
    assert_eq!(goto_definition(&edge_src, None, render_xcol_b), None);
    assert_eq!(goto_definition(&edge_src, None, b_dup), None);
    assert_eq!(
        references(&edge_src, None, b_def, true),
        vec![b_def..b_def + 1]
    );
    // Render scalar property `title = title` DOES resolve to `let title`.
    assert_eq!(
        goto_definition(&edge_src, None, title_val),
        Some(title_def..title_def + 5)
    );
}

#[test]
fn rename_handles_quoted_identifiers_and_validation() {
    let text = "let ['my limit'] = 5;\nfrom 'orders.csv' | take ['my limit']\n";
    let src = SourceFile::new("q.ds", text);
    let def = text.find("['my limit']").unwrap();
    let use_off = text.rfind("['my limit']").unwrap();

    assert_eq!(
        prepare_rename(&src, None, use_off + 2),
        Ok(Some(RenameTarget {
            start: use_off,
            end: use_off + 12,
            placeholder: "my limit".into(),
        }))
    );
    // Plain identifier needs no quotes; spaces and reserved words are quoted automatically.
    let plain = rename(&src, None, def, "max_rows").unwrap();
    assert_eq!(plain[0].replacement, "max_rows");
    assert_eq!(plain[1].replacement, "max_rows");

    let spaced = rename(&src, None, def, "row limit").unwrap();
    assert_eq!(spaced[0].replacement, "['row limit']");

    let reserved = rename(&src, None, def, "and").unwrap();
    assert_eq!(reserved[0].replacement, "['and']");

    let already_quoted = rename(&src, None, def, "['row limit']").unwrap();
    assert_eq!(already_quoted[0].replacement, "['row limit']");

    let bracketed = rename(&src, None, def, "[total]").unwrap();
    assert_eq!(bracketed[0].replacement, "['[total]']");

    assert!(matches!(
        rename(&src, None, def, ""),
        Err(RenameError::InvalidName(_))
    ));
    assert!(matches!(
        rename(&src, None, def, "   "),
        Err(RenameError::InvalidName(_))
    ));
    assert!(matches!(
        rename(&src, None, def, "a\nb"),
        Err(RenameError::InvalidName(_))
    ));
    assert!(matches!(
        rename(&src, None, def, "['']"),
        Err(RenameError::InvalidName(_))
    ));
}

#[test]
fn document_symbols_for_lets_and_queries() {
    let text = "let cutoff = 100;\n\
                let big = from 'orders.csv' | where amount > cutoff;\n\
                from big | count;\n\
                from 'orders.csv' | take 5\n";
    let src = SourceFile::new("q.ds", text);
    let analysis = analyze(&src, &Orders);

    let full = document_symbols(&src, Some(&analysis));
    assert_eq!(full.len(), 4);
    assert_eq!(full[0].name, "cutoff");
    assert_eq!(full[0].kind, SymbolKind::Variable);
    assert_eq!(full[0].detail.as_deref(), Some("long"));
    assert_eq!(full[1].name, "big");
    assert_eq!(full[1].kind, SymbolKind::Table);
    assert_eq!(full[1].detail.as_deref(), Some("table (4 columns)"));
    assert_eq!(full[2].name, "query 1");
    assert_eq!(full[2].kind, SymbolKind::Query);
    assert_eq!(full[2].detail.as_deref(), Some("from big"));
    assert_eq!(full[3].name, "query 2");
    assert_eq!(full[3].kind, SymbolKind::Query);
    assert_eq!(full[3].detail.as_deref(), Some("from 'orders.csv'"));
    for s in &full {
        assert!(s.start <= s.selection_start && s.selection_end <= s.end);
    }

    // Syntax-only mode still reports every symbol with syntactic details.
    let syntax = document_symbols(&src, None);
    assert_eq!(syntax.len(), 4);
    assert_eq!(syntax[0].detail.as_deref(), Some("scalar"));
    assert_eq!(syntax[1].detail.as_deref(), Some("table"));
    assert_eq!(syntax[2].detail.as_deref(), Some("from big"));

    // Duplicate `let` of a different kind keeps its own syntactic fallback detail instead of
    // inheriting the first binding's HIR type.
    let dup = SourceFile::new("dup.ds", "let x = 1;\nlet x = from 'orders.csv';\n");
    let dup_analysis = analyze(&dup, &Orders);
    let dup_syms = document_symbols(&dup, Some(&dup_analysis));
    assert_eq!(dup_syms.len(), 2);
    assert_eq!(dup_syms[0].kind, SymbolKind::Variable);
    assert_eq!(dup_syms[0].detail.as_deref(), Some("long"));
    assert_eq!(dup_syms[1].kind, SymbolKind::Table);
    assert_eq!(dup_syms[1].detail.as_deref(), Some("table"));
}

#[test]
fn semantic_tokens_classify_contextual_keywords_and_syntax() {
    let text = "// comment\n\
                let limit = 10;\n\
                from 'orders.csv'\n\
                | where by > 1 and on == has and from != long(null)\n\
                | summarize n = count() by by\n\
                | sort by on asc nulls last\n\
                | join kind = inner 'orders.csv' on $left.kind == $right.on\n\
                | project-away by, on*\n\
                | render barchart with (kind = stacked, xcolumn = by)\n";
    let src = SourceFile::new("q.ds", text);
    let tokens = semantic_tokens(&src, None);

    // Strictly ascending and non-overlapping.
    for pair in tokens.windows(2) {
        assert!(
            pair[0].end <= pair[1].start,
            "overlapping or unsorted tokens: {:?} and {:?}",
            pair[0],
            pair[1]
        );
    }

    let tagged: Vec<(&str, SemanticTokenKind, SemanticTokenModifiers)> = tokens
        .iter()
        .map(|t| (&text[t.start..t.end], t.kind, t.modifiers))
        .collect();

    let find_at = |needle: &str, nth: usize| {
        tagged
            .iter()
            .filter(|(s, _, _)| *s == needle)
            .nth(nth)
            .copied()
            .unwrap_or_else(|| panic!("missing #{nth} `{needle}` in {tagged:?}"))
    };

    assert_eq!(
        find_at("// comment", 0),
        (
            "// comment",
            SemanticTokenKind::Comment,
            SemanticTokenModifiers::NONE
        )
    );
    assert_eq!(
        find_at("let", 0),
        (
            "let",
            SemanticTokenKind::Keyword,
            SemanticTokenModifiers::DECLARATION
        )
    );
    assert_eq!(
        find_at("limit", 0),
        (
            "limit",
            SemanticTokenKind::Variable,
            SemanticTokenModifiers::DECLARATION.union(SemanticTokenModifiers::READONLY)
        )
    );
    // `from` at pipeline head is a keyword; `from` inside `where` is a column variable.
    assert_eq!(
        find_at("from", 0),
        (
            "from",
            SemanticTokenKind::Keyword,
            SemanticTokenModifiers::NONE
        )
    );
    assert_eq!(
        find_at("from", 1),
        (
            "from",
            SemanticTokenKind::Variable,
            SemanticTokenModifiers::NONE
        )
    );
    // Pipeline operators get Keyword + DEFAULT_LIBRARY, including merged `project-away`.
    assert_eq!(
        find_at("where", 0),
        (
            "where",
            SemanticTokenKind::Keyword,
            SemanticTokenModifiers::DEFAULT_LIBRARY
        )
    );
    assert_eq!(
        find_at("project-away", 0),
        (
            "project-away",
            SemanticTokenKind::Keyword,
            SemanticTokenModifiers::DEFAULT_LIBRARY
        )
    );
    // Contextual keywords used as columns in `where` are Variable, not Keyword/Operator.
    assert_eq!(
        find_at("by", 0),
        (
            "by",
            SemanticTokenKind::Variable,
            SemanticTokenModifiers::NONE
        )
    );
    assert_eq!(
        find_at("on", 0),
        (
            "on",
            SemanticTokenKind::Variable,
            SemanticTokenModifiers::NONE
        )
    );
    assert_eq!(
        find_at("has", 0),
        (
            "has",
            SemanticTokenKind::Variable,
            SemanticTokenModifiers::NONE
        )
    );
    // `long(null)` has Type + EnumMember.
    assert_eq!(
        find_at("long", 0),
        (
            "long",
            SemanticTokenKind::Type,
            SemanticTokenModifiers::NONE
        )
    );
    assert_eq!(
        find_at("null", 0),
        (
            "null",
            SemanticTokenKind::EnumMember,
            SemanticTokenModifiers::NONE
        )
    );
    // `summarize n = count() by by`: `count` is Function+DEFAULT_LIBRARY, first `by` is Keyword, second `by` is Variable.
    assert_eq!(
        find_at("count", 0),
        (
            "count",
            SemanticTokenKind::Function,
            SemanticTokenModifiers::DEFAULT_LIBRARY
        )
    );
    assert_eq!(
        find_at("by", 1),
        (
            "by",
            SemanticTokenKind::Keyword,
            SemanticTokenModifiers::NONE
        )
    );
    assert_eq!(
        find_at("by", 2),
        (
            "by",
            SemanticTokenKind::Variable,
            SemanticTokenModifiers::NONE
        )
    );
    // `join kind = inner ... on $left.kind == $right.on`
    assert_eq!(
        find_at("kind", 0),
        (
            "kind",
            SemanticTokenKind::Keyword,
            SemanticTokenModifiers::NONE
        )
    );
    assert_eq!(
        find_at("inner", 0),
        (
            "inner",
            SemanticTokenKind::EnumMember,
            SemanticTokenModifiers::NONE
        )
    );
    assert_eq!(
        find_at("$left", 0),
        (
            "$left",
            SemanticTokenKind::Variable,
            SemanticTokenModifiers::DEFAULT_LIBRARY
        )
    );
    assert_eq!(
        find_at("kind", 1),
        (
            "kind",
            SemanticTokenKind::Variable,
            SemanticTokenModifiers::NONE
        )
    );
    // `render barchart with (kind = stacked, xcolumn = by)`
    assert_eq!(
        find_at("barchart", 0),
        (
            "barchart",
            SemanticTokenKind::EnumMember,
            SemanticTokenModifiers::NONE
        )
    );
    assert_eq!(
        find_at("kind", 2),
        (
            "kind",
            SemanticTokenKind::Property,
            SemanticTokenModifiers::NONE
        )
    );
    assert_eq!(
        find_at("stacked", 0),
        (
            "stacked",
            SemanticTokenKind::EnumMember,
            SemanticTokenModifiers::NONE
        )
    );
    assert_eq!(
        find_at("xcolumn", 0),
        (
            "xcolumn",
            SemanticTokenKind::Property,
            SemanticTokenModifiers::NONE
        )
    );
    assert_eq!(
        find_at("by", 5),
        (
            "by",
            SemanticTokenKind::Variable,
            SemanticTokenModifiers::NONE
        )
    );

    // Built-in readers (`read_csv`) get Function + DEFAULT_LIBRARY, and reader options get Property.
    let reader_src = SourceFile::new("r.ds", "from read_csv('orders.csv', header = true)");
    let reader_tokens = semantic_tokens(&reader_src, None);
    let reader_tagged: Vec<(&str, SemanticTokenKind, SemanticTokenModifiers)> = reader_tokens
        .iter()
        .map(|t| (&reader_src.text()[t.start..t.end], t.kind, t.modifiers))
        .collect();
    assert!(reader_tagged.contains(&(
        "read_csv",
        SemanticTokenKind::Function,
        SemanticTokenModifiers::DEFAULT_LIBRARY,
    )));
    assert!(reader_tagged.contains(&(
        "header",
        SemanticTokenKind::Property,
        SemanticTokenModifiers::NONE,
    )));
}
