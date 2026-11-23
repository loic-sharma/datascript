//! Lowering the IR to DataFusion logical plans (ARCHITECTURE.md § Compilation pipeline).
//!
//! Plan nodes map onto `LogicalPlanBuilder` calls one to one. Expressions use DataFusion's own
//! operators only where its semantics are exactly LANGUAGE.md's (comparisons, `and`/`or`/`not`,
//! `in`, `between`, real arithmetic, `CASE`, `coalesce`, casts that widen); everything else is a
//! UDF from [`crate::udf`] or [`crate::udaf`] that carries the span of its expression.
//! [`SUPPORTED`] lists the catalog functions this backend lowers, and the engine's
//! `Capabilities` are built from it, so a function missing here is `DS4001` at check time.

use std::collections::HashSet;
use std::sync::Arc;

use arrow_schema::{DataType, SchemaRef};
use datafusion::common::{Column, DFSchema, JoinType, ScalarValue};
use datafusion::datasource::listing::{ListingTable, ListingTableConfig};
use datafusion::datasource::provider_as_source;
use datafusion::error::DataFusionError;
use datafusion::functions::expr_fn::{coalesce, nullif};
use datafusion::functions_aggregate::expr_fn as agg;
use datafusion::logical_expr::{
    EmptyRelation, LogicalPlan, LogicalPlanBuilder, Operator, binary_expr, cast, when,
};
use datafusion::prelude::{Expr as DfExpr, lit};
use datascript_compiler::Type;
use datascript_compiler::arrow::to_arrow_type;
use datascript_compiler::ir::{
    BinaryOp, Expr, ExprKind, JoinKind, Plan, PlanNode, PrefixOp, SortKey, Value,
};
use datascript_compiler::{SourceRef, operators::op_symbol};
use datascript_diagnostics::codes::{DS4001, DS9001};
use datascript_diagnostics::{Diagnostic, Span};
use datascript_syntax::ast::StringOp;

use crate::checked;
use crate::errors::ErrorContext;
use crate::sources::{Resolved, SourceFile};
use crate::udaf;
use crate::udf::{self, Kernel};

/// The catalog functions this backend implements. `Capabilities::functions` is this list, and a
/// test checks that it is the whole M1 catalog and that each one lowers.
pub(crate) const SUPPORTED: &[&str] = &[
    // Aggregates.
    "count",
    "countif",
    "sum",
    "sumif",
    "avg",
    "min",
    "max",
    "dcount",
    "percentile",
    "stdev",
    "variance",
    "take_any",
    // Conditional.
    "iff",
    "case",
    "coalesce",
    "isnull",
    "isnotnull",
    "isempty",
    "isnotempty",
    // Conversion.
    "tostring",
    "toint",
    "tolong",
    "toreal",
    "tobool",
    "todatetime",
    "totimespan",
    // String.
    "strlen",
    "tolower",
    "toupper",
    "trim",
    "substring",
    "strcat",
    "replace_string",
    "indexof",
    "extract",
    // Datetime.
    "now",
    "ago",
    "bin",
    "startofday",
    "startofweek",
    "startofmonth",
    "startofyear",
    "endofday",
    "endofweek",
    "endofmonth",
    "endofyear",
    "datetime_part",
    "dayofweek",
    "format_datetime",
    "datetime_diff",
    // Math.
    "abs",
    "round",
    "floor",
    "ceiling",
    "sqrt",
    "pow",
    "log",
    "log10",
    "exp",
];

/// Lowers one query plan. Scans are resolved before lowering starts (resolving reads files,
/// lowering does not), and `resolved` holds them in the order [`scans`] lists them.
pub(crate) struct Lowering {
    resolved: Vec<Arc<Resolved>>,
    next_scan: usize,
    /// Joins so far; each gets its own side aliases.
    joins: usize,
    pub(crate) exprs: Exprs,
    pub(crate) errors: ErrorContext,
}

/// The `Scan` nodes of `plan`, in the order lowering meets them.
pub(crate) fn scans(plan: &Plan) -> Vec<&Plan> {
    let mut out = Vec::new();
    let mut stack = vec![plan];
    while let Some(p) = stack.pop() {
        if let PlanNode::Scan { .. } = p.node {
            out.push(p);
        }
        // Reversed, so the first input is visited first.
        stack.extend(p.inputs().into_iter().rev());
    }
    out
}

impl Lowering {
    pub(crate) fn new(resolved: Vec<Arc<Resolved>>, now: i64) -> Self {
        Lowering {
            resolved,
            next_scan: 0,
            joins: 0,
            exprs: Exprs {
                now,
                qualifier: None,
            },
            errors: ErrorContext { scans: Vec::new() },
        }
    }

