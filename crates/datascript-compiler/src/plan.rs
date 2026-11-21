//! Planning: lowers the typed [`Hir`](crate::hir::Hir) to the logical plan IR.
//!
//! M1-06 grows this into the full IR builder (desugaring, the other plan nodes). For now it
//! handles `.csv` scans, tabular `let`s and parenthesized pipelines (inlined), `where`, the
//! `project` family, `extend` (as projects) and `take`; other operators give `DS9001` "cannot run
//! yet". `let` references in expressions are replaced by their values.

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
    let mut columns_before: Vec<Column> = match &pipeline.source {
        hir::Source::Scan { columns, .. } => columns.clone(),
        hir::Source::Let { pipeline, .. } | hir::Source::Subquery { pipeline, .. } => {
            pipeline.columns.clone().unwrap_or_default()
        }
        hir::Source::Error { .. } => Vec::new(),
    };
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
        if let hir::Op::Extend(items) = &stage.op {
            // One `Project` per item: an item can read the columns made by the ones before it.
            for item in items {
                let mut after = columns_before.clone();
                let column = Column {
                    name: item.name.clone(),
                    ty: item.expr.ty,
                    read_from: None,
                };
                match after.iter_mut().find(|c| c.name == item.name) {
                    Some(existing) => *existing = column,
                    None => after.push(column),
                }
                let project = after
                    .iter()
                    .map(|c| {
                        Ok(ir::ProjectItem {
                            name: c.name.clone(),
                            expr: if c.name == item.name {
                                expr(&item.expr)?
                            } else {
                                ir::Expr {
                                    kind: ir::ExprKind::Column(c.name.clone()),
                                    ty: c.ty,
                                    span: stage.span,
                                }
                            },
                        })
                    })
                    .collect::<Result<_, Diagnostic>>()?;
                plan = Plan {
                    node: PlanNode::Project {
                        input: Box::new(plan),
                        columns: project,
                    },
                    schema: schema_of(&after),
                    span: stage.span,
                };
                columns_before = after;
            }
            columns_before = columns.clone();
            continue;
        }
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
            hir::Op::Extend(_) => unreachable!("planned above"),
            hir::Op::Summarize { .. }
            | hir::Op::Sort(_)
            | hir::Op::Top { .. }
            | hir::Op::Count
            | hir::Op::Distinct(_)
            | hir::Op::Join(_)
            | hir::Op::Union(_)
            | hir::Op::GetSchema => return Err(cannot_run_yet(stage)),
            hir::Op::Unsupported { .. } | hir::Op::Error => {
                return Err(internal(stage.span, "an operator with an error"));
            }
        };
        columns_before = columns.clone();
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

/// `DS9001` for an operator the binder checks but the planner cannot plan yet (M1-06).
fn cannot_run_yet(stage: &hir::Stage) -> Diagnostic {
    let name = match &stage.op {
        hir::Op::Summarize { .. } => "summarize",
        hir::Op::Sort(_) => "sort",
        hir::Op::Top { .. } => "top",
        hir::Op::Count => "count",
        hir::Op::Distinct(_) => "distinct",
        hir::Op::Join(_) => "join",
        hir::Op::Union(_) => "union",
        hir::Op::GetSchema => "getschema",
        _ => "this operator",
    };
    Diagnostic::error(DS9001, format!("`{name}` cannot run yet"))
        .primary(stage.span, "this version can check it, but not run it")
        .note(
            "this version can run `where`, `project`, `project-away`, `project-rename`, \
             `extend` and `take`",
        )
}

fn internal(span: datascript_diagnostics::Span, what: &str) -> Diagnostic {
    Diagnostic::error(
        DS9001,
        format!("internal error: the planner was given {what}"),
    )
    .primary(span, "while planning this")
    .note("this is a bug in DataScript; please report it")
}
