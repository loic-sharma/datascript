//! The binder: resolves `let`s, sources, columns and functions, type-checks expressions, and
//! produces the typed [`Hir`].
//!
//! What it does (M1-05):
//!
//! - **Scopes.** Program-level `let`s, scalar or tabular by their syntax (D-036). A name cannot be
//!   defined twice (`DS1003`). In an expression a column wins over a scalar `let` of the same
//!   name (D-038).
//! - **Expressions** are checked against the operator table (LANGUAGE.md §4.1) and the function
//!   catalog, with conversions made explicit as casts.
//! - **Aggregate context** ([`Agg`]): aggregates only in `summarize`'s aggregate list (`DS2004`),
//!   columns there only inside an aggregate (`DS2005`).
//! - **Recovery.** A name that does not resolve gets the error type, which matches anything
//!   silently (DIAGNOSTICS.md rule 8). A node with a syntax error is not bound at all, so a
//!   syntax error adds no semantic errors (ARCHITECTURE.md § Compilation pipeline).
//!
//! Operator schemas and column naming (M1-16) are in [`ops`], `join` in [`join`], and sources
//! and the engine's file formats (M1-17) in [`sources`]. The other capability checks (DS4001)
//! are where the function or `join` kind is bound. `render` (M1-10) is in [`render`].

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use arrow_schema::{Schema, SchemaRef};
use datascript_diagnostics::codes::{
    DS0003, DS1001, DS1002, DS1003, DS1004, DS1006, DS1031, DS2001, DS2004, DS2005, DS2008, DS2010,
    DS2011, DS2012, DS2050, DS4001, DS4004,
};
use datascript_diagnostics::{
    Applicability, Diagnostic, FileId, Span, Suggestion, TextEdit, did_you_mean,
};
use datascript_syntax::ast::{self, AstNode, LiteralKind, Operator, PrefixOp, Stmt};
use datascript_syntax::{SyntaxKind, SyntaxNode, literal};

use crate::Type;
use crate::arg_values::{self, DatetimePart};
use crate::arrow::to_arrow;
use crate::catalog::{CallSite, FunctionCatalog, FunctionKind, ResolveError};
use crate::hir::{
    Column, Expr, ExprKind, Hir, LetBinding, LetValue, Op, Pipeline, Query, Source, Stage,
    Statement,
};

mod join;
mod ops;

/// An argument whose constant value [`Binder::check_constant_arg`] checks.
#[derive(Debug, Clone, Copy)]
enum ConstArg {
    DatetimePart,
    DatetimeFormat,
    Regex,
    Percentage,
}

mod render;
mod sources;

use crate::capabilities::Capabilities;
use crate::ir::{BinaryOp, RenderNumber, Value};
use crate::operators::{self, Operand};
use crate::source::{SchemaLookup, SchemaProvider, SourceRef};
pub(crate) use ops::unique_name;
pub(crate) use render::{property_names, property_takes_columns, property_words};

/// The schema after one pipeline stage (for column completion and hover).
#[derive(Debug, Clone, PartialEq)]
pub struct StageSchema {
    /// The stage: the source or the operator.
    pub span: Span,
    pub schema: SchemaRef,
}

/// The columns an expression can see.
#[derive(Debug, Clone, Copy)]
enum Cols<'c> {
    /// Not in a pipeline (a scalar `let`): there are no rows, so a name must be a `let`.
    NoRows,
    /// In a pipeline whose columns are not known (an error, or a pending source): every name
    /// that is not a `let` silently has the error type.
    Unknown,
    Known(&'c [Column]),
}

impl<'c> Cols<'c> {
    fn of(columns: &'c Option<Vec<Column>>) -> Self {
        match columns {
            Some(columns) => Cols::Known(columns),
            None => Cols::Unknown,
        }
    }
}

/// Whether an expression may call aggregate functions. M1-16 passes the right one for each
/// operator's expressions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Agg {
    /// One value per row: no aggregates. The text says where the expression is, for messages
    /// ("`where`", "`by`", "a `let`").
    Row(&'static str),
    /// An item of `summarize`'s aggregate list: aggregates are allowed, and columns may only be
    /// used inside them.
    Aggregates,
    /// Inside the arguments of this aggregate: no nested aggregates.
    Inside(&'static str),
}

/// Where an expression is bound.
#[derive(Debug, Clone, Copy)]
struct Cx<'c> {
    cols: Cols<'c>,
    agg: Agg,
}

/// What a `let` name stands for.
#[derive(Debug, Clone)]
enum LetEntry {
    Scalar(Expr),
    Table(Arc<Pipeline>),
    Error,
}

pub(crate) struct Binder<'a> {
    file: FileId,
    text: &'a str,
    schemas: &'a dyn SchemaProvider,
    caps: &'a Capabilities,
    catalog: &'static FunctionCatalog,
    /// Nodes that contain a syntax error directly (not inside a smaller expression). They are
    /// not bound, and their value has the error type.
    broken: HashSet<SyntaxNode>,
    lets: HashMap<String, (Span, LetEntry)>,
    /// Whether the current `summarize` item already has a `DS2005`.
    ds2005_reported: bool,
    pub(crate) diagnostics: Vec<Diagnostic>,
    pub(crate) pending: BTreeSet<SourceRef>,
    pub(crate) stage_schemas: Vec<StageSchema>,
    /// The pipeline of the query statement being bound: the only place `render` may be
    /// (DS5008).
    query_pipeline: Option<SyntaxNode>,
}

impl<'a> Binder<'a> {
    /// A binder for `program`, whose syntax and lexical errors are `syntax_errors`.
    pub(crate) fn new(
        file: FileId,
        text: &'a str,
        schemas: &'a dyn SchemaProvider,
        caps: &'a Capabilities,
        program: &ast::Program,
        syntax_errors: &[Diagnostic],
    ) -> Self {
        Binder {
            file,
            text,
            schemas,
            caps,
            catalog: FunctionCatalog::builtin(),
            broken: broken_nodes(program.syntax(), file, syntax_errors),
            lets: HashMap::new(),
            ds2005_reported: false,
            diagnostics: Vec::new(),
            pending: BTreeSet::new(),
            stage_schemas: Vec::new(),
            query_pipeline: None,
        }
    }