    /// Lowers `plan`. Chains of single-input nodes are lowered in a loop, so only joins and
    /// unions recurse.
    pub(crate) fn plan(&mut self, plan: &Plan) -> Result<LogicalPlan, Diagnostic> {
        let mut chain = vec![plan];
        while let Some(&last) = chain.last() {
            match last.inputs().as_slice() {
                [only] if !matches!(last.node, PlanNode::Union { .. }) => chain.push(only),
                _ => break,
            }
        }
        let bottom = chain
            .pop()
            .ok_or_else(|| internal(plan.span, "the plan is empty"))?;
        let mut builder = self.leaf(bottom)?;
        for node in chain.into_iter().rev() {
            builder = self.single(node, builder)?;
        }
        builder.build().map_err(|e| self.fail(plan.span, &e))
    }

    fn fail(&self, span: Span, error: &DataFusionError) -> Diagnostic {
        self.errors.diagnostic(span, error)
    }

    /// A node whose inputs (if any) are lowered by recursion.
    fn leaf(&mut self, node: &Plan) -> Result<LogicalPlanBuilder, Diagnostic> {
        let span = node.span;
        let fail = |this: &Self, e: DataFusionError| this.fail(span, &e);
        match &node.node {
            PlanNode::Scan { source } => self.scan(source, node),
            PlanNode::Values { rows } => self.values(rows, &node.schema, span),
            PlanNode::Join {
                left,
                right,
                kind,
                on,
            } => {
                let left_plan = self.plan(left)?;
                let right_plan = self.plan(right)?;
                self.join(node, left, left_plan, right_plan, *kind, on)
            }
            PlanNode::Union { inputs } => {
                let mut builder: Option<LogicalPlanBuilder> = None;
                for input in inputs {
                    let lowered = self.plan(input)?;
                    builder = Some(match builder {
                        None => LogicalPlanBuilder::from(lowered),
                        Some(b) => b.union(lowered).map_err(|e| fail(self, e))?,
                    });
                }
                let builder = builder.ok_or_else(|| internal(span, "a union with no inputs"))?;
                // Name the columns as the node does; the first input's names already match, but
                // its qualifiers do not belong to the union.
                let columns = node
                    .schema
                    .fields()
                    .iter()
                    .enumerate()
                    .map(|(i, f)| {
                        let (qualifier, field) = builder.schema().qualified_field(i);
                        DfExpr::Column(Column::from((qualifier, field))).alias(f.name())
                    })
                    .collect::<Vec<_>>();
                builder.project(columns).map_err(|e| fail(self, e))
            }
            _ => Err(internal(span, "a plan node is missing its input")),
        }
    }

    /// A single-input node over its lowered input.
    fn single(
        &mut self,
        node: &Plan,
        input: LogicalPlanBuilder,
    ) -> Result<LogicalPlanBuilder, Diagnostic> {
        let span = node.span;
        let fail = |e: DataFusionError| self.errors.diagnostic(span, &e);
        let x = &self.exprs;
        Ok(match &node.node {
            PlanNode::Filter { predicate, .. } => input.filter(x.expr(predicate)?).map_err(fail)?,
            PlanNode::Project { columns, .. } => {
                let exprs = columns
                    .iter()
                    .map(|item| Ok(x.expr(&item.expr)?.alias(&item.name)))
                    .collect::<Result<Vec<_>, Diagnostic>>()?;
                input.project(exprs).map_err(fail)?
            }
            PlanNode::Aggregate {
                group_by,
                aggregates,
                ..
            } => {
                let keys = group_by
                    .iter()
                    .map(|item| Ok(x.expr(&item.expr)?.alias(&item.name)))
                    .collect::<Result<Vec<_>, Diagnostic>>()?;
                let aggs = aggregates
                    .iter()
                    .map(|item| Ok(x.aggregate(&item.expr)?.alias(&item.name)))
                    .collect::<Result<Vec<_>, Diagnostic>>()?;
                input.aggregate(keys, aggs).map_err(fail)?
            }
            PlanNode::Sort { keys, .. } => input.sort(x.sort_keys(keys)?).map_err(fail)?,
            PlanNode::Limit { count, .. } => {
                input.limit(0, Some(to_usize(*count))).map_err(fail)?
            }
            PlanNode::TopN { count, keys, .. } => input
                .sort(x.sort_keys(keys)?)
                .and_then(|b| b.limit(0, Some(to_usize(*count))))
                .map_err(fail)?,
            PlanNode::Distinct { .. } => input.distinct().map_err(fail)?,
            _ => return Err(internal(span, "a multi-input node in a chain")),
        })
    }

