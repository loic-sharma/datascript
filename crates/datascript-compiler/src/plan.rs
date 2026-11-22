//! Planning: lowers the typed [`Hir`](crate::hir::Hir) to the logical plan IR.
//!
//! The binder has already computed every stage's output columns; the planner turns each stage
//! into IR nodes whose schemas match them, desugaring as it goes:
//!
//! - `project`, `project-away`, `project-rename` → `Project`; `extend` → one `Project` per item
//!   (an item can read the columns made by the ones before it, D-040).
//! - `summarize` → `Aggregate`, plus a `Project` when an item computes over aggregates
//!   (`sum(a) / count()`); `count` → `Aggregate` with no keys.
//! - `sort` → `Sort`, `top` → `TopN`, `take` → `Limit`, `distinct` → `Distinct` (after a
//!   `Project` when it keeps only some columns), `getschema` → `Values`.
//! - `join` → `Join`, with a `Project` on the right that keeps output names unique and one after
//!   the join that merges `on key` columns (D-019).
//! - `union` → `Union`, each input projected to the output columns (missing ones are typed
//!   nulls, widened ones are cast).
//! - Tabular `let`s and parenthesized pipelines are inlined; scalar `let`s are replaced by their
//!   values.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use arrow_schema::{Field, Schema, SchemaRef};
use datascript_diagnostics::codes::DS9001;
use datascript_diagnostics::{Diagnostic, Span};

use crate::Type;
use crate::arrow::to_arrow;
use crate::binder::unique_name;
use crate::catalog::{FunctionCatalog, FunctionKind};
use crate::hir::{self, Column};
use crate::ir::{self, Plan, PlanNode, ProjectItem};

/// The plan of one query statement.
pub(crate) fn plan_query(query: &hir::Query) -> Result<ir::QueryPlan, Diagnostic> {
    let root = plan_pipeline(&query.pipeline)?.plan;
    Ok(ir::QueryPlan {
        output_schema: root.schema.clone(),
        root,
        render: None,
        span: query.span,
    })
}

/// A plan with the HIR columns its schema was built from.
struct Planned {
    plan: Plan,
    columns: Vec<Column>,
}

impl Planned {
    fn new(node: PlanNode, columns: Vec<Column>, span: Span) -> Planned {
        Planned {
            plan: Plan {
                node,
                schema: schema_of(&columns),
                span,
            },
            columns,
        }
    }
}

fn plan_pipeline(pipeline: &hir::Pipeline) -> Result<Planned, Diagnostic> {
    let mut planned = plan_source(&pipeline.source)?;
    for stage in &pipeline.stages {
        planned = plan_stage(planned, stage)?;
    }
    Ok(planned)
}

fn plan_source(source: &hir::Source) -> Result<Planned, Diagnostic> {
    match source {
        hir::Source::Scan {
            source,
            span,
            columns,
        } => Ok(Planned::new(
            PlanNode::Scan {
                source: source.clone(),
            },
            columns.clone(),
            *span,
        )),
        hir::Source::Let { pipeline, .. } | hir::Source::Subquery { pipeline, .. } => {
            plan_pipeline(pipeline)
        }
        hir::Source::Error { span } => Err(internal(*span, "a source with an error")),
    }
}