    pub(crate) fn bind_program(&mut self, program: &ast::Program) -> Hir {
        let mut statements = Vec::new();
        for stmt in program.statements() {
            match stmt {
                Stmt::Let(stmt) => {
                    if let Some(binding) = self.bind_let(&stmt) {
                        statements.push(Statement::Let(binding));
                    }
                }
                Stmt::Query(pipeline) => {
                    let span = self.span(&pipeline);
                    self.query_pipeline = Some(pipeline.syntax().clone());
                    let pipeline = self.bind_pipeline(&pipeline);
                    statements.push(Statement::Query(Query { span, pipeline }));
                }
            }
        }
        Hir { statements }
    }

    // --- `let` ---

    fn bind_let(&mut self, stmt: &ast::LetStmt) -> Option<LetBinding> {
        let name_node = stmt.name()?;
        let name = name_node.text()?;
        let name_span = self.span(&name_node);
        let value = if self.broken.contains(stmt.syntax()) {
            LetValue::Error
        } else if stmt.is_tabular() {
            let mut value = stmt.value();
            while let Some(ast::Expr::Paren(paren)) = &value {
                value = paren.inner();
            }
            match value {
                Some(ast::Expr::Pipeline(pipeline)) => {
                    LetValue::Table(self.bind_pipeline(&pipeline))
                }
                _ => LetValue::Error,
            }
        } else {
            match stmt.value() {
                Some(value) => self.bind_scalar_let(&name, &value),
                None => LetValue::Error,
            }
        };

        if let Some((first, _)) = self.lets.get(&name) {
            let first = *first;
            self.diagnostics.push(
                Diagnostic::error(DS1003, format!("`{name}` is already defined"))
                    .primary(name_span, "defined again here")
                    .secondary(first, "first defined here")
                    .help(format!(
                        "a `let` name cannot be reused; give this one another name, such as \
                         `{name}2`"
                    )),
            );
            return None;
        }
        let entry = match &value {
            LetValue::Scalar(expr) => LetEntry::Scalar(expr.clone()),
            LetValue::Table(pipeline) => LetEntry::Table(pipeline.clone()),
            LetValue::Error => LetEntry::Error,
        };
        self.lets.insert(name.clone(), (name_span, entry));
        Some(LetBinding {
            name,
            name_span,
            value,
        })
    }

    fn bind_scalar_let(&mut self, let_name: &str, value: &ast::Expr) -> LetValue {
        // `let alias = orders;` where `orders` is a table (D-036).
        if let ast::Expr::NameRef(name_ref) = value
            && let Some(name) = name_ref.text()
            && let Some((_, LetEntry::Table(_))) = self.lets.get(&name)
        {
            let span = self.span(value);
            self.table_in_scalar_let(let_name, &name, span);
            return LetValue::Error;
        }
        // `let alias = orders;` where `orders` is a host table.
        if let ast::Expr::NameRef(name_ref) = value
            && let Some(name) = name_ref.text()
            && !self.lets.contains_key(&name)
        {
            let span = self.span(value);
            let source = SourceRef::Table(name.clone());
            match self.schemas.schema_of(&source) {
                SchemaLookup::Ready(_) => {
                    self.table_in_scalar_let(let_name, &name, span);
                    return LetValue::Error;
                }
                // Known on the next pass; until then the name has the error type silently.
                SchemaLookup::Pending => {
                    self.pending.insert(source);
                    return LetValue::Error;
                }
                SchemaLookup::Failed(_) => {}
            }
        }
        let cx = Cx {
            cols: Cols::NoRows,
            agg: Agg::Row("a `let`"),
        };
        let expr = self.expr(value, &cx);
        let expr = self.value_sink(expr);
        if expr.ty.is_error() {
            LetValue::Error
        } else {
            LetValue::Scalar(expr)
        }
    }

    /// `DS1006`: `let alias = orders;` where `orders` is a table (D-036).
    fn table_in_scalar_let(&mut self, let_name: &str, name: &str, span: Span) {
        self.diagnostics.push(
            Diagnostic::error(DS1006, format!("`{name}` is a table, not a value"))
                .primary(span, "this names a table")
                .help_with(
                    format!(
                        "a `let` is a table only when its value is a pipeline: \
                         `let {let_name} = from {};`",
                        &self.text[span.range()]
                    ),
                    Suggestion::new(
                        vec![TextEdit::new(Span::empty(self.file, span.start), "from ")],
                        Applicability::MachineApplicable,
                    ),
                ),
        );
    }

    // --- Pipelines ---

    fn bind_pipeline(&mut self, pipeline: &ast::Pipeline) -> Arc<Pipeline> {
        let span = self.span(pipeline);
        let (source, mut columns) = match pipeline.source() {
            Some(source) => self.bind_source(&source),
            None => (Source::Error { span }, None),
        };
        let mut stages = Vec::new();
        for op in pipeline.operators() {
            let op_span = self.span(&op);
            let (bound, new_columns) = self.bind_operator(&op, &columns);
            columns = new_columns;
            if let Some(columns) = &columns {
                self.stage_schemas.push(StageSchema {
                    span: op_span,
                    schema: schema_of(columns),
                });
            }
            stages.push(Stage {
                span: op_span,
                op: bound,
                columns: columns.clone(),
            });
        }
        Arc::new(Pipeline {
            span,
            source,
            stages,
            columns,
        })
    }

    // --- Operators ---