    fn values(
        &mut self,
        rows: &[Vec<Value>],
        schema: &SchemaRef,
        span: Span,
    ) -> Result<LogicalPlanBuilder, Diagnostic> {
        let fail = |e: DataFusionError| self.errors.diagnostic(span, &e);
        if rows.is_empty() {
            let schema = DFSchema::try_from(schema.as_ref().clone()).map_err(fail)?;
            return Ok(LogicalPlanBuilder::from(LogicalPlan::EmptyRelation(
                EmptyRelation {
                    produce_one_row: false,
                    schema: Arc::new(schema),
                },
            )));
        }
        let exprs = rows
            .iter()
            .map(|row| {
                row.iter()
                    .zip(schema.fields())
                    .map(|(v, f)| Ok(lit(scalar(v, f.data_type()).map_err(fail)?)))
                    .collect::<Result<Vec<_>, Diagnostic>>()
            })
            .collect::<Result<Vec<_>, Diagnostic>>()?;
        let builder = LogicalPlanBuilder::values(exprs).map_err(fail)?;
        let columns = schema
            .fields()
            .iter()
            .enumerate()
            .map(|(i, f)| {
                DfExpr::Column(Column::new_unqualified(format!("column{}", i + 1))).alias(f.name())
            })
            .collect::<Vec<_>>();
        builder.project(columns).map_err(fail)
    }

    fn join(
        &mut self,
        node: &Plan,
        left: &Plan,
        left_plan: LogicalPlan,
        right_plan: LogicalPlan,
        kind: JoinKind,
        on: &[datascript_compiler::ir::JoinCondition],
    ) -> Result<LogicalPlanBuilder, Diagnostic> {
        let span = node.span;
        let fail = |e: DataFusionError| self.errors.diagnostic(span, &e);
        self.joins += 1;
        let (la, ra) = (
            format!("join{}_left", self.joins),
            format!("join{}_right", self.joins),
        );
        let left_plan = LogicalPlanBuilder::from(left_plan)
            .alias(la.as_str())
            .and_then(LogicalPlanBuilder::build)
            .map_err(fail)?;
        let right_plan = LogicalPlanBuilder::from(right_plan)
            .alias(ra.as_str())
            .and_then(LogicalPlanBuilder::build)
            .map_err(fail)?;
        let side = |qualifier: &str| Exprs {
            now: self.exprs.now,
            qualifier: Some(qualifier.to_string()),
        };
        let (lx, rx) = (side(&la), side(&ra));
        let left_keys = on
            .iter()
            .map(|c| lx.expr(&c.left))
            .collect::<Result<Vec<_>, _>>()?;
        let right_keys = on
            .iter()
            .map(|c| rx.expr(&c.right))
            .collect::<Result<Vec<_>, _>>()?;
        let join_type = match kind {
            JoinKind::Inner => JoinType::Inner,
            JoinKind::LeftOuter => JoinType::Left,
            JoinKind::RightOuter => JoinType::Right,
            JoinKind::FullOuter => JoinType::Full,
            JoinKind::LeftSemi => JoinType::LeftSemi,
            JoinKind::LeftAnti => JoinType::LeftAnti,
            JoinKind::RightSemi => JoinType::RightSemi,
            JoinKind::RightAnti => JoinType::RightAnti,
        };
        let joined = LogicalPlanBuilder::from(left_plan)
            .join_with_expr_keys(right_plan, join_type, (left_keys, right_keys), None)
            .map_err(fail)?;
        // Drop the side qualifiers so later nodes refer to columns by name alone.
        let left_names: HashSet<&str> = left
            .schema
            .fields()
            .iter()
            .map(|f| f.name().as_str())
            .collect();
        let only_right = matches!(kind, JoinKind::RightSemi | JoinKind::RightAnti);
        let columns = node
            .schema
            .fields()
            .iter()
            .map(|f| {
                let name = f.name();
                let qualifier = if !only_right && left_names.contains(name.as_str()) {
                    &la
                } else {
                    &ra
                };
                DfExpr::Column(Column::new(Some(qualifier.as_str()), name)).alias(name)
            })
            .collect::<Vec<_>>();
        joined.project(columns).map_err(fail)
    }