fn plan_stage(input: Planned, stage: &hir::Stage) -> Result<Planned, Diagnostic> {
    let span = stage.span;
    let out = stage
        .columns
        .clone()
        .ok_or_else(|| internal(span, "a stage with unknown columns"))?;
    let boxed = |p: Planned| Box::new(p.plan);
    Ok(match &stage.op {
        hir::Op::Where(predicate) => Planned::new(
            PlanNode::Filter {
                predicate: expr(predicate)?,
                input: boxed(input),
            },
            out,
            span,
        ),
        hir::Op::Project(items) => {
            let columns = items
                .iter()
                .map(|item| project_item(item, &input.columns))
                .collect::<Result<_, _>>()?;
            Planned::new(
                PlanNode::Project {
                    input: boxed(input),
                    columns,
                },
                out,
                span,
            )
        }
        hir::Op::Extend(items) => plan_extend(input, items, span)?,
        hir::Op::Take(count) => Planned::new(
            PlanNode::Limit {
                input: boxed(input),
                count: *count,
            },
            out,
            span,
        ),
        hir::Op::Summarize { keys, aggregates } => {
            plan_summarize(input, keys, aggregates, out, span)?
        }
        hir::Op::Count => {
            let count = ir::Expr {
                kind: ir::ExprKind::Call {
                    function: "count".to_string(),
                    args: Vec::new(),
                },
                ty: Type::Long,
                span,
            };
            let name = out
                .first()
                .map_or_else(|| "Count".to_string(), |c| c.name.clone());
            Planned::new(
                PlanNode::Aggregate {
                    input: boxed(input),
                    group_by: Vec::new(),
                    aggregates: vec![ProjectItem { name, expr: count }],
                },
                out,
                span,
            )
        }
        hir::Op::Sort(keys) => Planned::new(
            PlanNode::Sort {
                input: boxed(input),
                keys: keys.iter().map(sort_key).collect::<Result<_, _>>()?,
            },
            out,
            span,
        ),
        hir::Op::Top { count, key } => Planned::new(
            PlanNode::TopN {
                input: boxed(input),
                count: *count,
                keys: vec![sort_key(key)?],
            },
            out,
            span,
        ),
        hir::Op::Distinct(names) => {
            let all: Vec<&str> = input.columns.iter().map(|c| c.name.as_str()).collect();
            let input = if all == names.iter().map(String::as_str).collect::<Vec<_>>() {
                input
            } else {
                let columns = out.iter().map(|c| pass_through(c, span)).collect();
                Planned::new(
                    PlanNode::Project {
                        input: boxed(input),
                        columns,
                    },
                    out.clone(),
                    span,
                )
            };
            Planned::new(
                PlanNode::Distinct {
                    input: boxed(input),
                },
                out,
                span,
            )
        }
        hir::Op::Join(join) => plan_join(input, join, out, span)?,
        hir::Op::Union(sources) => {
            let mut inputs = vec![align(input, &out, span)];
            for source in sources {
                inputs.push(align(plan_source(source)?, &out, span));
            }
            Planned::new(PlanNode::Union { inputs }, out, span)
        }
        hir::Op::GetSchema => {
            let rows = input
                .columns
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let ty = match &c.arrow_only {
                        Some(arrow) => arrow.to_string(),
                        None => c.ty.to_string(),
                    };
                    vec![
                        ir::Value::String(c.name.clone()),
                        ir::Value::Long(i as i64),
                        ir::Value::String(ty),
                    ]
                })
                .collect();
            Planned::new(PlanNode::Values { rows }, out, span)
        }
        hir::Op::Unsupported { .. } | hir::Op::Error => {
            return Err(internal(span, "an operator with an error"));
        }
    })
}

/// `extend`: one `Project` per item, each keeping every column and adding or replacing one.
fn plan_extend(
    mut planned: Planned,
    items: &[hir::ProjectItem],
    span: Span,
) -> Result<Planned, Diagnostic> {
    for item in items {
        let mut after = planned.columns.clone();
        let column = Column {
            name: item.name.clone(),
            ty: item.expr.ty,
            read_from: None,
            arrow_only: None,
        };
        match after.iter_mut().find(|c| c.name == item.name) {
            Some(existing) => *existing = column,
            None => after.push(column),
        }
        let columns = after
            .iter()
            .map(|c| {
                Ok(if c.name == item.name {
                    ProjectItem {
                        name: c.name.clone(),
                        expr: expr(&item.expr)?,
                    }
                } else {
                    pass_through(c, span)
                })
            })
            .collect::<Result<_, Diagnostic>>()?;
        planned = Planned::new(
            PlanNode::Project {
                input: Box::new(planned.plan),
                columns,
            },
            after,
            span,
        );
    }
    Ok(planned)
}

