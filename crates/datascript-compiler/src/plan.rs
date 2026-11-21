//! Planning: lowers the typed [`Hir`](crate::hir::Hir) to the logical plan IR.
//!
//! M1-06 grows this into the full IR builder (desugaring, the other plan nodes). For now it
//! handles what the binder can produce without errors: `.csv` scans, tabular `let`s and
//! parenthesized pipelines (inlined), `where`, `project` and `take`. `let` references in
//! expressions are replaced by their values.

use std::sync::Arc;

use arrow_schema::{Schema, SchemaRef};
use datascript_diagnostics::Diagnostic;
use datascript_diagnostics::codes::DS9001;

use crate::arrow::to_arrow;
use crate::hir::{self, Column};
use crate::ir::{self, Plan, PlanNode};

/// The plan of one query statement.
pub(crate) fn plan_query(query: &hir::Query) -> Result<ir::QueryPlan, Diagnostic> {
    let root = plan_pipeline(&query.pipeline)?;
    Ok(ir::QueryPlan {
        output_schema: root.schema.clone(),
        root,
    })
}

fn plan_pipeline(pipeline: &hir::Pipeline) -> Result<Plan, Diagnostic> {
    let mut plan = match &pipeline.source {
        hir::Source::Scan {
            source,
            span,
            columns,
        } => Plan {
            node: PlanNode::Scan {
                source: source.clone(),
            },
            schema: schema_of(columns),
            span: *span,
        },
        hir::Source::Let { pipeline, .. } | hir::Source::Subquery { pipeline, .. } => {
            plan_pipeline(pipeline)?
        }
        hir::Source::Error { span } => return Err(internal(*span, "a source with an error")),
    };
    for stage in &pipeline.stages {
        let columns = stage
            .columns
            .as_ref()
            .ok_or_else(|| internal(stage.span, "a stage with unknown columns"))?;
        let input = Box::new(plan);
        let node = match &stage.op {
            hir::Op::Where(predicate) => PlanNode::Filter {
                input,
                predicate: expr(predicate)?,
            },
            hir::Op::Project(items) => PlanNode::Project {
                input,
                columns: items
                    .iter()
                    .map(|item| {
                        Ok(ir::ProjectItem {
                            name: item.name.clone(),
                            expr: expr(&item.expr)?,
                        })
                    })
                    .collect::<Result<_, Diagnostic>>()?,
            },
            hir::Op::Take(count) => PlanNode::Limit {
                input,
                count: *count,
            },
            hir::Op::Unsupported { .. } | hir::Op::Error => {
                return Err(internal(stage.span, "an operator with an error"));
            }
        };
        plan = Plan {
            node,
            schema: schema_of(columns),
            span: stage.span,
        };
    }
    Ok(plan)
}

fn expr(e: &hir::Expr) -> Result<ir::Expr, Diagnostic> {
    let boxed = |e: &hir::Expr| expr(e).map(Box::new);
    let list = |es: &[hir::Expr]| es.iter().map(expr).collect::<Result<Vec<_>, _>>();
    let kind = match &e.kind {
        // A `let` is inlined; its value keeps its own spans.
        hir::ExprKind::Let { value, .. } => return expr(value),
        hir::ExprKind::Column(name) => ir::ExprKind::Column(name.clone()),
        hir::ExprKind::Literal(value) => ir::ExprKind::Literal(value.clone()),
        hir::ExprKind::Unary { op, operand } => ir::ExprKind::Unary {
            op: *op,
            operand: boxed(operand)?,
        },
        hir::ExprKind::Binary { op, left, right } => ir::ExprKind::Binary {
            op: *op,
            left: boxed(left)?,
            right: boxed(right)?,
        },
        hir::ExprKind::Call { function, args } => ir::ExprKind::Call {
            function: function.to_string(),
            args: list(args)?,
        },
        hir::ExprKind::In {
            expr: lhs,
            list: items,
            negated,
            case_insensitive,
        } => ir::ExprKind::In {
            expr: boxed(lhs)?,
            list: list(items)?,
            negated: *negated,
            case_insensitive: *case_insensitive,
        },
        hir::ExprKind::Between {
            expr: lhs,
            low,
            high,
            negated,
        } => ir::ExprKind::Between {
            expr: boxed(lhs)?,
            low: boxed(low)?,
            high: boxed(high)?,
            negated: *negated,
        },
        hir::ExprKind::Cast(inner) => {
            let inner = expr(inner)?;
            // An inlined `let` can put a literal under a cast; fold it as the binder does.
            match inner.kind {
                ir::ExprKind::Literal(ir::Value::Long(n)) if e.ty == crate::Type::Real => {
                    ir::ExprKind::Literal(ir::Value::Real(n as f64))
                }
                ir::ExprKind::Literal(ir::Value::Null) => ir::ExprKind::Literal(ir::Value::Null),
                _ => ir::ExprKind::Cast(Box::new(inner)),
            }
        }
        hir::ExprKind::Error => return Err(internal(e.span, "an expression with an error")),
    };
    if e.ty.is_error() || e.ty.is_null() {
        return Err(internal(e.span, "an expression without a type"));
    }
    Ok(ir::Expr {
        kind,
        ty: e.ty,
        span: e.span,
    })
}

fn schema_of(columns: &[Column]) -> SchemaRef {
    Arc::new(Schema::new(
        columns
            .iter()
            .map(|c| to_arrow(&c.name, c.ty))
            .collect::<Vec<_>>(),
    ))
}

fn internal(span: datascript_diagnostics::Span, what: &str) -> Diagnostic {
    Diagnostic::error(
        DS9001,
        format!("internal error: the planner was given {what}"),
    )
    .primary(span, "while planning this")
    .note("this is a bug in DataScript; please report it")
}
