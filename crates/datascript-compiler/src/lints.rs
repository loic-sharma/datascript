//! Warning lints: programs that run, but probably not as intended (DIAGNOSTICS.md § Severity).
//!
//! Lints run on the bound [`Hir`] after the binder, so they see resolved names and types. Every
//! lint is a row of [`LINTS`], which holds its code and default severity; a new lint adds a row
//! and a check function. The walk visits every pipeline once: queries, tabular `let`s where they
//! are defined, subqueries, and the right side of `join` and `union`. A node with an error is
//! [`Op::Error`] or [`ExprKind::Error`], which no lint matches, so a lint never adds to an error.

use datascript_diagnostics::codes::{DS2102, DS2103};
use datascript_diagnostics::{
    Applicability, Code, Diagnostic, FileId, Severity, Span, Suggestion, TextEdit,
};

use crate::Type;
use crate::hir::{Expr, ExprKind, Hir, LetValue, Op, Pipeline, Source, Stage, Statement};
use crate::ir::{BinaryOp, PrefixOp, Value};

/// One lint.
#[derive(Debug, Clone, Copy)]
pub struct Lint {
    pub code: Code,
    /// The severity it is reported with unless configured otherwise.
    pub default_severity: Severity,
    /// What it reports, in a few words.
    pub summary: &'static str,
    check: fn(&LintCx<'_>, &Stage, &mut Vec<Diagnostic>),
}

/// Every lint, in code order.
pub const LINTS: &[Lint] = &[
    Lint {
        code: DS2102,
        default_severity: Severity::Warning,
        summary: "`where` condition that is always true or always false",
        check: constant_predicate,
    },
    Lint {
        code: DS2103,
        default_severity: Severity::Warning,
        summary: "`sort` key without `asc` or `desc`",
        check: sort_without_direction,
    },
];

/// What a lint's check function gets besides the stage.
struct LintCx<'a> {
    lint: &'a Lint,
    file: FileId,
    text: &'a str,
}

impl LintCx<'_> {
    /// A diagnostic with the lint's code and severity.
    fn diagnostic(&self, message: impl Into<String>) -> Diagnostic {
        Diagnostic::new(self.lint.default_severity, self.lint.code, message).in_file(self.file)
    }

    fn text(&self, span: Span) -> &str {
        &self.text[span.range()]
    }
}

/// Runs every lint over `hir`, the program bound from `text` (file `file`).
pub(crate) fn run(hir: &Hir, file: FileId, text: &str) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for lint in LINTS {
        let cx = LintCx { lint, file, text };
        for statement in &hir.statements {
            match statement {
                Statement::Query(query) => visit_pipeline(&cx, &query.pipeline, &mut out),
                Statement::Let(binding) => {
                    if let LetValue::Table(pipeline) = &binding.value {
                        visit_pipeline(&cx, pipeline, &mut out);
                    }
                }
            }
        }
    }
    out
}

fn visit_pipeline(cx: &LintCx<'_>, pipeline: &Pipeline, out: &mut Vec<Diagnostic>) {
    visit_source(cx, &pipeline.source, out);
    for stage in &pipeline.stages {
        (cx.lint.check)(cx, stage, out);
        match &stage.op {
            Op::Join(join) => visit_source(cx, &join.right, out),
            Op::Union(sources) => {
                for source in sources {
                    visit_source(cx, source, out);
                }
            }
            _ => {}
        }
    }
}

fn visit_source(cx: &LintCx<'_>, source: &Source, out: &mut Vec<Diagnostic>) {
    match source {
        Source::Subquery { pipeline, .. } => visit_pipeline(cx, pipeline, out),
        // A tabular `let` is linted once, where it is defined.
        Source::Let { .. } | Source::Scan { .. } | Source::Error { .. } => {}
    }
}

// --- DS2102: constant `where` condition ---

fn constant_predicate(cx: &LintCx<'_>, stage: &Stage, out: &mut Vec<Diagnostic>) {
    let Op::Where(predicate) = &stage.op else {
        return;
    };
    let diagnostic = match fold(predicate) {
        Some(Value::Bool(true)) => cx
            .diagnostic("`where` condition is always true")
            .primary(
                predicate.span,
                "true for every row, so `where` keeps them all",
            )
            .help("if this is on purpose, remove the `where`: it has no effect"),
        Some(Value::Bool(false)) => cx
            .diagnostic("`where` condition is always false")
            .primary(
                predicate.span,
                "false for every row, so the result is empty",
            )
            .help("to get the columns with no rows, use `take 0`"),
        _ => return,
    };
    out.push(diagnostic);
}

/// The value of `expr` if it is the same for every row, computed from literals with the rules of
/// LANGUAGE.md §4 (nulls included). Only what is certain folds: columns, function calls (which
/// may depend on the clock or on randomness) and scalar `let`s do not (D-051), except that `or`
/// with a `true` side and `and` with a `false` side are constant whatever the other side is.
fn fold(expr: &Expr) -> Option<Value> {
    match &expr.kind {
        ExprKind::Literal(value) => Some(value.clone()),
        ExprKind::Cast(inner) => match (fold(inner)?, expr.ty) {
            (Value::Null, _) => Some(Value::Null),
            (Value::Long(n), Type::Real) => Some(Value::Real(n as f64)),
            (value, ty) if same_type(&value, ty) => Some(value),
            _ => None,
        },
        ExprKind::Unary { op, operand } => match (op, fold(operand)?) {
            (_, Value::Null) => Some(Value::Null),
            (PrefixOp::Not, Value::Bool(b)) => Some(Value::Bool(!b)),
            (PrefixOp::Neg, Value::Long(n)) => n.checked_neg().map(Value::Long),
            (PrefixOp::Neg, Value::Real(x)) => Some(Value::Real(-x)),
            _ => None,
        },
        ExprKind::Binary { op, left, right } => fold_binary(*op, left, right),
        _ => None,
    }
}

