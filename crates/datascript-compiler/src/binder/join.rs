//! `join` (M1-16): the kind, the conditions, and the output columns (LANGUAGE.md §3.3, D-019).

use std::collections::HashSet;

use datascript_diagnostics::codes::{DS1001, DS1007, DS1030, DS2003, DS4001};
use datascript_diagnostics::{Diagnostic, Span, Suggestion, did_you_mean};
use datascript_syntax::ast::{self, AstNode};
use datascript_syntax::{SyntaxKind, display_name};

use super::Binder;
use super::ops::{paren_pipeline, unique_name};
use crate::Type;
use crate::hir::{Column, Join, JoinKey, JoinKind, JoinSide, Op, Source};
use crate::ir::BinaryOp;

/// A condition as written, before its columns are checked.
enum Condition {
    /// `on key`.
    Shared { name: String, span: Span },
    /// `$left.a == $right.b` (or the sides the other way round).
    Pair {
        left: (String, Span),
        right: (String, Span),
        span: Span,
    },
}

impl Binder<'_> {
    pub(super) fn bind_join(
        &mut self,
        op: &ast::JoinOp,
        left: &Option<Vec<Column>>,
    ) -> (Op, Option<Vec<Column>>) {
        let kind_token = op.kind().and_then(|k| k.value_token());
        let kind = kind_token
            .as_ref()
            .and_then(|t| JoinKind::from_name(t.text()))
            .unwrap_or(JoinKind::Inner);
        // Capabilities (DS4001): at the kind if it is written, else at `join`.
        let kind_name = kind_token.as_ref().map_or("inner", |t| t.text());
        if let Some(caps_kind) = crate::capabilities::JoinKind::from_name(kind_name)
            && !self.caps.supports_join(caps_kind)
        {
            let op_span = self.span(op);
            let at = match &kind_token {
                Some(t) => {
                    let r = t.text_range();
                    Span::new(self.file, r.start().into(), r.end().into())
                }
                None => self.first_token_span(op.syntax(), op_span),
            };
            self.diagnostics.push(
                Diagnostic::error(
                    DS4001,
                    format!("`{kind_name}` joins are not supported by this engine"),
                )
                .primary(at, "this engine does not implement this kind of join"),
            );
        }
        let Some(right_expr) = op.right() else {
            return (Op::Error, None);
        };
        let right_span = self.span(&right_expr);
        let (right_source, right) = self.bind_source(&right_expr);

        let mut conditions = Vec::new();
        let mut ok = true;
        for cond in op.conditions() {
            if self.broken.contains(cond.syntax()) {
                ok = false;
                continue;
            }
            match self.condition(&cond) {
                Some(c) => conditions.push(c),
                None => ok = false,
            }
        }

        let (Some(left), Some(right)) = (left, &right) else {
            return (Op::Error, None);
        };
        let right_text = self.text[right_span.range()].to_string();
        let mut keys = Vec::new();
        for condition in conditions {
            let key = self.join_key(condition, left, right, &right_text);
            // A key with an error is kept, so an `on key` column is still merged and causes no
            // `DS1007` for a column that only looks duplicated.
            ok &= !key.ty.is_error();
            keys.push(key);
        }

        let (columns, renamed) = match kind.one_side() {
            Some(JoinSide::Left) => (left.clone(), Vec::new()),
            Some(JoinSide::Right) => (right.clone(), Vec::new()),
            None => self.join_columns(left, right, &keys, right_span, ok),
        };
        if !ok || matches!(right_source, Source::Error { .. }) {
            return (Op::Error, Some(columns));
        }
        let join = Join {
            kind,
            right: right_source,
            keys,
            renamed,
        };
        (Op::Join(Box::new(join)), Some(columns))
    }

    /// Reads one condition, or reports `DS1030` and gives `None`.
    fn condition(&mut self, cond: &ast::Expr) -> Option<Condition> {
        let span = self.span(cond);
        if let ast::Expr::NameRef(name_ref) = cond
            && let Some(token) = name_ref.token()
            && !matches!(
                token.kind(),
                SyntaxKind::DOLLAR_LEFT | SyntaxKind::DOLLAR_RIGHT
            )
        {
            return Some(Condition::Shared {
                name: name_ref.text()?,
                span,
            });
        }
        if let ast::Expr::Bin(bin) = cond
            && bin.op() == Some(BinaryOp::Eq)
            && let (Some(a), Some(b)) = (bin.lhs(), bin.rhs())
            && let (Some((side_a, col_a)), Some((side_b, col_b))) =
                (self.side_column(&a), self.side_column(&b))
        {
            return match (side_a, side_b) {
                (JoinSide::Left, JoinSide::Right) => Some(Condition::Pair {
                    left: col_a,
                    right: col_b,
                    span,
                }),
                (JoinSide::Right, JoinSide::Left) => Some(Condition::Pair {
                    left: col_b,
                    right: col_a,
                    span,
                }),
                (same, _) => {
                    let side = if same == JoinSide::Left {
                        "$left"
                    } else {
                        "$right"
                    };
                    self.diagnostics.push(
                        Diagnostic::error(
                            DS1030,
                            format!("both sides of this `join` condition use `{side}`"),
                        )
                        .primary(span, "this compares one table with itself")
                        .help("compare a left column with a right one: `$left.a == $right.b`"),
                    );
                    None
                }
            };
        }
        let text = &self.text[span.range()];
        let mut d = Diagnostic::error(
            DS1030,
            "a `join` condition must be a column name or `$left.a == $right.b`",
        )
        .primary(span, "not a `join` condition");
        if let ast::Expr::Bin(bin) = cond
            && bin.op() == Some(BinaryOp::Eq)
            && let (Some(ast::Expr::NameRef(a)), Some(ast::Expr::NameRef(b))) =
                (bin.lhs(), bin.rhs())
            && let (Some(a), Some(b)) = (a.text(), b.text())
        {
            d = if a == b {
                d.help(format!(
                    "to join on the column both tables have, name it once: `on {a}`"
                ))
            } else {
                d.help(format!(
                    "say which table each column is from: `$left.{a} == $right.{b}`"
                ))
            };
        } else {
            d = d.help(format!(
                "`join` compares columns: compute `{text}` with `extend` on each side first, \
                 then name the new column after `on`"
            ));
        }
        self.diagnostics.push(d);
        None
    }

    /// `$left.a` or `$right.a`: the side and the column.
    fn side_column(&self, expr: &ast::Expr) -> Option<(JoinSide, (String, Span))> {
        let ast::Expr::Field(field) = expr else {
            return None;
        };
        let Some(ast::Expr::NameRef(base)) = field.base() else {
            return None;
        };
        let side = match base.token()?.kind() {
            SyntaxKind::DOLLAR_LEFT => JoinSide::Left,
            SyntaxKind::DOLLAR_RIGHT => JoinSide::Right,
            _ => return None,
        };
        let column = field.field()?;
        Some((side, (column.text()?, self.span(&column))))
    }

    /// Checks a condition's columns against both sides. A key with an error has the error type.
    fn join_key(
        &mut self,
        condition: Condition,
        left: &[Column],
        right: &[Column],
        right_text: &str,
    ) -> JoinKey {
        let (l_name, l_span, r_name, r_span, span, merged) = match condition {
            Condition::Shared { name, span } => (name.clone(), span, name, span, span, true),
            Condition::Pair {
                left: (l, l_span),
                right: (r, r_span),
                span,
            } => (l, l_span, r, r_span, span, false),
        };
        let l_col = left.iter().find(|c| c.name == l_name);
        let r_col = right.iter().find(|c| c.name == r_name);
        let ty = match (l_col, r_col) {
            (Some(l), Some(r)) => match l.ty.common(r.ty) {
                Some(ty) => ty,
                None => {
                    self.key_type_conflict(span, l, r, merged, right_text);
                    Type::Error
                }
            },
            (None, None) if merged => {
                self.missing_key(span, &l_name, left, right);
                Type::Error
            }
            (l, r) => {
                if l.is_none() {
                    self.missing_column(l_span, &l_name, left, "left");
                }
                if r.is_none() {
                    self.missing_column(r_span, &r_name, right, "right");
                }
                Type::Error
            }
        };
        JoinKey {
            left: l_name,
            right: r_name,
            ty,
            merged,
            span,
        }
    }

    /// `DS1001` for `on key` where neither side has `key`, with did-you-mean over the columns
    /// both sides have.
    fn missing_key(&mut self, span: Span, name: &str, left: &[Column], right: &[Column]) {
        let shared: Vec<&str> = left
            .iter()
            .filter(|l| right.iter().any(|r| r.name == l.name))
            .map(|c| c.name.as_str())
            .collect();
        let mut d = Diagnostic::error(DS1001, format!("neither table has a column `{name}`"))
            .primary(span, "not a column of either table");
        if let Some(best) = did_you_mean(name, shared.iter().copied()).single() {
            d = d.help_with(
                format!("a column both tables have has a similar name: `{best}`"),
                Suggestion::replace(span, display_name(best)),
            );
        }
        let note = if shared.is_empty() {
            "the two tables have no column in common; join them with `on $left.a == $right.b`"
                .to_string()
        } else {
            format!("columns both tables have: {}", shared.join(", "))
        };
        self.diagnostics.push(d.note(note));
    }

    fn key_type_conflict(
        &mut self,
        span: Span,
        l: &Column,
        r: &Column,
        merged: bool,
        right_text: &str,
    ) {
        let (lt, rt) = (l.ty, r.ty);
        let what = if merged {
            format!("`{}`", l.name)
        } else {
            format!("`$left.{}` and `$right.{}`", l.name, r.name)
        };
        let origin = |c: &Column| match &c.read_from {
            Some(source) => format!(" (read from {source})"),
            None => String::new(),
        };
        let mut d = Diagnostic::error(
            DS2003,
            format!("cannot join on {what}: the left side is `{lt}` and the right side is `{rt}`"),
        )
        .primary(span, "the key columns have types that cannot be compared")
        .note(format!(
            "on the left `{}` is `{lt}`{}; on the right `{}` is `{rt}`{}",
            l.name,
            origin(l),
            r.name,
            origin(r)
        ));
        let convert = format!("to{lt}");
        if self.catalog.lookup(&convert).is_ok() {
            let condition = &self.text[span.range()];
            let right = paren_pipeline(right_text, &format!("extend {0} = {convert}({0})", r.name));
            d = d.help(format!(
                "convert the right column first: `join {right} on {condition}`"
            ));
        }
        self.diagnostics.push(d);
    }

    /// The output of an inner or outer join: the left columns, then the right ones minus the
    /// `on key` columns. Right columns whose name is taken get `1`, `2`, … (`DS1007`).
    fn join_columns(
        &mut self,
        left: &[Column],
        right: &[Column],
        keys: &[JoinKey],
        right_span: Span,
        report: bool,
    ) -> (Vec<Column>, Vec<(String, String)>) {
        let merged: HashSet<&str> = keys
            .iter()
            .filter(|k| k.merged)
            .map(|k| k.left.as_str())
            .collect();
        let mut out: Vec<Column> = left
            .iter()
            .map(
                |c| match keys.iter().find(|k| k.merged && k.left == c.name) {
                    Some(key) => {
                        let r = right.iter().find(|r| r.name == c.name);
                        Column {
                            name: c.name.clone(),
                            ty: key.ty,
                            read_from: c
                                .read_from
                                .clone()
                                .filter(|_| r.is_some_and(|r| r.read_from == c.read_from)),
                            arrow_only: None,
                        }
                    }
                    None => c.clone(),
                },
            )
            .collect();
        let rest: Vec<&Column> = right
            .iter()
            .filter(|c| !merged.contains(c.name.as_str()))
            .collect();
        let mut taken: HashSet<String> = out
            .iter()
            .map(|c| c.name.clone())
            .chain(rest.iter().map(|c| c.name.clone()))
            .collect();
        let mut renamed = Vec::new();
        for column in rest {
            let mut column = column.clone();
            if out.iter().any(|c| c.name == column.name) {
                let new = unique_name(&column.name, &taken);
                taken.insert(new.clone());
                renamed.push((column.name.clone(), new.clone()));
                column.name = new;
            }
            out.push(column);
        }
        // After an error in the conditions, a rename may only be a symptom of it.
        if report && !renamed.is_empty() {
            self.renamed_info(right_span, &renamed);
        }
        (out, renamed)
    }

    fn renamed_info(&mut self, right_span: Span, renamed: &[(String, String)]) {
        let message = match renamed {
            [(old, new)] => {
                format!("`join` renamed column `{old}` of the right table to `{new}`")
            }
            many => {
                let list: Vec<String> = many
                    .iter()
                    .map(|(old, new)| format!("`{old}` to `{new}`"))
                    .collect();
                format!(
                    "`join` renamed {} columns of the right table: {}",
                    many.len(),
                    list.join(", ")
                )
            }
        };
        let label = match renamed {
            [(old, _)] => format!("this table also has a column `{old}`"),
            _ => "this table has columns with the same names as the left side".to_string(),
        };
        let renames: Vec<String> = renamed
            .iter()
            .map(|(old, _)| format!("right_{old} = {old}"))
            .collect();
        let table = &self.text[right_span.range()];
        let rewritten = paren_pipeline(table, &format!("project-rename {}", renames.join(", ")));
        self.diagnostics.push(
            Diagnostic::info(DS1007, message)
                .primary(right_span, label)
                .help(format!(
                    "to choose the names yourself, rename them on the right: `join {rewritten}`"
                ))
                .note(
                    "a column named in `on` is not renamed: the two sides make one column \
                     (only `$left.a == $right.b` keeps both)",
                ),
        );
    }
}