/// `summarize`: an `Aggregate` whose items are single aggregate calls. An item that computes
/// over aggregates (`sum(a) / count()`) has its calls computed as hidden columns and is
/// finished by a `Project`.
fn plan_summarize(
    input: Planned,
    keys: &[hir::ProjectItem],
    aggregates: &[hir::ProjectItem],
    out: Vec<Column>,
    span: Span,
) -> Result<Planned, Diagnostic> {
    let group_by: Vec<ProjectItem> = keys
        .iter()
        .map(|item| project_item(item, &input.columns))
        .collect::<Result<_, _>>()?;
    let mut taken: HashSet<String> = out.iter().map(|c| c.name.clone()).collect();
    let mut calls = Vec::new();
    let mut finish = Vec::new();
    let mut needs_project = false;
    for item in aggregates {
        let e = expr(&item.expr)?;
        if is_aggregate_call(&e) {
            finish.push(pass_through_named(&item.name, e.ty, span));
            calls.push(ProjectItem {
                name: item.name.clone(),
                expr: e,
            });
        } else {
            needs_project = true;
            let mut n = 0;
            let mut hide = |call: ir::Expr| {
                n += 1;
                let name = unique_name(&format!("{}_{n}", item.name), &taken);
                taken.insert(name.clone());
                let column = column_ref(&name, call.ty, call.span);
                calls.push(ProjectItem { name, expr: call });
                column
            };
            finish.push(ProjectItem {
                name: item.name.clone(),
                expr: extract_aggregates(e, &mut hide),
            });
        }
    }
    let mut columns: Vec<Column> = out[..group_by.len()].to_vec();
    columns.extend(calls.iter().map(|item| Column {
        name: item.name.clone(),
        ty: item.expr.ty,
        read_from: None,
        arrow_only: None,
    }));
    let key_columns: Vec<ProjectItem> = columns[..group_by.len()]
        .iter()
        .map(|c| pass_through(c, span))
        .collect();
    let aggregate = Planned::new(
        PlanNode::Aggregate {
            input: Box::new(input.plan),
            group_by,
            aggregates: calls,
        },
        columns,
        span,
    );
    if !needs_project {
        return Ok(aggregate);
    }
    let columns = key_columns.into_iter().chain(finish).collect();
    Ok(Planned::new(
        PlanNode::Project {
            input: Box::new(aggregate.plan),
            columns,
        },
        out,
        span,
    ))
}

fn is_aggregate_call(e: &ir::Expr) -> bool {
    matches!(&e.kind, ir::ExprKind::Call { function, .. }
        if FunctionCatalog::builtin()
            .lookup(function)
            .is_ok_and(|f| f.kind == FunctionKind::Aggregate))
}

/// Replaces each aggregate call in `e` with what `hide` returns for it. Aggregates do not nest
/// (the binder reports that), so the calls' arguments are not searched.
fn extract_aggregates(e: ir::Expr, hide: &mut impl FnMut(ir::Expr) -> ir::Expr) -> ir::Expr {
    if is_aggregate_call(&e) {
        return hide(e);
    }
    let mut go = |e: Box<ir::Expr>| Box::new(extract_aggregates(*e, hide));
    let kind = match e.kind {
        k @ (ir::ExprKind::Column(_) | ir::ExprKind::Literal(_)) => k,
        ir::ExprKind::Binary { op, left, right } => {
            let left = go(left);
            ir::ExprKind::Binary {
                op,
                left,
                right: go(right),
            }
        }
        ir::ExprKind::Unary { op, operand } => ir::ExprKind::Unary {
            op,
            operand: go(operand),
        },
        ir::ExprKind::Cast(inner) => ir::ExprKind::Cast(go(inner)),
        ir::ExprKind::Call { function, args } => ir::ExprKind::Call {
            function,
            args: args
                .into_iter()
                .map(|a| extract_aggregates(a, hide))
                .collect(),
        },
        ir::ExprKind::In {
            expr,
            list,
            negated,
            case_insensitive,
        } => ir::ExprKind::In {
            expr: Box::new(extract_aggregates(*expr, hide)),
            list: list
                .into_iter()
                .map(|a| extract_aggregates(a, hide))
                .collect(),
            negated,
            case_insensitive,
        },
        ir::ExprKind::Between {
            expr,
            low,
            high,
            negated,
        } => {
            let expr = go(expr);
            let low = go(low);
            ir::ExprKind::Between {
                expr,
                low,
                high: go(high),
                negated,
            }
        }
    };
    ir::Expr { kind, ..e }
}

