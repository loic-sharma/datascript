//! The binder: resolves sources and columns, types expressions, and builds the IR.
//!
//! This is the walking-skeleton binder (M1-15). It handles one CSV path source, column
//! references, literals, comparisons, `where`, `project` of columns and `take`. Everything else
//! gets `DS9001` "not implemented yet", never a panic. M1-05, M1-16 and M1-17 extend it, and
//! M1-06 splits planning out of it.

use std::collections::BTreeSet;
use std::sync::Arc;

use arrow_schema::{Schema, SchemaRef};
use datascript_diagnostics::codes::{DS1001, DS3001, DS9001};
use datascript_diagnostics::{
    Applicability, Diagnostic, FileId, Span, Suggestion, TextEdit, did_you_mean,
};
use datascript_syntax::ast::{self, AstNode, LiteralKind, Operator, Stmt};
use datascript_syntax::{SyntaxKind, literal};

use crate::Type;
use crate::arrow::{from_arrow, to_arrow};
use crate::ir::{BinaryOp, Expr, ExprKind, Plan, PlanNode, ProjectItem, Value};
use crate::operators::{self, Operand};
use crate::source::{SchemaLookup, SchemaProvider, SourceError, SourceRef};

/// The binder's view of one query statement.
#[derive(Debug, Clone, PartialEq)]
pub struct BoundQuery {
    /// The whole statement.
    pub span: Span,
    /// The plan, if the statement has no errors and every source is known.
    pub plan: Option<Plan>,
}

/// The schema after one pipeline stage (for column completion, later).
#[derive(Debug, Clone, PartialEq)]
pub struct StageSchema {
    /// The stage: the source or the operator.
    pub span: Span,
    pub schema: SchemaRef,
}

/// The columns in scope. `None` means unknown (the source failed or is pending): every column
/// reference then silently has the error type, so nothing cascades.
type Columns = Option<Vec<(String, Type)>>;

pub(crate) struct Binder<'a> {
    file: FileId,
    text: &'a str,
    schemas: &'a dyn SchemaProvider,
    pub(crate) diagnostics: Vec<Diagnostic>,
    pub(crate) pending: BTreeSet<SourceRef>,
    pub(crate) stage_schemas: Vec<StageSchema>,
}

/// A bound expression: its type, and the IR if it has no error.
struct Bound {
    ty: Type,
    expr: Option<Expr>,
}

impl Bound {
    fn error() -> Self {
        Bound {
            ty: Type::Error,
            expr: None,
        }
    }

    fn ok(expr: Expr) -> Self {
        Bound {
            ty: expr.ty,
            expr: Some(expr),
        }
    }
}

impl<'a> Binder<'a> {
    pub(crate) fn new(file: FileId, text: &'a str, schemas: &'a dyn SchemaProvider) -> Self {
        Binder {
            file,
            text,
            schemas,
            diagnostics: Vec::new(),
            pending: BTreeSet::new(),
            stage_schemas: Vec::new(),
        }
    }

    pub(crate) fn bind_program(&mut self, program: &ast::Program) -> Vec<BoundQuery> {
        let mut queries = Vec::new();
        for stmt in program.statements() {
            match stmt {
                Stmt::Let(stmt) => {
                    let span = self.span(&stmt);
                    self.not_implemented(self.first_token_span(stmt.syntax(), span), "`let`");
                }
                Stmt::Query(pipeline) => {
                    let span = self.span(&pipeline);
                    let plan = self.bind_pipeline(&pipeline);
                    queries.push(BoundQuery { span, plan });
                }
            }
        }
        queries
    }