    /// Scans a source and casts each column to the type of `plan.schema`.
    ///
    /// Files with the same schema are read by one table. Otherwise each file is read with its
    /// own schema, projected to the merged columns (null where a file lacks one), and the files
    /// are combined with `UNION ALL`, in path order. That keeps CSV columns aligned by name when
    /// headers differ: DataFusion applies a table schema to CSV files by position.
    fn scan(&mut self, source: &SourceRef, plan: &Plan) -> Result<LogicalPlanBuilder, Diagnostic> {
        let span = plan.span;
        let resolved = self
            .resolved
            .get(self.next_scan)
            .cloned()
            .ok_or_else(|| internal(span, "a scan that was not resolved"))?;
        self.next_scan += 1;
        self.errors.scans.push((span, source_name(source)));
        let fail = |e: DataFusionError| self.errors.diagnostic(span, &e);
        let groups: Vec<Vec<&SourceFile>> = if resolved.uniform() {
            vec![resolved.files.iter().collect()]
        } else {
            resolved.files.iter().map(|f| vec![f]).collect()
        };
        let mut combined: Option<LogicalPlanBuilder> = None;
        let null_value = resolved.reader.null_value();
        for (i, files) in groups.into_iter().enumerate() {
            let raw = files[0].raw.clone();
            // With `null_value`, read text and turn the value into null here (see `CsvReader`).
            let read_as = match null_value {
                Some(_) => as_text(&raw),
                None => raw.clone(),
            };
            let urls = files.iter().map(|f| f.url.clone()).collect();
            let config = ListingTableConfig::new_with_multi_paths(urls)
                .with_listing_options(resolved.reader.listing_options())
                .with_schema(read_as.clone());
            let provider = ListingTable::try_new(config).map_err(fail)?;
            let name = format!("source{}_{i}", self.next_scan);
            let builder =
                LogicalPlanBuilder::scan(name, provider_as_source(Arc::new(provider)), None)
                    .map_err(fail)?;
            let columns = plan
                .schema
                .fields()
                .iter()
                .map(|field| scan_column(&read_as, field.name(), field.data_type(), null_value))
                .collect::<Vec<_>>();
            let builder = builder.project(columns).map_err(fail)?;
            combined = Some(match combined {
                None => builder,
                Some(before) => before.union(builder.build().map_err(fail)?).map_err(fail)?,
            });
        }
        combined.ok_or_else(|| internal(span, "a source with no files"))
    }
}

/// How a source is named in `DS3008`: its path or table name as written.
fn source_name(source: &SourceRef) -> String {
    match source {
        SourceRef::Table(name) => name.clone(),
        other => other
            .path()
            .map_or_else(|| other.to_string(), str::to_string),
    }
}

fn to_usize(count: u64) -> usize {
    usize::try_from(count).unwrap_or(usize::MAX)
}

/// Column `name` of a file with schema `raw`, as type `target`: cast if needed, null if the
/// file does not have it, null where it holds `null_value`. `UInt64` is converted with a check,
/// since values above `i64::MAX` must be an error naming the column (LANGUAGE.md §5.1).
fn scan_column(
    raw: &arrow_schema::Schema,
    name: &str,
    target: &DataType,
    null_value: Option<&str>,
) -> DfExpr {
    let Ok(field) = raw.field_with_name(name) else {
        return cast(lit(ScalarValue::Null), target.clone()).alias(name);
    };
    let mut column = DfExpr::Column(Column::new_unqualified(name));
    if let Some(text) = null_value {
        column = nullif(column, lit(text));
    }
    if field.data_type() == target {
        column.alias(name)
    } else if field.data_type() == &DataType::UInt64 && target == &DataType::Int64 {
        checked::uint64_to_long(name).call(vec![column]).alias(name)
    } else {
        cast(column, target.clone()).alias(name)
    }
}

/// `schema` with every column read as text.
fn as_text(schema: &arrow_schema::Schema) -> SchemaRef {
    let fields: Vec<arrow_schema::Field> = schema
        .fields()
        .iter()
        .map(|f| arrow_schema::Field::new(f.name(), DataType::Utf8, true))
        .collect();
    Arc::new(arrow_schema::Schema::new(fields))
}

/// A literal as a DataFusion scalar of type `ty`.
fn scalar(value: &Value, ty: &DataType) -> Result<ScalarValue, DataFusionError> {
    Ok(match value {
        Value::Bool(b) => ScalarValue::Boolean(Some(*b)),
        Value::Long(n) => ScalarValue::Int64(Some(*n)),
        Value::Real(x) => ScalarValue::Float64(Some(*x)),
        Value::String(s) => ScalarValue::Utf8(Some(s.clone())),
        Value::DateTime(us) => ScalarValue::TimestampMicrosecond(Some(*us), Some("UTC".into())),
        Value::TimeSpan(us) => ScalarValue::DurationMicrosecond(Some(*us)),
        Value::Null => ScalarValue::try_from(ty)?,
    })
}

