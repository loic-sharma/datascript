//! The engine-neutral logical plan IR (ARCHITECTURE.md § Semantics, D-002).
//!
//! The planner (`plan.rs`) lowers the typed HIR to these nodes, desugaring as it goes: `extend`,
//! `project-away` and `project-rename` become [`PlanNode::Project`], `count` an
//! [`PlanNode::Aggregate`], `top` a [`PlanNode::TopN`], `getschema` [`PlanNode::Values`], and
//! tabular `let`s are inlined. Every node and expression carries the [`Span`] it came from, so
//! backends can map runtime errors back to the source. The IR serializes with serde, for
//! debugging and for engines in another process.

use std::fmt::{self, Write as _};

use arrow_schema::SchemaRef;
use datascript_diagnostics::Span;
use serde::{Deserialize, Serialize};

use crate::Type;
use crate::source::SourceRef;

pub use crate::capabilities::JoinKind;
pub use datascript_syntax::ast::{BinaryOp, PrefixOp};

/// A compiled program: one plan per query statement, in source order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Program {
    pub queries: Vec<QueryPlan>,
}

/// The plan of one query statement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueryPlan {
    pub root: Plan,
    /// The schema of the result, in canonical Arrow types (LANGUAGE.md §5).
    pub output_schema: SchemaRef,
    /// What a final `render` asks frontends to draw; `None` without one.
    pub render: Option<RenderSpec>,
    /// The whole query statement, from `from` to its last operator. Frontends name a result by
    /// the line it starts on.
    pub span: Span,
}

/// Placeholder for the chart a `render` operator describes. M1-10 defines its contents.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RenderSpec {}

/// A plan node with its output schema and source span.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    pub node: PlanNode,
    /// The node's output schema, in canonical Arrow types. A column with no DataScript type
    /// (`hir::Column::arrow_only`) keeps the Arrow type it was read with.
    pub schema: SchemaRef,
    pub span: Span,
}

/// A relational operator. Column names in a node's output schema are unique.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PlanNode {
    /// Reads a source. `schema` is the widened source schema; the backend casts each column to
    /// its canonical type at the scan (LANGUAGE.md §5.1).
    Scan { source: SourceRef },
    /// Literal rows; each row has one value per column of `schema`.
    Values { rows: Vec<Vec<Value>> },
    /// Keeps the rows for which `predicate` is true.
    Filter { input: Box<Plan>, predicate: Expr },
    /// Computes the output columns, in order.
    Project {
        input: Box<Plan>,
        columns: Vec<ProjectItem>,
    },
    /// Groups by `group_by` and computes `aggregates` per group. The output is the keys, then
    /// the aggregates. Each aggregate is one call of an aggregate function whose arguments are
    /// row expressions. With no keys there is exactly one group, even over no rows; with no
    /// aggregates the output is the distinct keys.
    Aggregate {
        input: Box<Plan>,
        group_by: Vec<ProjectItem>,
        aggregates: Vec<ProjectItem>,
    },
    /// Orders the rows by `keys`, the first key first.
    Sort {
        input: Box<Plan>,
        keys: Vec<SortKey>,
    },
    /// Keeps at most `count` rows.
    Limit { input: Box<Plan>, count: u64 },
    /// The first `count` rows in the order of `keys`: `Limit` over `Sort`.
    TopN {
        input: Box<Plan>,
        count: u64,
        keys: Vec<SortKey>,
    },
    /// Removes duplicate rows.
    Distinct { input: Box<Plan> },
    /// An equi-join. Each condition's `left` reads the left input and `right` the right one.
    /// The output is the left columns, then the right ones; semi and anti joins output only
    /// their side's columns. The planner renames right columns so that names stay unique.
    Join {
        left: Box<Plan>,
        right: Box<Plan>,
        kind: JoinKind,
        on: Vec<JoinCondition>,
    },
    /// Appends the rows of each input, in order. Every input has the node's schema.
    Union { inputs: Vec<Plan> },
}

