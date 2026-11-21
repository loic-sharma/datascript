//! Unit tests for the binder that golden cases cannot express: rendering against the documented
//! example, stage schemas, pending sources, and the shape of the HIR.

use std::collections::HashMap;
use std::sync::Arc;

use arrow_schema::{DataType, Field, Schema, SchemaRef, TimeUnit};
use datascript_diagnostics::{FileId, Renderer, SourceFile, SourceMap, codes};

use crate::hir::{LetValue, Statement};
use crate::{
    Capabilities, NoSchemas, SchemaLookup, SchemaProvider, SourceError, SourceRef, Type, check,
};

/// A provider that knows `orders.csv`.
struct Orders;

fn orders_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("orderId", DataType::Utf8, true),
        Field::new("customerId", DataType::Utf8, true),
        Field::new("amount", DataType::Float64, true),
        Field::new(
            "timestamp",
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
            true,
        ),
    ]))
}

impl SchemaProvider for Orders {
    fn schema_of(&self, source: &SourceRef) -> SchemaLookup {
        match source {
            SourceRef::Path(path) if path == "orders.csv" => SchemaLookup::Ready(orders_schema()),
            _ => SchemaLookup::Failed(SourceError::NotFound { nearby: Vec::new() }),
        }
    }
}

/// The DS1001 example at the top of DIAGNOSTICS.md, rendered from a real check.
#[test]
fn ds1001_renders_as_documented() {
    let text =
        "from 'orders.csv'\n| where amount > 0\n| summarize revenue = sum(amout) by customerId\n";
    let mut sources = SourceMap::new();
    let file = sources.add("top_customers.ds", text);
    let analysis = check(
        &SourceFile::new("top_customers.ds", text),
        file,
        &Orders,
        &Capabilities::full(),
    );
    let d = analysis
        .diagnostics
        .iter()
        .find(|d| d.code == codes::DS1001)
        .expect("a DS1001");
    let rendered = Renderer::terminal(false).render(d, &sources);

    let doc = include_str!("../../../docs/DIAGNOSTICS.md").replace('\r', "");
    let documented = doc
        .split("```")
        .find(|block| block.contains("error[DS1001]"))
        .expect("DIAGNOSTICS.md has the DS1001 example")
        .trim_start_matches('\n');
    assert_eq!(rendered, documented);
}

#[test]
fn stage_schemas_follow_each_stage() {
    let text = "from 'orders.csv' | where amount > 1 | project orderId, amount | take 2";
    let analysis = check(
        &SourceFile::new("q", text),
        FileId(0),
        &Orders,
        &Capabilities::full(),
    );
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let shapes: Vec<(&str, usize)> = analysis
        .stage_schemas
        .iter()
        .map(|s| (&text[s.span.range()], s.schema.fields().len()))
        .collect();
    assert_eq!(
        shapes,
        [
            ("'orders.csv'", 4),
            ("where amount > 1", 4),
            ("project orderId, amount", 2),
            ("take 2", 2),
        ]
    );
}

#[test]
fn pending_sources_are_silent_then_bind() {
    let text = "let big = from 'orders.csv' | where amount > 1;\nfrom big | where amout > 2";
    let src = SourceFile::new("q", text);
    let first = check(&src, FileId(0), &NoSchemas, &Capabilities::full());
    assert!(first.diagnostics.is_empty(), "{:?}", first.diagnostics);
    assert_eq!(
        first.pending_sources,
        [SourceRef::Path("orders.csv".into())]
    );
    let second = check(&src, FileId(0), &Orders, &Capabilities::full());
    assert!(second.pending_sources.is_empty());
    let codes: Vec<_> = second.diagnostics.iter().map(|d| d.code).collect();
    assert_eq!(codes, [codes::DS1001]);
}

#[test]
fn lets_are_typed_and_inlined() {
    let text =
        "let n = 1 + 2;\nlet r = n * 1.5;\nlet t = from 'orders.csv' | where amount > r;\nfrom t";
    let analysis = check(
        &SourceFile::new("q", text),
        FileId(0),
        &Orders,
        &Capabilities::full(),
    );
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let types: HashMap<&str, Option<Type>> = analysis
        .hir
        .statements
        .iter()
        .filter_map(|s| match s {
            Statement::Let(binding) => Some((
                binding.name.as_str(),
                match &binding.value {
                    LetValue::Scalar(e) => Some(e.ty),
                    _ => None,
                },
            )),
            Statement::Query(_) => None,
        })
        .collect();
    assert_eq!(types["n"], Some(Type::Long));
    assert_eq!(types["r"], Some(Type::Real));
    assert_eq!(types["t"], None);
    let program = crate::program_of(analysis, FileId(0)).expect("plans");
    assert_eq!(
        program.explain(),
        "Filter: amount > (cast(1 + 2, real) * 1.5)\n  Scan: 'orders.csv' (orderId: string, customerId: string, amount: real, timestamp: datetime)\n"
    );
}