/// Lowers scalar and aggregate expressions.
pub(crate) struct Exprs {
    /// `now()` for this run, in µs (D-015).
    pub now: i64,
    /// The table alias column references get inside a join condition.
    pub qualifier: Option<String>,
}

impl Exprs {
    fn sort_keys(
        &self,
        keys: &[SortKey],
    ) -> Result<Vec<datafusion::logical_expr::SortExpr>, Diagnostic> {
        // Null placement is always explicit: DataFusion's defaults differ from LANGUAGE.md §3.4.
        keys.iter()
            .map(|k| Ok(self.expr(&k.expr)?.sort(!k.descending, k.nulls_first)))
            .collect()
    }

    /// A UDF call computing `e`.
    fn kernel(&self, name: &'static str, kernel: Kernel, e: &Expr, args: Vec<DfExpr>) -> DfExpr {
        let what = match &e.kind {
            ExprKind::Call { function, .. } => format!("`{function}`"),
            ExprKind::Binary { op, .. } => format!("`{}`", op_symbol(*op)),
            ExprKind::Unary { .. } => "`-`".to_string(),
            _ => "this expression".to_string(),
        };
        udf::udf(name, kernel, to_arrow_type(e.ty), e.span, what, e.ty.name()).call(args)
    }

    /// A helper UDF whose result is an intermediate value of type `ty` (such as the lowercased
    /// operands of `=~`); its errors are reported at `e`.
    fn helper(
        &self,
        name: &'static str,
        kernel: Kernel,
        ty: Type,
        e: &Expr,
        args: Vec<DfExpr>,
    ) -> DfExpr {
        udf::udf(
            name,
            kernel,
            to_arrow_type(ty),
            e.span,
            format!("`{}`", name),
            ty.name(),
        )
        .call(args)
    }

    fn lower_text(&self, e: &Expr, arg: DfExpr) -> DfExpr {
        self.helper("tolower", udf::k_lower, Type::String, e, vec![arg])
    }

    pub(crate) fn expr(&self, e: &Expr) -> Result<DfExpr, Diagnostic> {
        Ok(match &e.kind {
            // `Column::new`, not `col()`, which lowercases unquoted names.
            ExprKind::Column(name) => {
                DfExpr::Column(Column::new(self.qualifier.as_deref(), name.as_str()))
            }
            ExprKind::Literal(value) => lit(scalar(value, &to_arrow_type(e.ty))
                .map_err(|err| internal(e.span, &err.to_string()))?),
            ExprKind::Binary { op, left, right } => self.binary(e, *op, left, right)?,
            ExprKind::Unary { op, operand } => {
                let inner = self.expr(operand)?;
                match op {
                    PrefixOp::Not => DfExpr::Not(Box::new(inner)),
                    PrefixOp::Neg if e.ty == Type::Real => DfExpr::Negative(Box::new(inner)),
                    PrefixOp::Neg => self.kernel("ds_neg", udf::k_neg, e, vec![inner]),
                }
            }
            ExprKind::In {
                expr: lhs,
                list,
                negated,
                case_insensitive,
            } => {
                let mut lhs = self.expr(lhs)?;
                let mut list = list
                    .iter()
                    .map(|i| self.expr(i))
                    .collect::<Result<Vec<_>, _>>()?;
                if *case_insensitive {
                    lhs = self.lower_text(e, lhs);
                    list = list.into_iter().map(|i| self.lower_text(e, i)).collect();
                }
                lhs.in_list(list, *negated)
            }
            ExprKind::Between {
                expr: lhs,
                low,
                high,
                negated,
            } => {
                let (lhs, low, high) = (self.expr(lhs)?, self.expr(low)?, self.expr(high)?);
                if *negated {
                    lhs.not_between(low, high)
                } else {
                    lhs.between(low, high)
                }
            }
            ExprKind::Cast(inner) => cast(self.expr(inner)?, to_arrow_type(e.ty)),
            ExprKind::Call { function, args } => self.call(e, function, args)?,
        })
    }

