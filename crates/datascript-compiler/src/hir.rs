//! The typed HIR: what the binder produces and the planner reads.
//!
//! Every name is resolved and every expression has a [`Type`]. An expression that already has a
//! reported error, or that reads a source whose schema is pending, has [`ExprKind::Error`] and
//! [`Type::Error`]. The planner only sees a HIR without errors.

use std::sync::Arc;

use datascript_diagnostics::Span;
use datascript_syntax::ast::PrefixOp;

use crate::Type;
use crate::ir::{BinaryOp, Value};
use crate::source::SourceRef;

/// A bound program: its statements, in source order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Hir {
    pub statements: Vec<Statement>,
}

impl Hir {
    /// The query statements, in order.
    pub fn queries(&self) -> impl Iterator<Item = &Query> {
        self.statements.iter().filter_map(|s| match s {
            Statement::Query(q) => Some(q),
            Statement::Let(_) => None,
        })
    }
}

/// A statement.
#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    Let(LetBinding),
    Query(Query),
}

/// `let name = value`.
#[derive(Debug, Clone, PartialEq)]
pub struct LetBinding {
    pub name: String,
    /// The name where it is defined.
    pub name_span: Span,
    pub value: LetValue,
}

/// What a `let` binds. Its kind comes from the syntax (D-036).
#[derive(Debug, Clone, PartialEq)]
pub enum LetValue {
    /// A scalar expression.
    Scalar(Expr),
    /// A pipeline.
    Table(Arc<Pipeline>),
    /// The value has an error; uses of the name are silently errors too.
    Error,
}

/// A query statement.
#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    pub span: Span,
    pub pipeline: Arc<Pipeline>,
}

/// `from source | stage | stage ...`
#[derive(Debug, Clone, PartialEq)]
pub struct Pipeline {
    pub span: Span,
    pub source: Source,
    pub stages: Vec<Stage>,
    /// The output columns; `None` if they are not known (an error or a pending source).
    pub columns: Option<Vec<Column>>,
}

/// A column in scope.
#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    pub name: String,
    pub ty: Type,
    /// The source the column was read from unchanged, as written, for messages such as
    /// "column `price` was read from products.csv".
    pub read_from: Option<String>,
}

/// Where a pipeline's rows come from.
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    /// A file.
    Scan {
        source: SourceRef,
        span: Span,
        columns: Vec<Column>,
    },
    /// A tabular `let`.
    Let {
        name: String,
        span: Span,
        pipeline: Arc<Pipeline>,
    },
    /// A pipeline in parentheses.
    Subquery { span: Span, pipeline: Arc<Pipeline> },
    /// The source has an error, or its schema is pending.
    Error { span: Span },
}

/// One operator of a pipeline, with the columns after it.
#[derive(Debug, Clone, PartialEq)]
pub struct Stage {
    pub span: Span,
    pub op: Op,
    /// The columns after this stage; `None` if not known.
    pub columns: Option<Vec<Column>>,
}

/// A bound operator.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    /// `where predicate`; the predicate is `bool`.
    Where(Expr),
    /// `project` of columns.
    Project(Vec<ProjectItem>),
    /// `take n`.
    Take(u64),
    /// An operator whose expressions were checked but whose output schema is not computed yet
    /// (M1-16). Reported as not implemented.
    Unsupported { name: String },
    /// The operator has an error.
    Error,
}

/// One output column of `project`.
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

/// The kinds of scalar expression. Operands already have the types their operator or function
/// expects: the binder inserts [`ExprKind::Cast`]s.
#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    /// A column of the input.
    Column(String),
    /// A scalar `let`, with its bound value.
    Let { name: String, value: Box<Expr> },
    /// A literal. A `null` literal has the type its context gave it.
    Literal(Value),
    /// `-x`, `not x`.
    Unary { op: PrefixOp, operand: Box<Expr> },
    /// `left op right`.
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    /// A function call; `function` is the catalog name.
    Call {
        function: &'static str,
        args: Vec<Expr>,
    },
    /// `expr in (list)`, `!in`, `in~`, `!in~`.
    In {
        expr: Box<Expr>,
        list: Vec<Expr>,
        negated: bool,
        case_insensitive: bool,
    },
    /// `expr between (low .. high)`, `!between`.
    Between {
        expr: Box<Expr>,
        low: Box<Expr>,
        high: Box<Expr>,
        negated: bool,
    },
    /// Converts the operand to [`Expr::ty`].
    Cast(Box<Expr>),
    /// The expression has an error.
    Error,
}

impl Expr {
    /// An expression with an error.
    pub fn error(span: Span) -> Expr {
        Expr {
            kind: ExprKind::Error,
            ty: Type::Error,
            span,
        }
    }

    /// The direct subexpressions.
    pub fn children(&self) -> Vec<&Expr> {
        match &self.kind {
            ExprKind::Column(_) | ExprKind::Literal(_) | ExprKind::Error => Vec::new(),
            ExprKind::Let { value, .. } => vec![value],
            ExprKind::Unary { operand, .. } | ExprKind::Cast(operand) => vec![operand],
            ExprKind::Binary { left, right, .. } => vec![left, right],
            ExprKind::Call { args, .. } => args.iter().collect(),
            ExprKind::In { expr, list, .. } => std::iter::once(&**expr).chain(list).collect(),
            ExprKind::Between {
                expr, low, high, ..
            } => vec![expr, low, high],
        }
    }

    /// Whether this expression or a subexpression satisfies `f`. A `let`'s value is not
    /// searched: it was checked where the `let` is defined.
    pub fn any(&self, f: &impl Fn(&Expr) -> bool) -> bool {
        f(self)
            || (!matches!(self.kind, ExprKind::Let { .. })
                && self.children().into_iter().any(|c| c.any(f)))
    }
}