    /// Binds one operator over `columns`, returning it and the columns after it.
    fn bind_operator(
        &mut self,
        op: &Operator,
        columns: &Option<Vec<Column>>,
    ) -> (Op, Option<Vec<Column>>) {
        if self.broken.contains(op.syntax()) {
            let after = match op {
                Operator::Where(_)
                | Operator::Take(_)
                | Operator::Sort(_)
                | Operator::Top(_)
                | Operator::Render(_) => columns.clone(),
                Operator::Count(_) => Some(Self::count_columns()),
                Operator::GetSchema(_) => Some(Self::getschema_columns()),
                _ => None,
            };
            return (Op::Error, after);
        }
        let row = |place| Cx {
            cols: Cols::of(columns),
            agg: Agg::Row(place),
        };
        match op {
            Operator::Where(op) => {
                let Some(predicate) = op.predicate() else {
                    return (Op::Error, columns.clone());
                };
                let expr = self.expr(&predicate, &row("`where`"));
                let expr = self.predicate(expr, &row("`where`"));
                (Op::Where(expr), columns.clone())
            }
            Operator::Project(op) => self.bind_project(op, columns),
            Operator::ProjectAway(op) => self.bind_project_away(op, columns),
            Operator::ProjectRename(op) => self.bind_project_rename(op, columns),
            Operator::Extend(op) => self.bind_extend(op, columns),
            Operator::Summarize(op) => self.bind_summarize(op, columns),
            Operator::Sort(op) => {
                let keys: Vec<_> = op
                    .keys()
                    .filter_map(|key| self.bind_sort_key(&key, columns, "`sort`", false))
                    .collect();
                (Op::Sort(keys), columns.clone())
            }
            Operator::Top(op) => {
                let count = op
                    .count()
                    .and_then(|count| self.take_count(&count, "top", columns));
                let key = op
                    .key()
                    .and_then(|key| self.bind_sort_key(&key, columns, "`top`", true));
                match (count, key) {
                    (Some(count), Some(key)) => (Op::Top { count, key }, columns.clone()),
                    _ => (Op::Error, columns.clone()),
                }
            }
            Operator::Take(op) => {
                let keyword = op
                    .syntax()
                    .first_token()
                    .map_or_else(|| "take".to_string(), |t| t.text().to_string());
                let count = op
                    .count()
                    .and_then(|count| self.take_count(&count, &keyword, columns));
                match count {
                    Some(count) => (Op::Take(count), columns.clone()),
                    None => (Op::Error, columns.clone()),
                }
            }
            Operator::Count(_) => (Op::Count, Some(Self::count_columns())),
            Operator::Distinct(op) => self.bind_distinct(op, columns),
            Operator::Join(op) => self.bind_join(op, columns),
            Operator::Union(op) => self.bind_union(op, columns),
            Operator::GetSchema(_) => (Op::GetSchema, Some(Self::getschema_columns())),
            Operator::Render(op) => self.bind_render(op, columns),
        }
    }

    /// The row count of `take` or `top`: a constant whole number of 0 or more, written as a
    /// literal or as a scalar `let` bound to one (`DS2050`).
    fn take_count(
        &mut self,
        count: &ast::Expr,
        op: &str,
        columns: &Option<Vec<Column>>,
    ) -> Option<u64> {
        if self.broken.contains(count.syntax()) {
            return None;
        }
        let span = self.span(count);
        let expr = self.const_value_of(count).ok()?;
        let (message, label) = match render::const_number(&expr) {
            Some(RenderNumber::Long(n)) => match u64::try_from(n) {
                Ok(n) => return Some(n),
                Err(_) => (
                    format!("the row count of `{op}` cannot be negative"),
                    format!("this is {n}"),
                ),
            },
            Some(RenderNumber::Real(_)) => (
                format!("the row count of `{op}` must be a whole number"),
                "this is `real`".to_string(),
            ),
            None => match &expr.kind {
                ExprKind::Column(word) if expr.ty.is_error() => {
                    let is_column = columns
                        .as_ref()
                        .is_some_and(|cols| cols.iter().any(|c| &c.name == word));
                    let label = if is_column {
                        format!("`{word}` is a column, which can differ from row to row")
                    } else {
                        format!("`{word}` is not a constant")
                    };
                    (format!("the row count of `{op}` must be a constant"), label)
                }
                _ if expr.ty == Type::Long || expr.ty == Type::Int => (
                    format!("the row count of `{op}` must be a constant"),
                    "this is computed".to_string(),
                ),
                _ => (
                    format!("the row count of `{op}` must be a whole number"),
                    format!("this is `{}`", expr.ty),
                ),
            },
        };
        let example = if op == "top" {
            "top 10 by x".to_string()
        } else {
            format!("{op} 10")
        };
        let mut d = Diagnostic::error(DS2050, message)
            .primary(span, label)
            .help(format!(
                "write a whole number of 0 or more, such as `{example}`, or the name of a `let` \
                 bound to one"
            ));
        if let ast::Expr::NameRef(name_ref) = count
            && let Some(name) = name_ref.text()
            && matches!(self.lets.get(&name), Some((_, LetEntry::Scalar(_))))
        {
            d = d.note(format!("`{name}` is defined by a `let`"));
        }
        self.diagnostics.push(d);
        None
    }

    /// Checks the constant arguments of a call whose values the language restricts
    /// (`arg_values`), so a bad one is `DS4004` at check time rather than when the query runs.
    fn check_constant_args(&mut self, function: &str, args: &[Expr]) {
        let checks: &[(usize, ConstArg)] = match function {
            "datetime_part" | "datetime_diff" => &[(0, ConstArg::DatetimePart)],
            "format_datetime" => &[(1, ConstArg::DatetimeFormat)],
            "trim" | "extract" => &[(0, ConstArg::Regex)],
            "percentile" => &[(1, ConstArg::Percentage)],
            _ => &[],
        };
        for &(index, kind) in checks {
            if let Some(arg) = args.get(index) {
                self.check_constant_arg(arg, kind);
            }
        }
    }

    fn check_constant_arg(&mut self, arg: &Expr, kind: ConstArg) {
        let string = || match sources::const_value(arg) {
            Some(Value::String(s)) => Some(s.as_str()),
            _ => None,
        };
        let (result, label) = match kind {
            ConstArg::DatetimePart => match string() {
                Some(s) => (DatetimePart::parse(s).map(|_| ()), "not one of the parts"),
                None => return,
            },
            ConstArg::DatetimeFormat => match string() {
                Some(s) => (arg_values::check_datetime_format(s), "in this format"),
                None => return,
            },
            ConstArg::Regex => match string() {
                Some(s) => (arg_values::check_regex(s), "this pattern does not compile"),
                None => return,
            },
            ConstArg::Percentage => {
                let p = match render::const_number(arg) {
                    Some(RenderNumber::Long(n)) => n as f64,
                    Some(RenderNumber::Real(x)) => x,
                    None => return,
                };
                (arg_values::check_percentage(p), "must be from 0 to 100")
            }
        };
        if let Err(e) = result {
            let mut d = Diagnostic::error(DS4004, e.message).primary(arg.span, label);
            if let (ConstArg::DatetimePart, ExprKind::Literal(Value::String(s))) = (kind, &arg.kind)
                && let Some(best) =
                    did_you_mean(s, arg_values::DATETIME_PARTS.iter().copied()).single()
            {
                d = d.help_with(
                    format!("did you mean `'{best}'`?"),
                    Suggestion::replace(arg.span, format!("'{best}'")),
                );
            }
            self.diagnostics.push(d.note(e.note));
        }
    }