/// A cache that knows the sources it was given, and answers `Pending` for the rest, like the
/// session's (D-022).
struct Cache(HashMap<SourceRef, SchemaRef>);

impl SchemaProvider for Cache {
    fn schema_of(&self, source: &SourceRef) -> SchemaLookup {
        match self.0.get(source) {
            Some(schema) => SchemaLookup::Ready(schema.clone()),
            None => SchemaLookup::Pending,
        }
    }
}

/// Every kind of source is found on the first pass, silently, and bound on the second.
#[test]
fn two_passes_bind_every_kind_of_source() {
    let text = "let p = 'orders.csv';\n\
                let alias = hosted;\n\
                from p | where amout > 1;\n\
                from read_csv(p, delimiter = ';') | take 1;\n\
                from hosted | union 'logs/*.csv';\n\
                from missing | take 1";
    let src = SourceFile::new("q", text);
    let first = check(
        &src,
        FileId(0),
        &Cache(HashMap::new()),
        &Capabilities::full(),
    );
    assert!(first.diagnostics.is_empty(), "{:?}", first.diagnostics);
    let reader = SourceRef::Reader(crate::ReaderCall {
        function: "read_csv".into(),
        path: "orders.csv".into(),
        options: vec![("delimiter".into(), crate::OptionValue::String(";".into()))],
    });
    let mut expected = vec![
        SourceRef::Path("orders.csv".into()),
        SourceRef::Glob("logs/*.csv".into()),
        reader.clone(),
        SourceRef::Table("hosted".into()),
        SourceRef::Table("missing".into()),
    ];
    expected.sort();
    assert_eq!(first.pending_sources, expected);

    // The session fetches them; `missing` does not exist.
    struct Fetched(Cache);
    impl SchemaProvider for Fetched {
        fn schema_of(&self, source: &SourceRef) -> SchemaLookup {
            match self.0.schema_of(source) {
                SchemaLookup::Pending => SchemaLookup::Failed(SourceError::NotFound {
                    nearby: vec!["hosted".into()],
                }),
                found => found,
            }
        }
    }
    let cache = Cache(
        [
            SourceRef::Path("orders.csv".into()),
            SourceRef::Glob("logs/*.csv".into()),
            reader,
            SourceRef::Table("hosted".into()),
        ]
        .into_iter()
        .map(|s| (s, orders_schema()))
        .collect(),
    );
    let second = check(&src, FileId(0), &Fetched(cache), &Capabilities::full());
    assert!(second.pending_sources.is_empty());
    let found: Vec<_> = second
        .diagnostics
        .iter()
        .map(|d| (d.code, d.message.clone()))
        .collect();
    assert_eq!(
        found,
        [
            (
                codes::DS1006,
                "`hosted` is a table, not a value".to_string()
            ),
            (codes::DS1001, "unknown column `amout`".to_string()),
            (codes::DS1002, "unknown table `missing`".to_string()),
        ]
    );
}

/// A column with no DataScript type can be projected, but not used (DS2008).
#[test]
fn projection_only_columns() {
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
    let text = "from 'x.csv' | project id, tags | project-rename t = tags | where t > 1";
    let analysis = check(
        &SourceFile::new("q", text),
        FileId(0),
        &Nested,
        &Capabilities::full(),
    );
    let ds2008: Vec<_> = analysis
        .diagnostics
        .iter()
        .map(|d| (d.code, d.message.as_str()))
        .collect();
    assert_eq!(
        ds2008,
        [(
            codes::DS2008,
            "column `t` has a type DataScript cannot use yet"
        )]
    );
}

/// The engine's deviations reach the analysis without a diagnostic.
#[test]
fn deviations_are_carried_through() {
    use crate::capabilities::Deviation;
    let caps = Capabilities {
        deviations: [Deviation::IntegerOverflowUnchecked].into(),
        ..Capabilities::full()
    };
    let analysis = check(
        &SourceFile::new("q", "from 'orders.csv' | take 1"),
        FileId(0),
        &Orders,
        &caps,
    );
    assert!(analysis.diagnostics.is_empty());
    assert_eq!(analysis.deviations, [Deviation::IntegerOverflowUnchecked]);
}
