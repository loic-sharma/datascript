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
        let SourceRef::Path(path) = source;
        match path.as_str() {
            "orders.csv" => SchemaLookup::Ready(orders_schema()),
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