    fn bind_pipeline(&mut self, pipeline: &ast::Pipeline) -> Option<Plan> {
        let errors_before = self.error_count();
        let source = pipeline.source()?;
        let (mut plan, mut columns) = self.bind_source(&source);
        for op in pipeline.operators() {
            let span = self.span(&op);
            let (node, new_columns) = match &op {
                Operator::Where(op) => {
                    let Some(predicate) = op.predicate() else {
                        continue;
                    };
                    let bound = self.bind_expr(&predicate, &columns);
                    if !matches!(bound.ty, Type::Bool | Type::Error) {
                        let what = format!("a `where` predicate of type `{}`", bound.ty);
                        self.not_implemented(self.span(&predicate), &what);
                    }
                    let node = bound.expr.map(Step::Filter);
                    (node, columns.clone())
                }
                Operator::Project(op) => {
                    let mut items = Some(Vec::new());
                    let mut out: Columns = columns.as_ref().map(|_| Vec::new());
                    for item in op.items() {
                        let item_span = self.span(&item);
                        if item.name().is_some() {
                            self.not_implemented(item_span, "naming a column in `project`");
                            items = None;
                            continue;
                        }
                        let Some(expr) = item.expr() else {
                            items = None;
                            continue;
                        };
                        let ast::Expr::NameRef(name_ref) = &expr else {
                            self.not_implemented(item_span, "a computed column in `project`");
                            items = None;
                            continue;
                        };
                        let bound = self.bind_expr(&expr, &columns);
                        let name = name_ref.text().unwrap_or_default();
                        if let Some(out) = &mut out {
                            out.push((name.clone(), bound.ty));
                        }
                        match (bound.expr, &mut items) {
                            (Some(expr), Some(items)) => items.push(ProjectItem { name, expr }),
                            _ => items = None,
                        }
                    }
                    let node = items.map(Step::Project);
                    (node, out)
                }
                Operator::Take(op) => {
                    let count = op.count().and_then(|count| self.take_count(&count));
                    let node = count.map(Step::Limit);
                    (node, columns.clone())
                }
                other => {
                    let what = format!("`{}`", other.name());
                    self.not_implemented(self.first_token_span(other.syntax(), span), &what);
                    (None, None)
                }
            };
            columns = new_columns;
            if let Some(columns) = &columns {
                self.stage_schemas.push(StageSchema {
                    span,
                    schema: schema_of(columns),
                });
            }
            plan = match (plan, node, &columns) {
                (Some(input), Some(node), Some(columns)) => Some(Plan {
                    node: node.attach(input),
                    schema: schema_of(columns),
                    span,
                }),
                _ => None,
            };
        }
        if self.error_count() > errors_before {
            return None;
        }
        plan
    }

    fn bind_source(&mut self, source: &ast::Expr) -> (Option<Plan>, Columns) {
        let span = self.span(source);
        let path = match source {
            ast::Expr::Literal(lit) if lit.kind() == Some(LiteralKind::String) => {
                let Some(token) = lit.token() else {
                    return (None, None);
                };
                match literal::string(token.text(), span) {
                    Ok(path) => path,
                    // The lexer already reported the bad literal.
                    Err(_) => return (None, None),
                }
            }
            ast::Expr::NameRef(_) => {
                self.not_implemented(span, "a table name as a source");
                return (None, None);
            }
            _ => {
                self.not_implemented(span, "this kind of source");
                return (None, None);
            }
        };
        if !path.to_ascii_lowercase().ends_with(".csv") {
            self.not_implemented(span, "reading files other than `.csv`");
            return (None, None);
        }
        let source_ref = SourceRef::Path(path.clone());
        match self.schemas.schema_of(&source_ref) {
            SchemaLookup::Ready(schema) => {
                let columns: Vec<(String, Type)> = schema
                    .fields()
                    .iter()
                    // Columns with no DataScript type are projection-only (DS2008, M1-17);
                    // the skeleton gives them the error type.
                    .map(|f| (f.name().clone(), from_arrow(f).unwrap_or(Type::Error)))
                    .collect();
                let schema = schema_of(&columns);
                self.stage_schemas.push(StageSchema {
                    span,
                    schema: schema.clone(),
                });
                let plan = Plan {
                    node: PlanNode::Scan { source: source_ref },
                    schema,
                    span,
                };
                (Some(plan), Some(columns))
            }
            SchemaLookup::Failed(error) => {
                self.source_error(span, &path, &error);
                (None, None)
            }
            SchemaLookup::Pending => {
                self.pending.insert(source_ref);
                (None, None)
            }
        }
    }

    fn take_count(&mut self, count: &ast::Expr) -> Option<u64> {
        let span = self.span(count);
        if let ast::Expr::Literal(lit) = count
            && lit.kind() == Some(LiteralKind::Long)
        {
            // A bad literal was reported by the lexer.
            return literal::long(lit.token()?.text(), span).ok();
        }
        self.not_implemented(span, "a `take` count that is not a whole-number literal");
        None
    }