fn fold_binary(op: BinaryOp, left: &Expr, right: &Expr) -> Option<Value> {
    let (l, r) = (fold(left), fold(right));
    match op {
        // Three-valued logic: a `true` side decides `or`, a `false` side decides `and`.
        BinaryOp::Or | BinaryOp::And => {
            let decider = op == BinaryOp::Or;
            if [&l, &r].contains(&&Some(Value::Bool(decider))) {
                return Some(Value::Bool(decider));
            }
            match (l?, r?) {
                (Value::Bool(_), Value::Bool(_)) => Some(Value::Bool(!decider)),
                (Value::Null | Value::Bool(_), Value::Null | Value::Bool(_)) => Some(Value::Null),
                _ => None,
            }
        }
        BinaryOp::Eq
        | BinaryOp::NotEq
        | BinaryOp::Lt
        | BinaryOp::LtEq
        | BinaryOp::Gt
        | BinaryOp::GtEq => {
            let (l, r) = (l?, r?);
            if l == Value::Null || r == Value::Null {
                return Some(Value::Null);
            }
            let ordering = match (&l, &r) {
                (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
                (Value::Long(a), Value::Long(b)) => a.cmp(b),
                (Value::Real(a), Value::Real(b)) => a.partial_cmp(b)?,
                (Value::String(a), Value::String(b)) => a.cmp(b),
                (Value::DateTime(a), Value::DateTime(b))
                | (Value::TimeSpan(a), Value::TimeSpan(b)) => a.cmp(b),
                _ => return None,
            };
            Some(Value::Bool(match op {
                BinaryOp::Eq => ordering.is_eq(),
                BinaryOp::NotEq => ordering.is_ne(),
                BinaryOp::Lt => ordering.is_lt(),
                BinaryOp::LtEq => ordering.is_le(),
                BinaryOp::Gt => ordering.is_gt(),
                _ => ordering.is_ge(),
            }))
        }
        BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul => {
            let (l, r) = (l?, r?);
            match (l, r) {
                (Value::Null, _) | (_, Value::Null) => Some(Value::Null),
                (Value::Long(a), Value::Long(b)) => match op {
                    BinaryOp::Add => a.checked_add(b),
                    BinaryOp::Sub => a.checked_sub(b),
                    _ => a.checked_mul(b),
                }
                .map(Value::Long),
                (Value::Real(a), Value::Real(b)) => Some(Value::Real(match op {
                    BinaryOp::Add => a + b,
                    BinaryOp::Sub => a - b,
                    _ => a * b,
                })),
                _ => None,
            }
        }
        _ => None,
    }
}

fn same_type(value: &Value, ty: Type) -> bool {
    matches!(
        (value, ty),
        (Value::Bool(_), Type::Bool)
            | (Value::Long(_), Type::Long)
            | (Value::Real(_), Type::Real)
            | (Value::String(_), Type::String)
            | (Value::DateTime(_), Type::DateTime)
            | (Value::TimeSpan(_), Type::TimeSpan)
    )
}

// --- DS2103: `sort` key without a direction ---

fn sort_without_direction(cx: &LintCx<'_>, stage: &Stage, out: &mut Vec<Diagnostic>) {
    let Op::Sort(keys) = &stage.op else {
        return;
    };
    let mut missing = keys
        .iter()
        .filter(|k| !k.explicit_direction && !matches!(k.expr.kind, ExprKind::Error))
        .map(|k| k.expr.span);
    let Some(first) = missing.next() else {
        return;
    };
    let others: Vec<Span> = missing.collect();
    let edits = std::iter::once(first)
        .chain(others.iter().copied())
        .map(|span| TextEdit::new(Span::empty(cx.file, span.end), " asc"))
        .collect();
    let example = cx.text(first);
    let help = if example.contains('\n') || example.len() > 40 {
        "write the direction: add `asc` after the key, or `desc` for largest first".to_string()
    } else {
        format!("write the direction: `{example} asc`, or `{example} desc` for largest first")
    };
    let mut diagnostic = cx.diagnostic("`sort` key without `asc` or `desc`").primary(
        first,
        "sorted ascending, while KQL sorts descending by default",
    );
    for span in others {
        diagnostic = diagnostic.secondary(span, "this key has no direction either");
    }
    out.push(diagnostic.help_with(
        help,
        Suggestion::new(edits, Applicability::MachineApplicable),
    ));
}

#[cfg(test)]
mod tests {
    use datascript_diagnostics::codes::{CodeStatus, lookup};

    use super::*;

    #[test]
    fn lints_match_the_registry() {
        assert!(LINTS.windows(2).all(|w| w[0].code < w[1].code));
        for lint in LINTS {
            let info = lookup(lint.code).expect("registered code");
            assert_eq!(info.severity, Some(lint.default_severity), "{}", lint.code);
            assert_eq!(info.status, CodeStatus::Active, "{}", lint.code);
            assert!((2100..2150).contains(&lint.code.number()), "{}", lint.code);
        }
    }
}