    /// Binds one item of `summarize`'s aggregate list: it must aggregate (`DS2005`).
    fn aggregate_item(&mut self, expr: &ast::Expr, cols: Cols<'_>) -> Expr {
        self.ds2005_reported = false;
        let cx = Cx {
            cols,
            agg: Agg::Aggregates,
        };
        let bound = self.expr(expr, &cx);
        let bound = self.value_sink(bound);
        let catalog = self.catalog;
        let aggregates = bound.any(&|e| match &e.kind {
            ExprKind::Call { function, .. } => catalog
                .lookup(function)
                .is_ok_and(|f| f.kind == FunctionKind::Aggregate),
            _ => false,
        });
        if !aggregates && !self.ds2005_reported && !bound.ty.is_error() {
            self.diagnostics.push(
                Diagnostic::error(DS2005, "this `summarize` item does not aggregate anything")
                    .primary(bound.span, "not an aggregate")
                    .help(
                        "use an aggregate function, such as `count()` or `sum(amount)`, or \
                         move the expression after `by` to group by it",
                    ),
            );
        }
        bound
    }

    /// Checks that a bound predicate is `bool` (`DS2001`).
    fn predicate(&mut self, expr: Expr, cx: &Cx<'_>) -> Expr {
        match expr.ty {
            Type::Bool | Type::Error => expr,
            Type::Null => convert(expr, Type::Bool),
            ty => {
                let Agg::Row(place) = cx.agg else {
                    return Expr::error(expr.span);
                };
                let text = &self.text[expr.span.range()];
                let mut d = Diagnostic::error(
                    DS2001,
                    format!("{place} needs a `bool` condition, found `{ty}`"),
                )
                .primary(expr.span, format!("this is `{ty}`"));
                if ty.is_numeric() {
                    d = d.help(format!(
                        "to keep the rows where it is not zero, compare it: `{text} != 0`"
                    ));
                } else if ty == Type::String {
                    d = d.help(format!(
                        "to keep the rows where it is not empty, use `isnotempty({text})`"
                    ));
                }
                self.annotate(&mut d, &expr, cx);
                self.diagnostics.push(d);
                Expr::error(expr.span)
            }
        }
    }

    /// A value that must have a type of its own: an untyped `null` there is `DS2009`.
    fn value_sink(&mut self, expr: Expr) -> Expr {
        if expr.ty == Type::Null {
            self.diagnostics.push(operators::untyped_null(expr.span));
            return Expr::error(expr.span);
        }
        expr
    }

    // --- Expressions ---

    fn expr(&mut self, expr: &ast::Expr, cx: &Cx<'_>) -> Expr {
        let span = self.span(expr);
        if self.broken.contains(expr.syntax()) {
            return Expr::error(span);
        }
        match expr {
            ast::Expr::Paren(paren) => match paren.inner() {
                Some(ast::Expr::Pipeline(_)) => self.table_as_value(span, cx),
                Some(inner) => {
                    let mut inner = self.expr(&inner, cx);
                    inner.span = span;
                    inner
                }
                None => Expr::error(span),
            },
            ast::Expr::Pipeline(_) => self.table_as_value(span, cx),
            ast::Expr::NameRef(name_ref) => self.name_ref(name_ref, span, cx),
            ast::Expr::Literal(lit) => self.literal(lit, span),
            ast::Expr::TypedNull(null) => {
                match null.type_token().and_then(|t| Type::from_name(t.text())) {
                    Some(ty) => Expr {
                        kind: ExprKind::Literal(Value::Null),
                        ty,
                        span,
                    },
                    None => Expr::error(span),
                }
            }
            ast::Expr::Prefix(prefix) => self.prefix(prefix, span, cx),
            ast::Expr::Bin(bin) => self.binary(bin, span, cx),
            ast::Expr::Call(call) => self.call(call, span, cx),
            ast::Expr::In(in_expr) => self.in_list(in_expr, span, cx),
            ast::Expr::Between(between) => self.between(between, span, cx),
            ast::Expr::Field(field) => {
                let base = field.base().map(|b| self.expr(&b, cx));
                self.needs_dynamic(base, span, "field access")
            }
            ast::Expr::Index(index) => {
                let base = index.base().map(|b| self.expr(&b, cx));
                if let Some(i) = index.index() {
                    self.expr(&i, cx);
                }
                self.needs_dynamic(base, span, "indexing")
            }
        }
    }

    fn name_ref(&mut self, name_ref: &ast::NameRef, span: Span, cx: &Cx<'_>) -> Expr {
        let Some(name) = name_ref.text() else {
            return Expr::error(span);
        };
        if let Some(token) = name_ref.token()
            && matches!(
                token.kind(),
                SyntaxKind::DOLLAR_LEFT | SyntaxKind::DOLLAR_RIGHT
            )
        {
            self.diagnostics.push(
                Diagnostic::error(
                    DS1031,
                    format!("`{name}` can only be used in a `join` condition"),
                )
                .primary(span, "not in a `join` condition")
                .help(format!(
                    "`{name}.column` names a column of one side of a `join`, in `on \
                     $left.a == $right.b`; elsewhere, use the column name"
                )),
            );
            return Expr::error(span);
        }
        if let Cols::Known(columns) = cx.cols
            && let Some(column) = columns.iter().find(|c| c.name == name)
        {
            if let Some(arrow) = &column.arrow_only {
                let mut d = Diagnostic::error(
                    DS2008,
                    format!("column `{name}` has a type DataScript cannot use yet"),
                )
                .primary(span, format!("this column is `{arrow}` in Arrow"))
                .help(format!(
                    "it can only be projected as it is: `project {name}`, `project-away {name}`"
                ));
                if let Some(source) = &column.read_from {
                    d = d.note(format!("column `{name}` was read from {source}"));
                }
                self.diagnostics.push(d.note(
                    "nested, JSON and GUID columns become `dynamic` and `guid` values in M2",
                ));
                return Expr::error(span);
            }
            if column.ty.is_error() {
                return Expr::error(span);
            }
            if cx.agg == Agg::Aggregates && !self.ds2005_reported {
                self.ds2005_reported = true;
                self.diagnostics.push(
                    Diagnostic::error(DS2005, format!("column `{name}` is not aggregated"))
                        .primary(span, "this has a value on each row")
                        .help(format!(
                            "use it inside an aggregate, such as `sum({name})`, or group by \
                             it: `by {name}`"
                        )),
                );
                return Expr::error(span);
            }
            return Expr {
                kind: ExprKind::Column(name),
                ty: column.ty,
                span,
            };
        }
        match self.lets.get(&name) {
            Some((_, LetEntry::Scalar(value))) => {
                return Expr {
                    ty: value.ty,
                    kind: ExprKind::Let {
                        name,
                        value: Box::new(value.clone()),
                    },
                    span,
                };
            }
            Some((_, LetEntry::Table(_))) => {
                self.diagnostics.push(
                    Diagnostic::error(DS1006, format!("`{name}` is a table, not a value"))
                        .primary(span, "this names a table")
                        .note("a table can be read with `from`, or joined with `join`"),
                );
                return Expr::error(span);
            }
            Some((_, LetEntry::Error)) => return Expr::error(span),
            None => {}
        }
        match cx.cols {
            Cols::Unknown => {}
            Cols::Known(columns) => self.unknown_column(span, &name, columns),
            Cols::NoRows => self.unknown_name(span, &name),
        }
        Expr::error(span)
    }