/// One key of a [`PlanNode::Sort`] or [`PlanNode::TopN`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SortKey {
    pub expr: Expr,
    pub descending: bool,
    /// Whether nulls come first. Always explicit: the defaults differ between engines.
    pub nulls_first: bool,
}

/// One equality of a [`PlanNode::Join`]; both sides have the same type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JoinCondition {
    pub left: Expr,
    pub right: Expr,
}

/// One output column of a [`PlanNode::Project`] or [`PlanNode::Aggregate`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectItem {
    pub name: String,
    pub expr: Expr,
}

/// A typed scalar expression.
///
/// A reference to a column with no DataScript type (one that can only be projected,
/// `hir::Column::arrow_only`) has the type [`Type::Error`]; its Arrow type is in the input
/// schema. No other expression has the error type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Expr {
    pub kind: ExprKind,
    pub ty: Type,
    pub span: Span,
}

/// The kinds of scalar expression.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ExprKind {
    /// A column of the input, by name.
    Column(String),
    Literal(Value),
    /// `left op right`; both operands already have the types the operator expects.
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    /// `-x` or `not x`; the operand already has the type the operator expects.
    Unary {
        op: PrefixOp,
        operand: Box<Expr>,
    },
    /// A function call by catalog name; arguments already have the parameter types.
    Call {
        function: String,
        args: Vec<Expr>,
    },
    /// `expr in (list)`; every operand has one type.
    In {
        expr: Box<Expr>,
        list: Vec<Expr>,
        negated: bool,
        case_insensitive: bool,
    },
    /// `expr between (low .. high)`, inclusive; every operand has one type.
    Between {
        expr: Box<Expr>,
        low: Box<Expr>,
        high: Box<Expr>,
        negated: bool,
    },
    /// Converts the operand to [`Expr::ty`].
    Cast(Box<Expr>),
}

/// A literal value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Value {
    Bool(bool),
    Long(i64),
    Real(f64),
    String(String),
    /// Microseconds since the Unix epoch, UTC.
    DateTime(i64),
    /// Microseconds.
    TimeSpan(i64),
    /// A null of the expression's type.
    Null,
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Bool(b) => write!(f, "{b}"),
            Value::Long(n) => write!(f, "{n}"),
            Value::Real(x) => write!(f, "{x:?}"),
            Value::String(s) => write!(f, "{s:?}"),
            Value::DateTime(us) => write!(f, "datetime({})", format_datetime(*us)),
            Value::TimeSpan(us) => write!(f, "{}", format_timespan(*us)),
            Value::Null => f.write_str("null"),
        }
    }
}

/// A timespan in the largest short-form unit that holds it exactly: `1d`, `90m`, `250ms`.
fn format_timespan(us: i64) -> String {
    const UNITS: [(i64, &str); 6] = [
        (86_400_000_000, "d"),
        (3_600_000_000, "h"),
        (60_000_000, "m"),
        (1_000_000, "s"),
        (1_000, "ms"),
        (1, "us"),
    ];
    if us == 0 {
        return "0s".to_string();
    }
    let (size, unit) = UNITS
        .into_iter()
        .find(|(size, _)| us % size == 0)
        .unwrap_or((1, "us"));
    format!("{}{unit}", us / size)
}

