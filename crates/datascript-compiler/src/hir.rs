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
    /// The Arrow type of a column with no DataScript type (`ty` is then the error type). It
    /// can be projected, but using it in an expression is `DS2008`.
    pub arrow_only: Option<arrow_schema::DataType>,
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
    /// The output columns, in order: `project`, and also `project-away` and `project-rename`,
    /// whose items are columns of the input.
    Project(Vec<ProjectItem>),
    /// `extend`: each item adds a column at the end, or replaces the column of that name in
    /// place. An item sees the columns made by the items before it (D-040).
    Extend(Vec<ProjectItem>),
    /// `summarize aggregates by keys`. The output is the keys, then the aggregates. With no
    /// aggregates it is the distinct keys.
    Summarize {
        keys: Vec<ProjectItem>,
        aggregates: Vec<ProjectItem>,
    },
    /// `sort by keys`.
    Sort(Vec<SortKey>),
    /// `top count by key`.
    Top { count: u64, key: SortKey },
    /// `take n`.
    Take(u64),
    /// `count`: one row with one `long` column, `Count`.
    Count,
    /// `distinct`: the distinct values of these input columns (all of them for `distinct *`).
    Distinct(Vec<String>),
    /// `join`.
    Join(Box<Join>),
    /// `union`: the tables appended to the input, in order. Columns are matched by name.
    Union(Vec<Source>),
    /// `getschema`: one row per input column.
    GetSchema,
    /// An operator whose output schema is not computed yet (`render`, M1-10). Reported as not
    /// implemented.
    Unsupported { name: String },
    /// The operator has an error.
    Error,
}

/// One key of `sort` or `top`.
#[derive(Debug, Clone, PartialEq)]
pub struct SortKey {
    pub expr: Expr,
    pub descending: bool,
    /// Whether nulls come first; the default depends on the direction (LANGUAGE.md §3.4).
    pub nulls_first: bool,
}

/// A bound `join`.
#[derive(Debug, Clone, PartialEq)]
pub struct Join {
    pub kind: JoinKind,
    /// The right table.
    pub right: Source,
    /// The conditions, in order.
    pub keys: Vec<JoinKey>,
    /// Right columns renamed because the left has a column of the same name (`DS1007`), as
    /// `(right name, output name)`.
    pub renamed: Vec<(String, String)>,
}

/// The kinds of `join` (LANGUAGE.md §3.2, D-004).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JoinKind {
    Inner,
    LeftOuter,
    RightOuter,
    FullOuter,
    LeftSemi,
    LeftAnti,
    RightSemi,
    RightAnti,
}

impl JoinKind {
    /// The kind as written after `kind =`.
    pub fn from_name(name: &str) -> Option<JoinKind> {
        Some(match name {
            "inner" => JoinKind::Inner,
            "leftouter" => JoinKind::LeftOuter,
            "rightouter" => JoinKind::RightOuter,
            "fullouter" => JoinKind::FullOuter,
            "leftsemi" => JoinKind::LeftSemi,
            "leftanti" => JoinKind::LeftAnti,
            "rightsemi" => JoinKind::RightSemi,
            "rightanti" => JoinKind::RightAnti,
            _ => return None,
        })
    }

    /// The name written after `kind =`.
    pub fn name(self) -> &'static str {
        match self {
            JoinKind::Inner => "inner",
            JoinKind::LeftOuter => "leftouter",
            JoinKind::RightOuter => "rightouter",
            JoinKind::FullOuter => "fullouter",
            JoinKind::LeftSemi => "leftsemi",
            JoinKind::LeftAnti => "leftanti",
            JoinKind::RightSemi => "rightsemi",
            JoinKind::RightAnti => "rightanti",
        }
    }

    /// Semi and anti joins return the columns of one side only.
    pub fn one_side(self) -> Option<JoinSide> {
        match self {
            JoinKind::LeftSemi | JoinKind::LeftAnti => Some(JoinSide::Left),
            JoinKind::RightSemi | JoinKind::RightAnti => Some(JoinSide::Right),
            _ => None,
        }
    }
}

/// A side of a `join`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JoinSide {
    Left,
    Right,
}

/// One equality condition of a `join`.
#[derive(Debug, Clone, PartialEq)]
pub struct JoinKey {
    /// The left column.
    pub left: String,
    /// The right column.
    pub right: String,
    /// The type both columns are compared as (they are widened to it, LANGUAGE.md §4.2).
    pub ty: Type,
    /// `on key`: the two columns become one output column, in the left column's position
    /// (D-019). Its value is the left one, the right one for `rightouter`, and
    /// `coalesce(left, right)` for `fullouter`. `$left.a == $right.b` keeps both.
    pub merged: bool,
    pub span: Span,
}

/// One output column of `project`, `extend` or `summarize`.
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