    fn bind_expr(&mut self, expr: &ast::Expr, columns: &Columns) -> Bound {
        let span = self.span(expr);
        match expr {
            ast::Expr::Paren(paren) => match paren.inner() {
                Some(inner) => self.bind_expr(&inner, columns),
                None => Bound::error(),
            },
            ast::Expr::NameRef(name_ref) => {
                let Some(name) = name_ref.text() else {
                    return Bound::error();
                };
                let Some(columns) = columns else {
                    return Bound::error();
                };
                match columns.iter().find(|(n, _)| *n == name) {
                    Some((_, Type::Error)) => Bound::error(),
                    Some((_, ty)) => Bound::ok(Expr {
                        kind: ExprKind::Column(name),
                        ty: *ty,
                        span,
                    }),
                    None => {
                        self.unknown_column(span, &name, columns);
                        Bound::error()
                    }
                }
            }
            ast::Expr::Literal(lit) => self.bind_literal(lit, span),
            ast::Expr::Bin(bin) => self.bind_binary(bin, span, columns),
            other => {
                let what = match other {
                    ast::Expr::Call(_) => "a function call",
                    ast::Expr::Prefix(_) => "a prefix operator",
                    ast::Expr::In(_) => "`in`",
                    ast::Expr::Between(_) => "`between`",
                    ast::Expr::TypedNull(_) => "a typed `null`",
                    ast::Expr::Pipeline(_) => "a pipeline inside an expression",
                    _ => "this kind of expression",
                };
                self.not_implemented(span, what);
                Bound::error()
            }
        }
    }

    fn bind_literal(&mut self, lit: &ast::Literal, span: Span) -> Bound {
        let (Some(kind), Some(token)) = (lit.kind(), lit.token()) else {
            return Bound::error();
        };
        // Literal errors were reported by the lexer, so they give the error type silently.
        let value = match kind {
            LiteralKind::Long => match literal::long(token.text(), span) {
                Ok(n) => match i64::try_from(n) {
                    Ok(n) => Value::Long(n),
                    Err(_) => return Bound::error(),
                },
                Err(_) => return Bound::error(),
            },
            LiteralKind::Real => match literal::real(token.text(), span) {
                Ok(x) => Value::Real(x),
                Err(_) => return Bound::error(),
            },
            LiteralKind::String => match literal::string(token.text(), span) {
                Ok(s) => Value::String(s),
                Err(_) => return Bound::error(),
            },
            LiteralKind::Bool => Value::Bool(token.kind() == SyntaxKind::TRUE_KW),
            LiteralKind::Timespan | LiteralKind::Datetime | LiteralKind::Null => {
                let what = match kind {
                    LiteralKind::Timespan => "a `timespan` literal",
                    LiteralKind::Datetime => "a `datetime` literal",
                    _ => "`null`",
                };
                self.not_implemented(span, what);
                return Bound::error();
            }
        };
        let ty = match value {
            Value::Bool(_) => Type::Bool,
            Value::Long(_) => Type::Long,
            Value::Real(_) => Type::Real,
            Value::String(_) => Type::String,
        };
        Bound::ok(Expr {
            kind: ExprKind::Literal(value),
            ty,
            span,
        })
    }

    fn bind_binary(&mut self, bin: &ast::BinExpr, span: Span, columns: &Columns) -> Bound {
        let (Some(op), Some(lhs), Some(rhs)) = (bin.op(), bin.lhs(), bin.rhs()) else {
            return Bound::error();
        };
        let is_comparison = matches!(
            op,
            BinaryOp::Eq
                | BinaryOp::NotEq
                | BinaryOp::Lt
                | BinaryOp::LtEq
                | BinaryOp::Gt
                | BinaryOp::GtEq
        );
        if !is_comparison {
            let op_span = bin
                .op_token()
                .map(|t| {
                    let r = t.text_range();
                    Span::new(self.file, r.start().into(), r.end().into())
                })
                .unwrap_or(span);
            let what = format!("the `{}` operator", operators::op_symbol(op));
            self.not_implemented(op_span, &what);
            return Bound::error();
        }
        let left = self.bind_expr(&lhs, columns);
        let right = self.bind_expr(&rhs, columns);
        match operators::binary(op, left.ty, right.ty) {
            Ok(typing) => {
                let (Some(l), Some(r)) = (left.expr, right.expr) else {
                    return Bound {
                        ty: typing.result,
                        expr: None,
                    };
                };
                Bound::ok(Expr {
                    kind: ExprKind::Binary {
                        op,
                        left: Box::new(convert(l, typing.left)),
                        right: Box::new(convert(r, typing.right)),
                    },
                    ty: typing.result,
                    span,
                })
            }
            Err(error) => {
                let (ls, rs) = (self.span(&lhs), self.span(&rhs));
                let diagnostic = error.diagnostic(
                    Operand {
                        span: ls,
                        text: &self.text[ls.range()],
                    },
                    Operand {
                        span: rs,
                        text: &self.text[rs.range()],
                    },
                );
                self.diagnostics.push(diagnostic);
                Bound::error()
            }
        }
    }

    // --- Diagnostics ---