/// ISO 8601 in UTC, with a fraction only when there is one.
fn format_datetime(us: i64) -> String {
    let days = us.div_euclid(86_400_000_000);
    let rem = us.rem_euclid(86_400_000_000);
    // Howard Hinnant's civil-from-days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let (secs, frac) = (rem / 1_000_000, rem % 1_000_000);
    let mut out = format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}",
        secs / 3600,
        secs / 60 % 60,
        secs % 60
    );
    if frac != 0 {
        let digits = format!("{frac:06}");
        out.push('.');
        out.push_str(digits.trim_end_matches('0'));
    }
    out.push('Z');
    out
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            ExprKind::Column(name) => f.write_str(name),
            ExprKind::Literal(Value::Null) => write!(f, "{}(null)", self.ty),
            ExprKind::Literal(value) => write!(f, "{value}"),
            ExprKind::Binary { op, left, right } => {
                write_operand(f, left)?;
                write!(f, " {} ", crate::operators::op_symbol(*op))?;
                write_operand(f, right)
            }
            ExprKind::Unary { op, operand } => {
                match op {
                    PrefixOp::Neg => f.write_str("-")?,
                    PrefixOp::Not => f.write_str("not ")?,
                }
                write_operand(f, operand)
            }
            ExprKind::Call { function, args } => {
                write!(f, "{function}(")?;
                write_list(f, args)?;
                f.write_str(")")
            }
            ExprKind::In {
                expr,
                list,
                negated,
                case_insensitive,
            } => {
                write_operand(f, expr)?;
                let bang = if *negated { "!" } else { "" };
                let tilde = if *case_insensitive { "~" } else { "" };
                write!(f, " {bang}in{tilde} (")?;
                write_list(f, list)?;
                f.write_str(")")
            }
            ExprKind::Between {
                expr,
                low,
                high,
                negated,
            } => {
                write_operand(f, expr)?;
                let bang = if *negated { "!" } else { "" };
                write!(f, " {bang}between ({low} .. {high})")
            }
            ExprKind::Cast(inner) => write!(f, "cast({inner}, {})", self.ty),
        }
    }
}

fn write_list(f: &mut fmt::Formatter<'_>, items: &[Expr]) -> fmt::Result {
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            f.write_str(", ")?;
        }
        write!(f, "{item}")?;
    }
    Ok(())
}

fn needs_parens(e: &Expr) -> bool {
    matches!(
        e.kind,
        ExprKind::Binary { .. }
            | ExprKind::In { .. }
            | ExprKind::Between { .. }
            | ExprKind::Unary { .. }
    )
}

fn write_operand(f: &mut fmt::Formatter<'_>, e: &Expr) -> fmt::Result {
    if needs_parens(e) {
        write!(f, "({e})")
    } else {
        write!(f, "{e}")
    }
}

impl Program {
    /// The deterministic tree printout used by `ds explain` and `mode: explain` golden tests:
    /// one tree per query, separated by blank lines, each node indented two spaces under its
    /// parent.
    pub fn explain(&self) -> String {
        let mut out = String::new();
        for (i, query) in self.queries.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            write_plan(&mut out, &query.root, 0);
        }
        out
    }
}

fn write_plan(out: &mut String, plan: &Plan, depth: usize) {
    let indent = "  ".repeat(depth);
    let _ = match &plan.node {
        PlanNode::Scan { source } => {
            writeln!(
                out,
                "{indent}Scan: {source} ({})",
                schema_text(&plan.schema)
            )
        }
        PlanNode::Values { rows } => {
            let rows: Vec<String> = rows
                .iter()
                .map(|row| {
                    let values: Vec<String> = row.iter().map(Value::to_string).collect();
                    format!("({})", values.join(", "))
                })
                .collect();
            writeln!(
                out,
                "{indent}Values: ({}) [{}]",
                schema_text(&plan.schema),
                rows.join(", ")
            )
        }
        PlanNode::Filter { predicate, .. } => writeln!(out, "{indent}Filter: {predicate}"),
        PlanNode::Project { columns, .. } => {
            writeln!(out, "{indent}Project: {}", items_text(columns))
        }
        PlanNode::Aggregate {
            group_by,
            aggregates,
            ..
        } => {
            let mut line = items_text(aggregates);
            if !group_by.is_empty() {
                if !line.is_empty() {
                    line.push(' ');
                }
                line.push_str("by ");
                line.push_str(&items_text(group_by));
            }
            writeln!(out, "{indent}Aggregate: {line}")
        }
        PlanNode::Sort { keys, .. } => writeln!(out, "{indent}Sort: {}", keys_text(keys)),
        PlanNode::Limit { count, .. } => writeln!(out, "{indent}Limit: {count}"),
        PlanNode::TopN { count, keys, .. } => {
            writeln!(out, "{indent}TopN: {count} by {}", keys_text(keys))
        }
        PlanNode::Distinct { .. } => writeln!(out, "{indent}Distinct"),
        PlanNode::Join { kind, on, .. } => {
            let on: Vec<String> = on
                .iter()
                .map(|c| {
                    let mut text = String::new();
                    let _ = write_operand_to(&mut text, &c.left);
                    text.push_str(" == ");
                    let _ = write_operand_to(&mut text, &c.right);
                    text
                })
                .collect();
            writeln!(out, "{indent}Join: {kind} on {}", on.join(", "))
        }
        PlanNode::Union { .. } => writeln!(out, "{indent}Union"),
    };
    for input in plan.inputs() {
        write_plan(out, input, depth + 1);
    }
}