    fn binary(
        &self,
        e: &Expr,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
    ) -> Result<DfExpr, Diagnostic> {
        let (l, r) = (self.expr(left)?, self.expr(right)?);
        let df = |op: Operator| Ok(binary_expr(l.clone(), op, r.clone()));
        let reals = left.ty == Type::Real && right.ty == Type::Real;
        match op {
            BinaryOp::Eq => df(Operator::Eq),
            BinaryOp::NotEq => df(Operator::NotEq),
            BinaryOp::Lt => df(Operator::Lt),
            BinaryOp::LtEq => df(Operator::LtEq),
            BinaryOp::Gt => df(Operator::Gt),
            BinaryOp::GtEq => df(Operator::GtEq),
            BinaryOp::And => df(Operator::And),
            BinaryOp::Or => df(Operator::Or),
            // Real arithmetic is IEEE 754 in DataFusion too (LANGUAGE.md §4.1).
            BinaryOp::Add if reals => df(Operator::Plus),
            BinaryOp::Sub if reals => df(Operator::Minus),
            BinaryOp::Mul if reals => df(Operator::Multiply),
            BinaryOp::Div if reals => df(Operator::Divide),
            BinaryOp::Mod if reals => df(Operator::Modulo),
            BinaryOp::Add => Ok(self.kernel("ds_add", udf::k_add, e, vec![l, r])),
            BinaryOp::Sub => Ok(self.kernel("ds_sub", udf::k_sub, e, vec![l, r])),
            BinaryOp::Mul => Ok(self.kernel("ds_mul", udf::k_mul, e, vec![l, r])),
            BinaryOp::Div if e.ty == Type::Real => {
                Ok(self.kernel("ds_div_real", udf::k_div_real, e, vec![l, r]))
            }
            BinaryOp::Div => Ok(self.kernel("ds_div", udf::k_div, e, vec![l, r])),
            BinaryOp::Mod => Ok(self.kernel("ds_mod", udf::k_mod, e, vec![l, r])),
            BinaryOp::EqCi | BinaryOp::NotEqCi => {
                let (l, r) = (self.lower_text(e, l), self.lower_text(e, r));
                let op = if op == BinaryOp::EqCi {
                    Operator::Eq
                } else {
                    Operator::NotEq
                };
                Ok(binary_expr(l, op, r))
            }
            BinaryOp::String { op, negated } => {
                let (name, kernel): (&'static str, Kernel) = match op {
                    StringOp::Has => ("has", udf::k_has),
                    StringOp::HasCs => ("has_cs", udf::k_has_cs),
                    StringOp::Contains => ("contains", udf::k_contains),
                    StringOp::ContainsCs => ("contains_cs", udf::k_contains_cs),
                    StringOp::StartsWith => ("startswith", udf::k_startswith),
                    StringOp::StartsWithCs => ("startswith_cs", udf::k_startswith_cs),
                    StringOp::EndsWith => ("endswith", udf::k_endswith),
                    StringOp::EndsWithCs => ("endswith_cs", udf::k_endswith_cs),
                };
                let test = self.kernel(name, kernel, e, vec![l, r]);
                Ok(if negated {
                    DfExpr::Not(Box::new(test))
                } else {
                    test
                })
            }
            BinaryOp::MatchesRegex => {
                Ok(self.kernel("matches_regex", udf::k_matches_regex, e, vec![l, r]))
            }
        }
    }