    /// `DS2010`: a pipeline where a value is expected.
    fn table_as_value(&mut self, span: Span, cx: &Cx<'_>) -> Expr {
        let mut d = Diagnostic::error(DS2010, "a table cannot be used as a value")
            .primary(span, "this is a table");
        if matches!(cx.cols, Cols::NoRows) {
            d = d.help(
                "a `let` defines a table only when the pipeline is its whole value: \
                 `let t = from ...;`",
            );
        }
        self.diagnostics.push(d);
        Expr::error(span)
    }

    /// `DS2012`: field access or indexing, which needs `dynamic` (M2).
    fn needs_dynamic(&mut self, base: Option<Expr>, span: Span, what: &str) -> Expr {
        let Some(base) = base else {
            return Expr::error(span);
        };
        if !base.ty.is_error() {
            self.diagnostics.push(
                Diagnostic::error(DS2012, format!("{what} needs a `dynamic` value"))
                    .primary(base.span, format!("this is `{}`", base.ty))
                    .note("`dynamic` values are not available yet; they are planned for M2"),
            );
        }
        Expr::error(span)
    }

    fn literal(&mut self, lit: &ast::Literal, span: Span) -> Expr {
        let (Some(kind), Some(token)) = (lit.kind(), lit.token()) else {
            return Expr::error(span);
        };
        let text = token.text();
        // Literal errors were reported by the lexer, so they give the error type silently.
        let (value, ty) = match kind {
            LiteralKind::Long => match literal::long(text, span) {
                Ok(n) => match i64::try_from(n) {
                    Ok(n) => (Value::Long(n), Type::Long),
                    Err(_) => {
                        // 2^63 is only valid after a unary minus, which folds it.
                        self.long_too_large(span, text);
                        return Expr::error(span);
                    }
                },
                Err(_) => return Expr::error(span),
            },
            LiteralKind::Real => match literal::real(text, span) {
                Ok(x) => (Value::Real(x), Type::Real),
                Err(_) => return Expr::error(span),
            },
            LiteralKind::String => match literal::string(text, span) {
                Ok(s) => (Value::String(s), Type::String),
                Err(_) => return Expr::error(span),
            },
            LiteralKind::Bool => (Value::Bool(token.kind() == SyntaxKind::TRUE_KW), Type::Bool),
            LiteralKind::Timespan => match literal::timespan(text, span) {
                Ok(Some(us)) => (Value::TimeSpan(us), Type::TimeSpan),
                Ok(None) => (Value::Null, Type::TimeSpan),
                Err(_) => return Expr::error(span),
            },
            LiteralKind::Datetime => match literal::datetime(text, span) {
                Ok(Some(us)) => (Value::DateTime(us), Type::DateTime),
                Ok(None) => (Value::Null, Type::DateTime),
                Err(_) => return Expr::error(span),
            },
            LiteralKind::Null => (Value::Null, Type::Null),
        };
        Expr {
            kind: ExprKind::Literal(value),
            ty,
            span,
        }
    }

    fn long_too_large(&mut self, span: Span, text: &str) {
        let digits: String = text.chars().filter(|c| *c != '_').collect();
        self.diagnostics.push(
            Diagnostic::error(DS0003, "number is too large for a `long`")
                .primary(span, "the largest `long` is 9223372036854775807")
                .help(format!(
                    "for numbers this large, use a `real` by adding a fraction: `{digits}.0`"
                )),
        );
    }

    fn prefix(&mut self, prefix: &ast::PrefixExpr, span: Span, cx: &Cx<'_>) -> Expr {
        let (Some(op), Some(operand)) = (prefix.op(), prefix.operand()) else {
            return Expr::error(span);
        };
        // `-9223372036854775808`: fold the minus into the literal (LANGUAGE.md §1).
        if op == PrefixOp::Neg
            && let ast::Expr::Literal(lit) = &operand
            && lit.kind() == Some(LiteralKind::Long)
            && !self.broken.contains(operand.syntax())
            && let Some(token) = lit.token()
        {
            return match literal::long(token.text(), span) {
                Ok(n) => Expr {
                    kind: ExprKind::Literal(Value::Long(0i64.wrapping_sub_unsigned(n))),
                    ty: Type::Long,
                    span,
                },
                Err(_) => Expr::error(span),
            };
        }
        let operand_expr = self.expr(&operand, cx);
        match operators::unary(op, operand_expr.ty) {
            Ok(typing) => Expr {
                kind: ExprKind::Unary {
                    op,
                    operand: Box::new(convert(operand_expr, typing.operand)),
                },
                ty: typing.result,
                span,
            },
            Err(error) => {
                let mut d = error.diagnostic(self.operand(operand_expr.span));
                self.annotate(&mut d, &operand_expr, cx);
                self.diagnostics.push(d);
                Expr::error(span)
            }
        }
    }