/// `name: type, ...`; a column with no DataScript type shows its Arrow type.
fn schema_text(schema: &SchemaRef) -> String {
    let columns: Vec<String> = schema
        .fields()
        .iter()
        .map(|field| match crate::arrow::from_arrow(field) {
            Ok(ty) => format!("{}: {ty}", field.name()),
            Err(_) => format!("{}: arrow {}", field.name(), field.data_type()),
        })
        .collect();
    columns.join(", ")
}

/// `name` for a column passed through unchanged, `name = expr` otherwise.
fn items_text(items: &[ProjectItem]) -> String {
    let items: Vec<String> = items
        .iter()
        .map(|item| match &item.expr.kind {
            ExprKind::Column(name) if *name == item.name => name.clone(),
            _ => format!("{} = {}", item.name, item.expr),
        })
        .collect();
    items.join(", ")
}

fn keys_text(keys: &[SortKey]) -> String {
    let keys: Vec<String> = keys
        .iter()
        .map(|k| {
            let mut text = String::new();
            let _ = write_operand_to(&mut text, &k.expr);
            text.push_str(if k.descending { " desc" } else { " asc" });
            text.push_str(if k.nulls_first {
                " nulls first"
            } else {
                " nulls last"
            });
            text
        })
        .collect();
    keys.join(", ")
}

fn write_operand_to(out: &mut String, e: &Expr) -> fmt::Result {
    if needs_parens(e) {
        write!(out, "({e})")
    } else {
        write!(out, "{e}")
    }
}

impl Plan {
    /// The node's inputs, in order: the left input of a join first.
    pub fn inputs(&self) -> Vec<&Plan> {
        match &self.node {
            PlanNode::Scan { .. } | PlanNode::Values { .. } => Vec::new(),
            PlanNode::Filter { input, .. }
            | PlanNode::Project { input, .. }
            | PlanNode::Aggregate { input, .. }
            | PlanNode::Sort { input, .. }
            | PlanNode::Limit { input, .. }
            | PlanNode::TopN { input, .. }
            | PlanNode::Distinct { input } => vec![input],
            PlanNode::Join { left, right, .. } => vec![left, right],
            PlanNode::Union { inputs } => inputs.iter().collect(),
        }
    }

    /// The first input: the only one of most nodes, the left one of a join.
    pub fn first_input(&self) -> Option<&Plan> {
        self.inputs().into_iter().next()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn datetimes_print_as_iso_8601() {
        assert_eq!(format_datetime(0), "1970-01-01T00:00:00Z");
        assert_eq!(
            format_datetime(1_706_659_200_000_000),
            "2024-01-31T00:00:00Z"
        );
        assert_eq!(
            format_datetime(951_782_400_000_000 + 1_500),
            "2000-02-29T00:00:00.0015Z"
        );
        assert_eq!(format_datetime(-1), "1969-12-31T23:59:59.999999Z");
    }

    #[test]
    fn timespans_print_in_the_largest_exact_unit() {
        assert_eq!(format_timespan(0), "0s");
        assert_eq!(format_timespan(86_400_000_000), "1d");
        assert_eq!(format_timespan(5_400_000_000), "90m");
        assert_eq!(format_timespan(-250_000), "-250ms");
        assert_eq!(format_timespan(7), "7us");
    }
}