    fn call(&self, e: &Expr, function: &str, args: &[Expr]) -> Result<DfExpr, Diagnostic> {
        let lowered = args
            .iter()
            .map(|a| self.expr(a))
            .collect::<Result<Vec<_>, _>>()?;
        let arg = |i: usize| lowered[i].clone();
        let real = |i: usize| {
            if args[i].ty == Type::Real {
                arg(i)
            } else {
                cast(arg(i), DataType::Float64)
            }
        };
        let k =
            |name: &'static str, kernel: Kernel, a: Vec<DfExpr>| self.kernel(name, kernel, e, a);
        let now = lit(ScalarValue::TimestampMicrosecond(
            Some(self.now),
            Some("UTC".into()),
        ));
        Ok(match function {
            "iff" => when(arg(0), arg(1))
                .otherwise(arg(2))
                .map_err(|err| internal(e.span, &err.to_string()))?,
            "case" => {
                let (pairs, last) = lowered.split_at(lowered.len() - 1);
                let mut builder = when(pairs[0].clone(), pairs[1].clone());
                for pair in pairs[2..].chunks(2) {
                    builder = builder.when(pair[0].clone(), pair[1].clone());
                }
                builder
                    .otherwise(last[0].clone())
                    .map_err(|err| internal(e.span, &err.to_string()))?
            }
            "coalesce" => coalesce(lowered),
            "isnull" => arg(0).is_null(),
            "isnotnull" => arg(0).is_not_null(),
            "isempty" if args[0].ty == Type::String => arg(0).is_null().or(arg(0).eq(lit(""))),
            "isempty" => arg(0).is_null(),
            "isnotempty" if args[0].ty == Type::String => {
                arg(0).is_not_null().and(arg(0).not_eq(lit("")))
            }
            "isnotempty" => arg(0).is_not_null(),
            "tostring" => k("tostring", udf::k_tostring, lowered),
            "toint" => k("toint", udf::k_toint, lowered),
            "tolong" => k("tolong", udf::k_tolong, lowered),
            "toreal" => k("toreal", udf::k_toreal, lowered),
            "tobool" => k("tobool", udf::k_tobool, lowered),
            "todatetime" => k("todatetime", udf::k_todatetime, lowered),
            "totimespan" => k("totimespan", udf::k_totimespan, lowered),
            "strlen" => k("strlen", udf::k_strlen, lowered),
            "tolower" => k("tolower", udf::k_lower, lowered),
            "toupper" => k("toupper", udf::k_upper, lowered),
            "trim" => k("trim", udf::k_trim, lowered),
            "substring" => k("substring", udf::k_substring, lowered),
            "strcat" => k("strcat", udf::k_strcat, lowered),
            "replace_string" => k("replace_string", udf::k_replace_string, lowered),
            "indexof" => k("indexof", udf::k_indexof, lowered),
            "extract" => k("extract", udf::k_extract, lowered),
            "now" if lowered.is_empty() => now,
            "now" => k("ds_add", udf::k_add, vec![now, arg(0)]),
            "ago" => k("ds_sub", udf::k_sub, vec![now, arg(0)]),
            "bin" => k("bin", udf::k_bin, lowered),
            "startofday" => k("startofday", udf::k_startofday, lowered),
            "startofweek" => k("startofweek", udf::k_startofweek, lowered),
            "startofmonth" => k("startofmonth", udf::k_startofmonth, lowered),
            "startofyear" => k("startofyear", udf::k_startofyear, lowered),
            "endofday" => k("endofday", udf::k_endofday, lowered),
            "endofweek" => k("endofweek", udf::k_endofweek, lowered),
            "endofmonth" => k("endofmonth", udf::k_endofmonth, lowered),
            "endofyear" => k("endofyear", udf::k_endofyear, lowered),
            "datetime_part" => k("datetime_part", udf::k_datetime_part, lowered),
            "dayofweek" => k("dayofweek", udf::k_dayofweek, lowered),
            "format_datetime" => k("format_datetime", udf::k_format_datetime, lowered),
            "datetime_diff" => k("datetime_diff", udf::k_datetime_diff, lowered),
            "abs" => k("abs", udf::k_abs, lowered),
            "round" => k("round", udf::k_round, lowered),
            "floor" | "ceiling" if e.ty != Type::Real => cast(arg(0), DataType::Int64),
            "floor" => k("floor", udf::k_floor, lowered),
            "ceiling" => k("ceiling", udf::k_ceiling, lowered),
            "sqrt" => k("sqrt", udf::k_sqrt, vec![real(0)]),
            "log" => k("log", udf::k_ln, vec![real(0)]),
            "log10" => k("log10", udf::k_log10, vec![real(0)]),
            "exp" => k("exp", udf::k_exp, vec![real(0)]),
            "pow" => k("pow", udf::k_pow, vec![real(0), real(1)]),
            other if SUPPORTED.contains(&other) => {
                return Err(internal(
                    e.span,
                    &format!("the aggregate `{other}` outside `summarize`"),
                ));
            }
            other => return Err(unsupported(e.span, other)),
        })
    }

