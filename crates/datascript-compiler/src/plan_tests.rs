//! Unit tests for planning that golden cases cannot express: serde, Arrow-only columns, spans.

use std::sync::Arc;

use arrow_schema::{DataType, Field, Schema};
use datascript_diagnostics::{FileId, SourceFile};

use crate::ir::{Expr, ExprKind, Plan, PlanNode, Program};
use crate::{Capabilities, SchemaLookup, SchemaProvider, SourceRef, compile};

/// Every source has `id: long, tags: list<long>, name: string`.
struct Nested;

impl SchemaProvider for Nested {
    fn schema_of(&self, _: &SourceRef) -> SchemaLookup {
        let item = Arc::new(Field::new("item", DataType::Int64, true));
        SchemaLookup::Ready(Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, true),
            Field::new("tags", DataType::List(item), true),
            Field::new("name", DataType::Utf8, true),
        ])))
    }
}

fn plan(text: &str) -> Program {
    compile(
        &SourceFile::new("q", text),
        FileId(0),
        &Nested,
        &Capabilities::full(),
    )
    .unwrap_or_else(|d| panic!("{d:?}"))
}

const README_LIKE: &str = "let top = from 'a.csv' | summarize n = count(), avg = sum(id) / count() by name | top 3 by n desc;\n\
     from 'a.csv' | where id > 1 | join kind = fullouter top on name | union 'b.csv' \
     | sort by id asc | distinct id, name | take 5;\n\
     from 'a.csv' | getschema";

#[test]
fn the_ir_round_trips_through_serde() {
    let program = plan(README_LIKE);
    let json = serde_json::to_string(&program).expect("serializes");
    let back: Program = serde_json::from_str(&json).expect("deserializes");
    assert_eq!(back, program);
    assert_eq!(back.explain(), program.explain());
}

/// A column with no DataScript type keeps its Arrow type through scans and projections.
#[test]
fn projection_only_columns_keep_their_arrow_type() {
    let program = plan("from 'x.parquet' | project tags, id | project-rename t = tags | take 1");
    let query = &program.queries[0];
    let list = DataType::List(Arc::new(Field::new("item", DataType::Int64, true)));
    assert_eq!(query.output_schema.field(0).name(), "t");
    assert_eq!(query.output_schema.field(0).data_type(), &list);
    assert_eq!(
        program.explain(),
        "Limit: 1\n\
         \x20 Project: t = tags, id\n\
         \x20   Project: tags, id\n\
         \x20     Scan: 'x.parquet' (id: long, tags: arrow List(Int64), name: string)\n"
    );
    // `getschema` names the Arrow type.
    let program = plan("from 'x.parquet' | getschema");
    let PlanNode::Values { rows } = &program.queries[0].root.node else {
        panic!("not a Values node");
    };
    assert_eq!(rows[1][2].to_string(), "\"List(Int64)\"");
}

/// Every node and expression points into the query text, so runtime errors can be traced back.
#[test]
fn every_node_and_expression_has_a_span_in_the_source() {
    let len = README_LIKE.len() as u32;
    fn check_expr(e: &Expr, len: u32) {
        assert!(e.span.end <= len && !e.span.is_empty(), "{e}: {:?}", e.span);
        match &e.kind {
            ExprKind::Binary { left, right, .. } => {
                check_expr(left, len);
                check_expr(right, len);
            }
            ExprKind::Unary { operand, .. } | ExprKind::Cast(operand) => check_expr(operand, len),
            ExprKind::Call { args, .. } => args.iter().for_each(|a| check_expr(a, len)),
            _ => {}
        }
    }
    fn check_plan(p: &Plan, len: u32) {
        assert!(p.span.end <= len && !p.span.is_empty(), "{:?}", p.node);
        match &p.node {
            PlanNode::Filter { predicate, .. } => check_expr(predicate, len),
            PlanNode::Project { columns, .. } => {
                columns.iter().for_each(|c| check_expr(&c.expr, len));
            }
            PlanNode::Aggregate {
                group_by,
                aggregates,
                ..
            } => group_by
                .iter()
                .chain(aggregates)
                .for_each(|c| check_expr(&c.expr, len)),
            PlanNode::Sort { keys, .. } | PlanNode::TopN { keys, .. } => {
                keys.iter().for_each(|k| check_expr(&k.expr, len));
            }
            PlanNode::Join { on, .. } => on.iter().for_each(|c| {
                check_expr(&c.left, len);
                check_expr(&c.right, len);
            }),
            _ => {}
        }
        for input in p.inputs() {
            check_plan(input, len);
        }
    }
    let program = plan(README_LIKE);
    assert_eq!(program.queries.len(), 2);
    for query in &program.queries {
        check_plan(&query.root, len);
        assert_eq!(query.output_schema, query.root.schema);
        assert!(query.render.is_none());
    }
}