    fn binary(&mut self, bin: &ast::BinExpr, span: Span, cx: &Cx<'_>) -> Expr {
        let (Some(op), Some(lhs), Some(rhs)) = (bin.op(), bin.lhs(), bin.rhs()) else {
            return Expr::error(span);
        };
        let left = self.expr(&lhs, cx);
        let right = self.expr(&rhs, cx);
        self.typed_binary(op, left, right, span, cx)
    }

    fn typed_binary(
        &mut self,
        op: BinaryOp,
        left: Expr,
        right: Expr,
        span: Span,
        cx: &Cx<'_>,
    ) -> Expr {
        match operators::binary(op, left.ty, right.ty) {
            Ok(typing) if typing.result.is_error() => Expr::error(span),
            Ok(typing) => {
                if op == BinaryOp::MatchesRegex {
                    self.check_constant_arg(&right, ConstArg::Regex);
                }
                Expr {
                    kind: ExprKind::Binary {
                        op,
                        left: Box::new(convert(left, typing.left)),
                        right: Box::new(convert(right, typing.right)),
                    },
                    ty: typing.result,
                    span,
                }
            }
            Err(error) => {
                let mut d = error.diagnostic(self.operand(left.span), self.operand(right.span));
                self.annotate(&mut d, &left, cx);
                self.annotate(&mut d, &right, cx);
                self.diagnostics.push(d);
                Expr::error(span)
            }
        }
    }

    fn call(&mut self, call: &ast::CallExpr, span: Span, cx: &Cx<'_>) -> Expr {
        let Some(callee) = call.callee() else {
            return Expr::error(span);
        };
        let Some(name) = callee.text() else {
            return Expr::error(span);
        };
        let callee_span = self.span(&callee);
        let function = match self.catalog.lookup(&name) {
            Ok(function) => function,
            Err(unknown) => {
                // Check the arguments anyway: their mistakes are independent.
                for arg in call.args() {
                    let expr = match arg {
                        ast::Arg::Positional(expr) => Some(expr),
                        ast::Arg::Named(named) => named.expr(),
                    };
                    if let Some(expr) = expr {
                        self.expr(&expr, cx);
                    }
                }
                self.unknown_function(callee_span, &name, &unknown);
                return Expr::error(span);
            }
        };

        let aggregate = function.kind == FunctionKind::Aggregate;
        let arg_cx = if aggregate {
            match cx.agg {
                Agg::Aggregates => Cx {
                    cols: cx.cols,
                    agg: Agg::Inside(function.name),
                },
                Agg::Row(place) => {
                    let call_text = self.text[span.range()].to_string();
                    self.aggregate_misplaced(callee_span, &call_text, function.name, place);
                    // Check the arguments as if the aggregate were allowed.
                    Cx {
                        cols: cx.cols,
                        agg: Agg::Inside(function.name),
                    }
                }
                Agg::Inside(outer) => {
                    self.diagnostics.push(
                        Diagnostic::error(
                            DS2004,
                            format!(
                                "aggregate function `{}` cannot be used inside another \
                                 aggregate",
                                function.name
                            ),
                        )
                        .primary(callee_span, format!("inside `{outer}`"))
                        .help("aggregate once, or compute the inner value in another `summarize`"),
                    );
                    *cx
                }
            }
        } else {
            *cx
        };

        let mut args = Vec::new();
        let mut named_reported = false;
        for arg in call.args() {
            let expr = match arg {
                ast::Arg::Positional(expr) => Some(expr),
                ast::Arg::Named(named) => {
                    if !named_reported {
                        named_reported = true;
                        let named_span = self.span(&named);
                        let mut d = Diagnostic::error(
                            DS2011,
                            format!("`{}` takes no named arguments", function.name),
                        )
                        .primary(named_span, "named argument");
                        if let Some(value) = named.expr() {
                            let value = &self.text[self.span(&value).range()];
                            let call_text = &self.text[span.range()];
                            let (start, end) = (
                                (named_span.start - span.start) as usize,
                                (named_span.end - span.start) as usize,
                            );
                            let rewrite =
                                format!("{}{value}{}", &call_text[..start], &call_text[end..]);
                            d = d.help(format!("pass the value by position: `{rewrite}`"));
                        }
                        self.diagnostics.push(d.note(signatures_note(function)));
                    }
                    named.expr()
                }
            };
            match expr {
                Some(expr) => args.push(self.expr(&expr, &arg_cx)),
                None => args.push(Expr::error(span)),
            }
        }
        if named_reported {
            return Expr::error(span);
        }

        let types: Vec<Type> = args.iter().map(|a| a.ty).collect();
        if !self.caps.supports_function(self.catalog, function.name) {
            self.diagnostics.push(
                Diagnostic::error(
                    DS4001,
                    format!(
                        "function `{}` is not supported by this engine",
                        function.name
                    ),
                )
                .primary(callee_span, "this engine does not implement it")
                .note(
                    "the function is part of DataScript, but the engine that runs this program \
                     cannot run it",
                ),
            );
        }
        match function.resolve(&types) {
            Ok(resolved) => {
                if resolved.return_type.is_error() {
                    return Expr::error(span);
                }
                self.check_constant_args(function.name, &args);
                if resolved.return_type.is_null() {
                    // Nothing gives the result a type: `abs(null)`, `iff(p, null, null)`.
                    let at = args
                        .iter()
                        .find(|a| a.ty.is_null())
                        .map_or(span, |a| a.span);
                    self.diagnostics.push(operators::untyped_null(at));
                    return Expr::error(span);
                }
                let args = args
                    .into_iter()
                    .zip(&resolved.arg_types)
                    .map(|(arg, ty)| convert(arg, *ty))
                    .collect();
                Expr {
                    kind: ExprKind::Call {
                        function: function.name,
                        args,
                    },
                    ty: resolved.return_type,
                    span,
                }
            }
            Err(error) => {
                let operands: Vec<Operand<'_>> = args
                    .iter()
                    .map(|a| Operand {
                        span: a.span,
                        text: &self.text[a.span.range()],
                    })
                    .collect();
                let site = CallSite {
                    span,
                    args: &operands,
                };
                let mut d = match &error {
                    ResolveError::Arity(e) => e.diagnostic(function, site),
                    ResolveError::ArgType(e) => e.diagnostic(function, site),
                };
                for arg in &args {
                    self.annotate(&mut d, arg, cx);
                }
                self.diagnostics.push(d);
                Expr::error(span)
            }
        }
    }