/// `join`. Semi and anti joins keep one side as it is. Inner and outer joins rename right
/// columns so that the join's output names are unique: to the names the binder chose
/// (`DS1007`), and `on key` right columns to a hidden name. A `Project` after the join then
/// merges each `on key` pair into one column (D-019).
fn plan_join(
    left: Planned,
    join: &hir::Join,
    out: Vec<Column>,
    span: Span,
) -> Result<Planned, Diagnostic> {
    let right = plan_source(&join.right)?;
    let condition = |l: &str, r: &str, key: &hir::JoinKey, lcols: &[Column], rcols: &[Column]| {
        Ok::<_, Diagnostic>(ir::JoinCondition {
            left: key_side(l, lcols, key)?,
            right: key_side(r, rcols, key)?,
        })
    };
    if let Some(side) = join.kind.one_side() {
        let on = join
            .keys
            .iter()
            .map(|k| condition(&k.left, &k.right, k, &left.columns, &right.columns))
            .collect::<Result<_, _>>()?;
        let columns = match side {
            hir::JoinSide::Left => left.columns.clone(),
            hir::JoinSide::Right => right.columns.clone(),
        };
        return Ok(Planned::new(
            PlanNode::Join {
                left: Box::new(left.plan),
                right: Box::new(right.plan),
                kind: join.kind,
                on,
            },
            columns,
            span,
        ));
    }

    let mut renames: HashMap<&str, String> = join
        .renamed
        .iter()
        .map(|(old, new)| (old.as_str(), new.clone()))
        .collect();
    let mut taken: HashSet<String> = left
        .columns
        .iter()
        .chain(&right.columns)
        .chain(&out)
        .map(|c| c.name.clone())
        .chain(renames.values().cloned())
        .collect();
    for key in join.keys.iter().filter(|k| k.merged) {
        let hidden = unique_name(&key.right, &taken);
        taken.insert(hidden.clone());
        renames.insert(&key.right, hidden);
    }
    let right = if renames.is_empty() {
        right
    } else {
        let renamed: Vec<Column> = right
            .columns
            .iter()
            .map(|c| Column {
                name: renames.get(c.name.as_str()).unwrap_or(&c.name).clone(),
                ..c.clone()
            })
            .collect();
        let columns = right
            .columns
            .iter()
            .zip(&renamed)
            .map(|(old, new)| ProjectItem {
                name: new.name.clone(),
                expr: column_ref(&old.name, old.ty, span),
            })
            .collect();
        Planned::new(
            PlanNode::Project {
                input: Box::new(right.plan),
                columns,
            },
            renamed,
            span,
        )
    };
    let right_name = |name: &str| {
        renames
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.to_string())
    };
    let on = join
        .keys
        .iter()
        .map(|k| {
            condition(
                &k.left,
                &right_name(&k.right),
                k,
                &left.columns,
                &right.columns,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let joined_columns: Vec<Column> = left.columns.iter().chain(&right.columns).cloned().collect();
    let joined = Planned::new(
        PlanNode::Join {
            left: Box::new(left.plan),
            right: Box::new(right.plan),
            kind: join.kind,
            on: on.clone(),
        },
        joined_columns,
        span,
    );
    if !join.keys.iter().any(|k| k.merged) {
        return Ok(joined);
    }
    let columns = out
        .iter()
        .map(|c| {
            let merged = join
                .keys
                .iter()
                .zip(&on)
                .find(|(k, _)| k.merged && k.left == c.name);
            match merged {
                Some((key, cond)) => {
                    let (l, r) = (cond.left.clone(), cond.right.clone());
                    let expr = match join.kind {
                        hir::JoinKind::RightOuter => r,
                        hir::JoinKind::FullOuter => ir::Expr {
                            kind: ir::ExprKind::Call {
                                function: "coalesce".to_string(),
                                args: vec![l, r],
                            },
                            ty: key.ty,
                            span: key.span,
                        },
                        _ => l,
                    };
                    ProjectItem {
                        name: c.name.clone(),
                        expr,
                    }
                }
                None => pass_through(c, span),
            }
        })
        .collect();
    Ok(Planned::new(
        PlanNode::Project {
            input: Box::new(joined.plan),
            columns,
        },
        out,
        span,
    ))
}

/// One side of a join condition: the column, cast to the key's type if it is narrower.
fn key_side(name: &str, columns: &[Column], key: &hir::JoinKey) -> Result<ir::Expr, Diagnostic> {
    let column = columns
        .iter()
        .find(|c| c.name == name)
        .ok_or_else(|| internal(key.span, "a join key that is not a column"))?;
    let reference = column_ref(name, column.ty, key.span);
    Ok(cast_to(reference, key.ty))
}

/// Projects a `union` input to the output columns: each by name, cast if the output type is
/// wider, or a typed null if the input does not have it. An input that already matches is kept.
fn align(input: Planned, out: &[Column], span: Span) -> Plan {
    let matches = input.columns.len() == out.len()
        && input
            .columns
            .iter()
            .zip(out)
            .all(|(a, b)| a.name == b.name && a.ty == b.ty && a.arrow_only == b.arrow_only);
    if matches {
        return input.plan;
    }
    let columns = out
        .iter()
        .map(|c| {
            let expr = match input.columns.iter().find(|i| i.name == c.name) {
                Some(i) => cast_to(column_ref(&i.name, i.ty, span), c.ty),
                None => ir::Expr {
                    kind: ir::ExprKind::Literal(ir::Value::Null),
                    ty: c.ty,
                    span,
                },
            };
            ProjectItem {
                name: c.name.clone(),
                expr,
            }
        })
        .collect();
    Planned::new(
        PlanNode::Project {
            input: Box::new(input.plan),
            columns,
        },
        out.to_vec(),
        span,
    )
    .plan
}

fn cast_to(e: ir::Expr, ty: Type) -> ir::Expr {
    if e.ty == ty || ty.is_error() || e.ty.is_error() {
        return e;
    }
    ir::Expr {
        span: e.span,
        kind: ir::ExprKind::Cast(Box::new(e)),
        ty,
    }
}

fn column_ref(name: &str, ty: Type, span: Span) -> ir::Expr {
    ir::Expr {
        kind: ir::ExprKind::Column(name.to_string()),
        ty,
        span,
    }
}

/// An output column that is the input column of the same name.
fn pass_through(c: &Column, span: Span) -> ProjectItem {
    pass_through_named(&c.name, c.ty, span)
}

fn pass_through_named(name: &str, ty: Type, span: Span) -> ProjectItem {
    ProjectItem {
        name: name.to_string(),
        expr: column_ref(name, ty, span),
    }
}

/// A `project` or `summarize ... by` item. A bare column with no DataScript type passes through
/// with the error type (see [`ir::Expr`]).
fn project_item(item: &hir::ProjectItem, input: &[Column]) -> Result<ProjectItem, Diagnostic> {
    let expr = match &item.expr.kind {
        hir::ExprKind::Column(name)
            if input
                .iter()
                .any(|c| c.name == *name && c.arrow_only.is_some()) =>
        {
            column_ref(name, Type::Error, item.expr.span)
        }
        _ => expr(&item.expr)?,
    };
    Ok(ProjectItem {
        name: item.name.clone(),
        expr,
    })
}

fn sort_key(key: &hir::SortKey) -> Result<ir::SortKey, Diagnostic> {
    Ok(ir::SortKey {
        expr: expr(&key.expr)?,
        descending: key.descending,
        nulls_first: key.nulls_first,
    })
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
                ir::ExprKind::Literal(ir::Value::Long(n)) if e.ty == Type::Real => {
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

/// The Arrow schema of `columns`; a column with no DataScript type keeps its Arrow type.
fn schema_of(columns: &[Column]) -> SchemaRef {
    Arc::new(Schema::new(
        columns
            .iter()
            .map(|c| match &c.arrow_only {
                Some(arrow) => Field::new(&c.name, arrow.clone(), true),
                None => to_arrow(&c.name, c.ty),
            })
            .collect::<Vec<_>>(),
    ))
}

fn internal(span: Span, what: &str) -> Diagnostic {
    Diagnostic::error(
        DS9001,
        format!("internal error: the planner was given {what}"),
    )
    .primary(span, "while planning this")
    .note("this is a bug in DataScript; please report it")
}