    /// An aggregate call of a `PlanNode::Aggregate` item.
    pub(crate) fn aggregate(&self, e: &Expr) -> Result<DfExpr, Diagnostic> {
        let ExprKind::Call { function, args } = &e.kind else {
            return Err(internal(e.span, "an aggregate item that is not a call"));
        };
        let lowered = args
            .iter()
            .map(|a| self.expr(a))
            .collect::<Result<Vec<_>, _>>()?;
        let arg = |i: usize| lowered[i].clone();
        let real = |i: usize| cast(arg(i), DataType::Float64);
        let case_err = |err: DataFusionError| internal(e.span, &err.to_string());
        let sum = |x: DfExpr| {
            if e.ty == Type::Real {
                agg::sum(x)
            } else {
                udaf::checked_sum(function, to_arrow_type(e.ty), e.ty.name(), e.span).call(vec![x])
            }
        };
        Ok(match function.as_str() {
            "count" => agg::count(lit(1i64)),
            "countif" => agg::count(when(arg(0), lit(1i64)).end().map_err(case_err)?),
            "sum" => sum(arg(0)),
            "sumif" => sum(when(arg(1), arg(0)).end().map_err(case_err)?),
            // The mean of a timespan truncates to whole µs.
            "avg" if e.ty == Type::TimeSpan => cast(
                cast(
                    agg::avg(cast(cast(arg(0), DataType::Int64), DataType::Float64)),
                    DataType::Int64,
                ),
                to_arrow_type(Type::TimeSpan),
            ),
            "avg" => agg::avg(real(0)),
            "min" => agg::min(arg(0)),
            "max" => agg::max(arg(0)),
            "dcount" => agg::count_distinct(arg(0)),
            "percentile" => {
                udaf::percentile(to_arrow_type(e.ty), e.span).call(vec![arg(0), real(1)])
            }
            "stdev" => agg::stddev(real(0)),
            "variance" => agg::var_sample(real(0)),
            // The smallest value: any non-null value, and the same one on every run.
            "take_any" => agg::min(arg(0)),
            other if SUPPORTED.contains(&other) => {
                return Err(internal(e.span, &format!("`{other}` used as an aggregate")));
            }
            other => return Err(unsupported(e.span, other)),
        })
    }
}

/// `DS4001` for a function this backend has no mapping for. The capability pass reports it at
/// check time, so this only fires if a plan bypassed checking.
fn unsupported(span: Span, function: &str) -> Diagnostic {
    Diagnostic::error(
        DS4001,
        format!("the DataFusion engine does not support `{function}`"),
    )
    .primary(span, "not supported by this engine")
}

pub(crate) fn internal(span: Span, message: &str) -> Diagnostic {
    Diagnostic::error(DS9001, format!("internal error: {message}"))
        .primary(span, "while running this")
        .note("this is a bug in DataScript; please report it")
}

#[cfg(test)]
mod tests {
    use super::*;
    use datascript_compiler::catalog::{FunctionCatalog, ParamType};
    use datascript_compiler::types::TypeClass;
    use datascript_compiler::{FunctionKind, capabilities::FunctionSupport};

    #[test]
    fn supported_is_the_whole_catalog() {
        let catalog = FunctionCatalog::builtin();
        let mut names: Vec<&str> = catalog.names().collect();
        names.sort_unstable();
        let mut supported = SUPPORTED.to_vec();
        supported.sort_unstable();
        assert_eq!(supported, names);
        let caps = crate::capabilities();
        assert!(matches!(caps.functions, FunctionSupport::Only(_)));
        assert!(names.iter().all(|n| caps.supports_function(catalog, n)));
        assert!(caps.deviations.is_empty());
    }

    /// A type a parameter of this class accepts.
    fn example(ty: ParamType) -> Type {
        let class = match ty {
            ParamType::Class(c) | ParamType::Var(c) => c,
        };
        match class {
            TypeClass::Exact(t) => t,
            TypeClass::Integer | TypeClass::Numeric | TypeClass::Comparable => Type::Long,
            TypeClass::Any => Type::String,
        }
    }

    /// Every form of every catalog function lowers, so a call the binder accepts never reaches
    /// the "not supported" arm at run time.
    #[test]
    fn every_signature_lowers() {
        let catalog = FunctionCatalog::builtin();
        let exprs = Exprs {
            now: 0,
            qualifier: None,
        };
        let span = Span::new(datascript_diagnostics::FileId(0), 0, 1);
        for name in catalog.names() {
            let def = catalog.lookup(name).unwrap();
            for sig in &def.signatures {
                // The required arguments, then each optional one in turn.
                let mut n = sig.min_args();
                loop {
                    if !sig.accepts_arity(n) {
                        break;
                    }
                    let types: Vec<Type> =
                        (0..n).map(|i| example(sig.param_for(i, n).ty)).collect();
                    let resolved = def.resolve(&types).unwrap();
                    let args = types
                        .iter()
                        .map(|t| Expr {
                            kind: ExprKind::Literal(Value::Null),
                            ty: *t,
                            span,
                        })
                        .collect();
                    let call = Expr {
                        kind: ExprKind::Call {
                            function: name.to_string(),
                            args,
                        },
                        ty: resolved.return_type,
                        span,
                    };
                    let lowered = match def.kind {
                        FunctionKind::Scalar => exprs.expr(&call),
                        FunctionKind::Aggregate => exprs.aggregate(&call),
                    };
                    assert!(lowered.is_ok(), "{name}/{n}: {lowered:?}");
                    if sig.max_args().is_none_or(|max| n >= max) {
                        break;
                    }
                    n += 1;
                }
            }
        }
    }
}
