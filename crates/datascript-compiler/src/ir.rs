//! The engine-neutral logical plan IR (ARCHITECTURE.md § Semantics).
//!
//! The walking skeleton (M1-15) has `Scan`, `Filter`, `Project` and `Limit`; M1-06 adds the
//! other nodes. Every node and expression carries the [`Span`] it came from, so backends can map
//! runtime errors back to the source.

use std::fmt::{self, Write as _};

use arrow_schema::SchemaRef;
use datascript_diagnostics::Span;

use crate::Type;
use crate::source::SourceRef;

pub use datascript_syntax::ast::BinaryOp;

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
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Bool(b) => write!(f, "{b}"),
            Value::Long(n) => write!(f, "{n}"),
            Value::Real(x) => write!(f, "{x:?}"),
            Value::String(s) => write!(f, "{s:?}"),
        }
    }
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            ExprKind::Column(name) => f.write_str(name),
            ExprKind::Literal(value) => write!(f, "{value}"),
            ExprKind::Binary { op, left, right } => {
                write_operand(f, left)?;
                write!(f, " {} ", crate::operators::op_symbol(*op))?;
                write_operand(f, right)
            }
            ExprKind::Cast(inner) => write!(f, "cast({inner}, {})", self.ty),
        }
    }
}

fn write_operand(f: &mut fmt::Formatter<'_>, e: &Expr) -> fmt::Result {
    if matches!(e.kind, ExprKind::Binary { .. }) {
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