    fn in_list(&mut self, in_expr: &ast::InExpr, span: Span, cx: &Cx<'_>) -> Expr {
        let (Some(lhs), Some(list)) = (in_expr.lhs(), in_expr.list()) else {
            return Expr::error(span);
        };
        let negated = in_expr.negated();
        let case_insensitive = in_expr.case_insensitive();
        let expr = self.expr(&lhs, cx);
        let mut items = Vec::new();
        for arg in list.args() {
            match arg {
                ast::Arg::Positional(item) => {
                    let item_span = self.span(&item);
                    if self.is_pipeline(&item) {
                        self.diagnostics.push(
                            Diagnostic::error(DS2010, "a table cannot be used as a value")
                                .primary(item_span, "this is a table")
                                .note(
                                    "`in` with a table is not available yet; it is planned for M2",
                                ),
                        );
                        items.push(Expr::error(item_span));
                    } else {
                        items.push(self.expr(&item, cx));
                    }
                }
                // The parser reports `=` in a list; the value still counts.
                ast::Arg::Named(named) => match named.expr() {
                    Some(item) => items.push(self.expr(&item, cx)),
                    None => items.push(Expr::error(span)),
                },
            }
        }
        // Every item is compared with the left side: check each pair, then convert all of them
        // to one type.
        let op = if case_insensitive {
            BinaryOp::EqCi
        } else {
            BinaryOp::Eq
        };
        let mut ty = expr.ty;
        for item in &items {
            if let Err(error) = operators::binary(op, expr.ty, item.ty) {
                let mut d = error.diagnostic(self.operand(expr.span), self.operand(item.span));
                self.annotate(&mut d, &expr, cx);
                self.diagnostics.push(d);
                return Expr::error(span);
            }
            ty = ty.common(item.ty).unwrap_or(Type::Error);
        }
        if ty.is_error() || expr.ty.is_error() || items.iter().any(|i| i.ty.is_error()) {
            return Expr {
                kind: ExprKind::Error,
                ty: Type::Bool,
                span,
            };
        }
        if ty.is_null() {
            // `null in (null)`: compare as nothing in particular; the result is null.
            ty = Type::Bool;
        }
        Expr {
            kind: ExprKind::In {
                expr: Box::new(convert(expr, ty)),
                list: items.into_iter().map(|i| convert(i, ty)).collect(),
                negated,
                case_insensitive,
            },
            ty: Type::Bool,
            span,
        }
    }

    fn between(&mut self, between: &ast::BetweenExpr, span: Span, cx: &Cx<'_>) -> Expr {
        let (Some(lhs), Some(low), Some(high)) = (between.lhs(), between.low(), between.high())
        else {
            return Expr::error(span);
        };
        let expr = self.expr(&lhs, cx);
        let low = self.expr(&low, cx);
        let high = self.expr(&high, cx);
        for (op, bound) in [(BinaryOp::GtEq, &low), (BinaryOp::LtEq, &high)] {
            if let Err(error) = operators::binary(op, expr.ty, bound.ty) {
                let mut d = error.diagnostic(self.operand(expr.span), self.operand(bound.span));
                self.annotate(&mut d, &expr, cx);
                self.diagnostics.push(d);
                return Expr::error(span);
            }
        }
        let ty = expr
            .ty
            .common(low.ty)
            .and_then(|t| t.common(high.ty))
            .unwrap_or(Type::Error);
        if ty.is_error() {
            return Expr {
                kind: ExprKind::Error,
                ty: Type::Bool,
                span,
            };
        }
        let ty = if ty.is_null() { Type::Bool } else { ty };
        Expr {
            kind: ExprKind::Between {
                expr: Box::new(convert(expr, ty)),
                low: Box::new(convert(low, ty)),
                high: Box::new(convert(high, ty)),
                negated: between.negated(),
            },
            ty: Type::Bool,
            span,
        }
    }

    // --- Diagnostics ---

    fn aggregate_misplaced(&mut self, span: Span, call: &str, name: &str, place: &str) {
        let mut d = Diagnostic::error(
            DS2004,
            format!("aggregate function `{name}` cannot be used in {place}"),
        )
        .primary(span, "this combines many rows into one value");
        d = match place {
            "`by`" => d.help(
                "`by` lists the grouping keys; put the aggregate before `by`: \
                 `summarize total = sum(amount) by key`",
            ),
            _ => d.help(format!(
                "aggregate in `summarize` first, then use the result: \
                 `| summarize result = {call} by ...`"
            )),
        };
        self.diagnostics.push(d);
    }

    fn unknown_column(&mut self, span: Span, name: &str, columns: &[Column]) {
        let column_names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
        let lets = self.scalar_let_names();
        let candidates = column_names
            .iter()
            .copied()
            .chain(lets.iter().map(String::as_str));
        let suggestions = did_you_mean(name, candidates);
        let mut d = Diagnostic::error(DS1001, format!("unknown column `{name}`"))
            .primary(span, format!("no column named `{name}` here"));
        if let Some(best) = suggestions.single() {
            let what = if column_names.contains(&best) {
                "a column"
            } else {
                "a `let`"
            };
            d = d.help_with(
                format!("{what} with a similar name exists: `{best}`"),
                Suggestion::replace(span, best),
            );
        } else if !suggestions.is_empty() {
            let list: Vec<String> = suggestions
                .names()
                .iter()
                .map(|n| format!("`{n}`"))
                .collect();
            d = d.help(format!("similar names exist: {}", list.join(", ")));
        }
        let available = if column_names.is_empty() {
            "there are no columns at this point".to_string()
        } else {
            format!(
                "columns available at this point: {}",
                column_names.join(", ")
            )
        };
        self.diagnostics.push(d.note(available));
    }

    fn unknown_name(&mut self, span: Span, name: &str) {
        let lets = self.scalar_let_names();
        let suggestions = did_you_mean(name, lets.iter().map(String::as_str));
        let mut d = Diagnostic::error(DS1002, format!("unknown name `{name}`"))
            .primary(span, "no `let` with this name is defined before here");
        if let Some(best) = suggestions.single() {
            d = d.help_with(
                format!("a `let` with a similar name exists: `{best}`"),
                Suggestion::replace(span, best),
            );
        } else if !suggestions.is_empty() {
            let list: Vec<String> = suggestions
                .names()
                .iter()
                .map(|n| format!("`{n}`"))
                .collect();
            d = d.help(format!("similar names exist: {}", list.join(", ")));
        }
        self.diagnostics
            .push(d.note("a `let` value is computed once, not per row, so it cannot use columns"));
    }

