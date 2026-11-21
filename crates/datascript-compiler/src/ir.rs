//! The engine-neutral logical plan IR (ARCHITECTURE.md § Semantics).
//!
//! The walking skeleton (M1-15) has `Scan`, `Filter`, `Project` and `Limit`; M1-06 adds the
//! other nodes. M1-05 added the expression kinds the binder checks. Every node and expression carries the [`Span`] it came from, so backends can map
//! runtime errors back to the source.

use std::fmt::{self, Write as _};

use arrow_schema::SchemaRef;
use datascript_diagnostics::Span;

use crate::Type;
use crate::source::SourceRef;

pub use datascript_syntax::ast::{BinaryOp, PrefixOp};

/// A compiled program: one plan per query statement, in source order.
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub queries: Vec<QueryPlan>,
}

/// The plan of one query statement.
#[derive(Debug, Clone, PartialEq)]
pub struct QueryPlan {
    pub root: Plan,
    /// The schema of the result, in canonical Arrow types (LANGUAGE.md §5).
    pub output_schema: SchemaRef,
}

/// A plan node with its output schema and source span.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub node: PlanNode,
    /// The node's output schema, in canonical Arrow types.
    pub schema: SchemaRef,
    pub span: Span,
}

/// A relational operator.
#[derive(Debug, Clone, PartialEq)]
pub enum PlanNode {
    /// Reads a source. `schema` is the widened source schema; the backend casts each column to
    /// its canonical type at the scan (LANGUAGE.md §5.1).
    Scan { source: SourceRef },
    /// Keeps the rows for which `predicate` is true.
    Filter { input: Box<Plan>, predicate: Expr },
    /// Computes the output columns, in order.
    Project {
        input: Box<Plan>,
        columns: Vec<ProjectItem>,
    },
    /// Keeps at most `count` rows.
    Limit { input: Box<Plan>, count: u64 },
}

/// One output column of a [`PlanNode::Project`].
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectItem {
    pub name: String,
    pub expr: Expr,
}

/// A typed scalar expression.
#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub ty: Type,
    pub span: Span,
}

/// The kinds of scalar expression.
#[derive(Debug, Clone, PartialEq)]
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
#[derive(Debug, Clone, PartialEq)]
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

fn write_operand(f: &mut fmt::Formatter<'_>, e: &Expr) -> fmt::Result {
    if matches!(
        e.kind,
        ExprKind::Binary { .. }
            | ExprKind::In { .. }
            | ExprKind::Between { .. }
            | ExprKind::Unary { .. }
    ) {
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
            let columns: Vec<String> = plan
                .schema
                .fields()
                .iter()
                .map(|field| match crate::arrow::from_arrow(field) {
                    Ok(ty) => format!("{}: {ty}", field.name()),
                    Err(e) => format!("{}: arrow:{e}", field.name()),
                })
                .collect();
            writeln!(out, "{indent}Scan: {source} ({})", columns.join(", "))
        }
        PlanNode::Filter { predicate, .. } => writeln!(out, "{indent}Filter: {predicate}"),
        PlanNode::Project { columns, .. } => {
            let items: Vec<String> = columns
                .iter()
                .map(|item| match &item.expr.kind {
                    ExprKind::Column(name) if *name == item.name => name.clone(),
                    _ => format!("{} = {}", item.name, item.expr),
                })
                .collect();
            writeln!(out, "{indent}Project: {}", items.join(", "))
        }
        PlanNode::Limit { count, .. } => writeln!(out, "{indent}Limit: {count}"),
    };
    if let Some(input) = plan.input() {
        write_plan(out, input, depth + 1);
    }
}

impl Plan {
    /// The node's input, if it has one.
    pub fn input(&self) -> Option<&Plan> {
        match &self.node {
            PlanNode::Scan { .. } => None,
            PlanNode::Filter { input, .. }
            | PlanNode::Project { input, .. }
            | PlanNode::Limit { input, .. } => Some(input),
        }
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