    fn not_implemented(&mut self, span: Span, what: &str) {
        self.diagnostics.push(
            Diagnostic::error(DS9001, format!("{what} is not implemented yet"))
                .primary(span, "not supported by this version of DataScript")
                .note(
                    "this version can only read a `.csv` file and apply `where` with \
                     comparisons, `project` of columns and `take`",
                ),
        );
    }

    fn unknown_column(&mut self, span: Span, name: &str, columns: &[(String, Type)]) {
        let names: Vec<&str> = columns.iter().map(|(n, _)| n.as_str()).collect();
        let mut d = Diagnostic::error(DS1001, format!("unknown column `{name}`"))
            .primary(span, format!("no column named `{name}` here"));
        let suggestions = did_you_mean(name, names.iter().copied());
        if let Some(best) = suggestions.single() {
            d = d.help_with(
                format!("a column with a similar name exists: `{best}`"),
                Suggestion::replace(span, best),
            );
        } else if !suggestions.is_empty() {
            let list: Vec<String> = suggestions
                .names()
                .iter()
                .map(|n| format!("`{n}`"))
                .collect();
            d = d.help(format!(
                "columns with similar names exist: {}",
                list.join(", ")
            ));
        }
        let available = if names.is_empty() {
            "there are no columns at this point".to_string()
        } else {
            format!("columns available at this point: {}", names.join(", "))
        };
        self.diagnostics.push(d.note(available));
    }

    fn source_error(&mut self, span: Span, path: &str, error: &SourceError) {
        match error {
            SourceError::NotFound { nearby } => {
                let (dir, file_name) = match path.rfind(['/', '\\']) {
                    Some(i) => (&path[..=i], &path[i + 1..]),
                    None => ("", path),
                };
                let mut d = Diagnostic::error(DS3001, format!("file `{path}` not found"))
                    .primary(span, "no file at this path");
                let suggestions = did_you_mean(file_name, nearby.iter().map(String::as_str));
                if let Some(best) = suggestions.single() {
                    let literal = &self.text[span.range()];
                    let help = format!("a file with a similar name exists: `{dir}{best}`");
                    // Replace the file name inside the literal; a literal with escapes that
                    // spell the name differently gets the help without an edit.
                    match literal.rfind(file_name) {
                        Some(i) => {
                            let start = span.start + i as u32;
                            let edit = TextEdit::new(
                                Span::new(self.file, start, start + file_name.len() as u32),
                                best,
                            );
                            d = d.help_with(
                                help,
                                Suggestion::new(vec![edit], Applicability::MaybeIncorrect),
                            );
                        }
                        None => d = d.help(help),
                    }
                }
                self.diagnostics.push(d);
            }
            SourceError::Unreadable { reason } => {
                self.diagnostics.push(
                    Diagnostic::error(DS9001, format!("cannot read the schema of `{path}`"))
                        .primary(span, "this file could not be read")
                        .note(format!("the engine said: {reason}")),
                );
            }
        }
    }

    // --- Helpers ---

    fn error_count(&self) -> usize {
        self.diagnostics.iter().filter(|d| d.is_error()).count()
    }

    fn span(&self, node: &impl AstNode) -> Span {
        node.span(self.file)
    }

    fn first_token_span(&self, node: &datascript_syntax::SyntaxNode, fallback: Span) -> Span {
        node.first_token()
            .map(|t| {
                let r = t.text_range();
                Span::new(self.file, r.start().into(), r.end().into())
            })
            .unwrap_or(fallback)
    }
}

/// Converts `expr` to `ty`. Number literals are converted in place so plans stay readable.
fn convert(expr: Expr, ty: Type) -> Expr {
    if expr.ty == ty || expr.ty == Type::Null {
        return expr;
    }
    match expr.kind {
        ExprKind::Literal(Value::Long(n)) if ty == Type::Real => Expr {
            kind: ExprKind::Literal(Value::Real(n as f64)),
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

fn schema_of(columns: &[(String, Type)]) -> SchemaRef {
    Arc::new(Schema::new(
        columns
            .iter()
            .map(|(name, ty)| to_arrow(name, *ty))
            .collect::<Vec<_>>(),
    ))
}

/// A bound operator, before it is attached to its input.
enum Step {
    Filter(Expr),
    Project(Vec<ProjectItem>),
    Limit(u64),
}

impl Step {
    fn attach(self, input: Plan) -> PlanNode {
        let input = Box::new(input);
        match self {
            Step::Filter(predicate) => PlanNode::Filter { input, predicate },
            Step::Project(columns) => PlanNode::Project { input, columns },
            Step::Limit(count) => PlanNode::Limit { input, count },
        }
    }
}