    fn unknown_function(
        &mut self,
        span: Span,
        name: &str,
        unknown: &crate::catalog::UnknownFunction,
    ) {
        if let Some(milestone) = unknown.planned {
            self.diagnostics.push(
                Diagnostic::error(DS1004, format!("function `{name}` is not available yet"))
                    .primary(span, format!("planned for {milestone}"))
                    .note("it is part of the language, but this version does not have it"),
            );
            return;
        }
        let mut d = Diagnostic::error(DS1004, format!("unknown function `{name}`"))
            .primary(span, format!("no function named `{name}`"));
        if let Some(ours) = unknown.equivalent {
            d = d.help_with(
                format!("DataScript calls this function `{ours}`"),
                Suggestion::replace(span, ours),
            );
        } else if let Some(best) = unknown.suggestions.single() {
            d = d.help_with(
                format!("a function with a similar name exists: `{best}`"),
                Suggestion::replace(span, best),
            );
        } else if !unknown.suggestions.is_empty() {
            let list: Vec<String> = unknown
                .suggestions
                .names()
                .iter()
                .map(|n| format!("`{n}`"))
                .collect();
            d = d.help(format!(
                "functions with similar names exist: {}",
                list.join(", ")
            ));
        }
        self.diagnostics.push(d);
    }

    /// Adds where an operand's type came from to the label that says its type:
    /// "this is `string` (column `price` was read from products.csv)".
    fn annotate(&self, d: &mut Diagnostic, operand: &Expr, cx: &Cx<'_>) {
        let origin = match (&operand.kind, cx.cols) {
            (ExprKind::Column(name), Cols::Known(columns)) => columns
                .iter()
                .find(|c| &c.name == name)
                .and_then(|c| c.read_from.as_ref())
                .map(|source| format!("column `{name}` was read from {source}")),
            (ExprKind::Let { name, .. }, _) => Some(format!("`{name}` is defined by a `let`")),
            _ => None,
        };
        let Some(origin) = origin else {
            return;
        };
        for label in d.primary.iter_mut().chain(d.secondary.iter_mut()) {
            if label.span == operand.span && label.message.starts_with("this is `") {
                label.message = format!("{} ({origin})", label.message);
            }
        }
    }

    // --- Helpers ---

    fn scalar_let_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .lets
            .iter()
            .filter(|(_, (_, entry))| matches!(entry, LetEntry::Scalar(_)))
            .map(|(name, _)| name.clone())
            .collect();
        names.sort();
        names
    }

    fn is_pipeline(&self, expr: &ast::Expr) -> bool {
        let mut expr = expr.clone();
        loop {
            match expr {
                ast::Expr::Pipeline(_) => return true,
                ast::Expr::Paren(paren) => match paren.inner() {
                    Some(inner) => expr = inner,
                    None => return false,
                },
                _ => return false,
            }
        }
    }

    fn operand(&self, span: Span) -> Operand<'a> {
        Operand {
            span,
            text: &self.text[span.range()],
        }
    }

    fn span(&self, node: &impl AstNode) -> Span {
        node.span(self.file)
    }

    fn first_token_span(&self, node: &SyntaxNode, fallback: Span) -> Span {
        node.first_token()
            .map(|t| {
                let r = t.text_range();
                Span::new(self.file, r.start().into(), r.end().into())
            })
            .unwrap_or(fallback)
    }
}

fn signatures_note(function: &crate::FunctionDef) -> String {
    match function.signature_lines().as_slice() {
        [one] => format!("signature: `{one}`"),
        many => {
            let list: Vec<String> = many.iter().map(|s| format!("`{s}`")).collect();
            format!("`{}` has these forms: {}", function.name, list.join("; "))
        }
    }
}

/// The nodes that directly contain a syntax error: for each error, the innermost expression,
/// operator or `let` around it. An error directly in a pipeline or the program (a missing `|`
/// or `;`, which the parser already repaired) marks nothing; neither does an empty span (an
/// insertion point).
fn broken_nodes(root: &SyntaxNode, file: FileId, errors: &[Diagnostic]) -> HashSet<SyntaxNode> {
    let mut broken = HashSet::new();
    for error in errors.iter().filter(|d| d.is_error()) {
        let Some(label) = &error.primary else {
            continue;
        };
        let span = label.span;
        if span.file != file || span.start == span.end {
            continue;
        }
        // The deepest node containing the span: the last one in preorder.
        let contains = |node: &SyntaxNode| {
            let r = node.text_range();
            u32::from(r.start()) <= span.start && span.end <= u32::from(r.end())
        };
        let Some(deepest) = root.descendants().filter(|n| contains(n)).last() else {
            continue;
        };
        for node in deepest.ancestors() {
            let kind = node.kind();
            if kind == SyntaxKind::PIPELINE || kind == SyntaxKind::PROGRAM {
                break;
            }
            if ast::Expr::can_cast(kind) || Operator::can_cast(kind) || ast::LetStmt::can_cast(kind)
            {
                broken.insert(node);
                break;
            }
        }
    }
    broken
}

/// Converts `expr` to `ty`. Number literals and `null`s are converted in place so plans stay
/// readable.
fn convert(expr: Expr, ty: Type) -> Expr {
    if expr.ty == ty || expr.ty.is_error() || ty.is_error() {
        return expr;
    }
    match expr.kind {
        ExprKind::Literal(Value::Long(n)) if ty == Type::Real => Expr {
            kind: ExprKind::Literal(Value::Real(n as f64)),
            ty,
            span: expr.span,
        },
        ExprKind::Literal(Value::Null) => Expr {
            kind: ExprKind::Literal(Value::Null),
            ty,
            span: expr.span,
        },
        _ => Expr {
            span: expr.span,
            kind: ExprKind::Cast(Box::new(expr)),
            ty,
        },
    }
}

fn schema_of(columns: &[Column]) -> SchemaRef {
    Arc::new(Schema::new(
        columns
            .iter()
            .map(|c| to_arrow(&c.name, c.ty))
            .collect::<Vec<_>>(),
    ))
}
