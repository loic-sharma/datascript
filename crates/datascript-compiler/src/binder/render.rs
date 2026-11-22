//! `render` (M1-10, LANGUAGE.md §6): checks the kind and properties against the output columns
//! and resolves every default into an explicit [`RenderSpec`] (D-047).
//!
//! Checks run in this order, so one root cause gives one error:
//!
//! 1. `render` outside a query statement is `DS5008`, and nothing else is checked.
//! 2. The kind: `DS5003`.
//! 3. Each property: unknown or repeated `DS5004`, not for this kind `DS5006`, bad value
//!    `DS5005`, unknown column `DS1001`. A bad property does not stop the others.
//! 4. Roles, unless the kind is bad or a role's own property had an error: a column in two
//!    roles `DS5007`, then a wrongly typed explicit column or a role that defaults cannot fill
//!    `DS5002`.

use std::collections::HashMap;

use datascript_diagnostics::codes::{
    DS1001, DS5002, DS5003, DS5004, DS5005, DS5006, DS5007, DS5008,
};
use datascript_diagnostics::{Diagnostic, Span, Suggestion, did_you_mean};
use datascript_syntax::ast::{self, AstNode, LiteralKind, PrefixOp};
use datascript_syntax::{SyntaxKind, SyntaxNode, display_name, literal};

use super::sources::const_value;
use super::{Binder, LetEntry};
use crate::Type;
use crate::hir::{Column, Expr, ExprKind, Op};
use crate::ir::{ChartKind, Legend, RenderNumber, RenderSpec, Stacking, Value};

/// KQL chart kinds DataScript does not draw yet: `DS5003` says so instead of a did-you-mean.
const KQL_KINDS: &[&str] = &[
    "anomalychart",
    "stackedareachart",
    "ladderchart",
    "pivotchart",
    "timepivot",
    "treemap",
];

/// KQL properties DataScript does not support yet: `DS5004` says so instead of a did-you-mean.
const KQL_PROPS: &[&str] = &["accumulate", "xaxis", "yaxis", "ysplit", "anomalycolumns"];

/// A `render` property (LANGUAGE.md §6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Prop {
    Title,
    XColumn,
    YColumns,
    Series,
    XTitle,
    YTitle,
    Legend,
    YMin,
    YMax,
    Kind,
}

impl Prop {
    const ALL: [Prop; 10] = [
        Prop::Title,
        Prop::XColumn,
        Prop::YColumns,
        Prop::Series,
        Prop::XTitle,
        Prop::YTitle,
        Prop::Legend,
        Prop::YMin,
        Prop::YMax,
        Prop::Kind,
    ];

    fn name(self) -> &'static str {
        match self {
            Prop::Title => "title",
            Prop::XColumn => "xcolumn",
            Prop::YColumns => "ycolumns",
            Prop::Series => "series",
            Prop::XTitle => "xtitle",
            Prop::YTitle => "ytitle",
            Prop::Legend => "legend",
            Prop::YMin => "ymin",
            Prop::YMax => "ymax",
            Prop::Kind => "kind",
        }
    }

    fn from_name(name: &str) -> Option<Prop> {
        Prop::ALL.into_iter().find(|p| p.name() == name)
    }

    fn applies_to(self, kind: ChartKind) -> bool {
        match self {
            Prop::Title => true,
            Prop::XColumn | Prop::YColumns | Prop::XTitle | Prop::YTitle | Prop::Legend => {
                kind.has_axes()
            }
            Prop::Series => kind.has_series(),
            Prop::YMin | Prop::YMax => kind.has_y_range(),
            Prop::Kind => kind.has_stacking(),
        }
    }
}

/// What the user wrote for one role.
#[derive(Debug, Clone, Default)]
enum Given {
    #[default]
    Unset,
    Columns(Vec<(String, Span)>),
    /// The property had an error that is already reported: the role is not checked.
    Error,
}

/// The checked properties of one `render`.
#[derive(Debug, Default)]
struct Props {
    x: Given,
    y: Given,
    series: Given,
    title: Option<String>,
    xtitle: Option<String>,
    ytitle: Option<String>,
    legend: Option<Legend>,
    ymin: Option<(RenderNumber, Span)>,
    ymax: Option<(RenderNumber, Span)>,
    stacking: Option<Stacking>,
}

/// A chart role, for messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    X,
    Y,
    Series,
}

impl Role {
    fn text(self) -> &'static str {
        match self {
            Role::X => "the x-axis",
            Role::Y => "the y-axis",
            Role::Series => "`series`",
        }
    }
}

fn is_numeric(ty: Type) -> bool {
    matches!(ty, Type::Int | Type::Long | Type::Real | Type::Decimal)
}

fn is_typed(c: &Column) -> bool {
    c.arrow_only.is_none() && !c.ty.is_error()
}

/// Whether the x role of `kind` accepts `c`, and how to say what it needs.
fn x_accepts(kind: ChartKind, c: &Column) -> bool {
    is_typed(c)
        && match kind {
            ChartKind::Timechart => c.ty == Type::DateTime,
            ChartKind::Linechart | ChartKind::Areachart | ChartKind::Scatterchart => {
                c.ty == Type::DateTime || is_numeric(c.ty)
            }
            _ => true,
        }
}

fn x_needs(kind: ChartKind) -> &'static str {
    match kind {
        ChartKind::Timechart => "a `datetime` column",
        ChartKind::Linechart | ChartKind::Areachart | ChartKind::Scatterchart => {
            "a `datetime` or numeric column"
        }
        _ => "a column with a type",
    }
}

fn y_accepts(c: &Column) -> bool {
    is_typed(c) && is_numeric(c.ty)
}

fn series_accepts(c: &Column) -> bool {
    is_typed(c) && matches!(c.ty, Type::String | Type::Bool | Type::Int | Type::Long)
}

/// `name: type` for a column in messages.
fn column_text(c: &Column) -> String {
    let name = display_name(&c.name);
    match &c.arrow_only {
        Some(arrow) => format!("{name}: arrow:{arrow}"),
        None => format!("{name}: {}", c.ty),
    }
}

/// "is a `string`" / "has no DataScript type".
fn type_text(c: &Column) -> String {
    if is_typed(c) {
        format!("is a `{}`", c.ty)
    } else {
        "has no DataScript type".to_string()
    }
}

fn columns_note(columns: &[Column]) -> String {
    if columns.is_empty() {
        return "the result has no columns".to_string();
    }
    let list: Vec<String> = columns.iter().map(column_text).collect();
    format!("the result's columns: {}", list.join(", "))
}

fn kinds_note() -> String {
    let list: Vec<&str> = ChartKind::ALL.iter().map(|k| k.name()).collect();
    format!("the chart kinds are {}", list.join(", "))
}

fn props_note() -> String {
    let list: Vec<&str> = Prop::ALL.iter().map(|p| p.name()).collect();
    format!("the properties are {}", list.join(", "))
}

/// The value of a numeric constant: a literal, a negated literal, or a `let` bound to one.
fn const_number(expr: &Expr) -> Option<RenderNumber> {
    match &expr.kind {
        ExprKind::Literal(Value::Long(n)) => Some(RenderNumber::Long(*n)),
        ExprKind::Literal(Value::Real(x)) => Some(RenderNumber::Real(*x)),
        ExprKind::Let { value, .. } => const_number(value),
        ExprKind::Unary {
            op: PrefixOp::Neg,
            operand,
        } => match const_number(operand)? {
            RenderNumber::Long(n) => n.checked_neg().map(RenderNumber::Long),
            RenderNumber::Real(x) => Some(RenderNumber::Real(-x)),
        },
        ExprKind::Cast(inner) => const_number(inner),
        _ => None,
    }
}

impl Binder<'_> {
    /// Binds `render` over `columns`. The output columns are the input columns.
    pub(super) fn bind_render(
        &mut self,
        op: &ast::RenderOp,
        columns: &Option<Vec<Column>>,
    ) -> (Op, Option<Vec<Column>>) {
        let after = columns.clone();
        let op_span = self.span(op);

        // 1. Only a query statement's result is shown.
        if op.syntax().parent() != self.query_pipeline {
            self.render_misplaced(op, op_span);
            return (Op::Error, after);
        }

        // 2. The kind.
        let Some(kind_token) = op.kind_token() else {
            return (Op::Error, after);
        };
        let kind_span = self.token_span(&kind_token);
        let kind = ChartKind::from_name(kind_token.text());
        if kind.is_none() {
            self.unknown_kind(kind_token.text(), kind_span);
        }

        // 3. The properties.
        let mut props = Props::default();
        let mut ok = kind.is_some();
        let mut seen: HashMap<Prop, Span> = HashMap::new();
        for prop in op.properties() {
            if self.broken.contains(prop.syntax()) {
                ok = false;
                continue;
            }
            let Some(name_node) = prop.name() else {
                ok = false;
                continue;
            };
            let Some(name) = name_node.text() else {
                ok = false;
                continue;
            };
            let name_span = self.span(&name_node);
            let values: Vec<ast::Expr> = prop.values().collect();
            let Some(p) = Prop::from_name(&name) else {
                self.unknown_prop(&name, name_span);
                ok = false;
                continue;
            };
            if let Some(&first) = seen.get(&p) {
                self.diagnostics.push(
                    Diagnostic::error(DS5004, format!("`{name}` is given twice"))
                        .primary(name_span, "given again here")
                        .secondary(first, "first given here")
                        .help("remove one of them"),
                );
                ok = false;
                continue;
            }
            seen.insert(p, name_span);
            if let Some(kind) = kind
                && !p.applies_to(kind)
            {
                self.prop_not_for_kind(p, kind, name_span, &values);
                ok = false;
                continue;
            }
            if !self.prop_value(p, kind, &values, columns, &mut props) {
                ok = false;
            }
        }
        if let (Some((min, min_span)), Some((max, max_span))) = (props.ymin, props.ymax)
            && min.as_f64() >= max.as_f64()
        {
            self.diagnostics.push(
                Diagnostic::error(DS5005, "`ymin` must be less than `ymax`")
                    .primary(min_span, format!("`ymin` is {min}"))
                    .secondary(max_span, format!("`ymax` is {max}")),
            );
            ok = false;
        }

        // 4. Roles.
        let (Some(kind), Some(cols)) = (kind, columns) else {
            return (Op::Error, after);
        };
        let Some(spec) = self.resolve_roles(kind, kind_span, cols, &props) else {
            return (Op::Error, after);
        };
        if !ok {
            return (Op::Error, after);
        }
        let mut spec = spec;
        spec.title = props.title;
        if kind.has_axes() {
            spec.xtitle = props.xtitle;
            spec.ytitle = props.ytitle;
            spec.legend = Some(props.legend.unwrap_or_default());
        }
        spec.ymin = props.ymin.map(|(n, _)| n);
        spec.ymax = props.ymax.map(|(n, _)| n);
        if kind.has_stacking() {
            spec.stacking = Some(props.stacking.unwrap_or_default());
        }
        (Op::Render(Box::new(spec)), after)
    }

    /// `DS5008`: `render` in a tabular `let`, in parentheses, or on the right of `join`/`union`.
    fn render_misplaced(&mut self, op: &ast::RenderOp, op_span: Span) {
        let place = op
            .syntax()
            .parent()
            .map_or("in a pipeline in parentheses", |p| render_place(&p));
        let span = self.first_token_span(op.syntax(), op_span);
        self.diagnostics.push(
            Diagnostic::error(DS5008, "`render` must end a query statement")
                .primary(span, format!("this `render` is {place}"))
                .help("move the `render` to the end of the query statement that shows this table")
                .note(
                    "only a query statement's result is shown, so a chart anywhere else could \
                     never be drawn",
                ),
        );
    }

    /// `DS5003`: an unknown chart kind.
    fn unknown_kind(&mut self, text: &str, span: Span) {
        let d = if KQL_KINDS.contains(&text) {
            Diagnostic::error(
                DS5003,
                format!("the chart kind `{text}` is not supported yet"),
            )
            .primary(span, "a KQL chart kind DataScript cannot draw yet")
        } else {
            let mut d = Diagnostic::error(DS5003, format!("unknown chart kind `{text}`"))
                .primary(span, "not a chart kind");
            let suggestions = did_you_mean(text, ChartKind::ALL.iter().map(|k| k.name()));
            if let Some(best) = suggestions.single() {
                d = d.help_with(
                    format!("a chart kind with a similar name exists: `{best}`"),
                    Suggestion::replace(span, best),
                );
            }
            d
        };
        self.diagnostics.push(d.note(kinds_note()));
    }

    /// `DS5004`: an unknown property.
    fn unknown_prop(&mut self, name: &str, span: Span) {
        let d = if KQL_PROPS.contains(&name) {
            Diagnostic::error(
                DS5004,
                format!("the render property `{name}` is not supported yet"),
            )
            .primary(span, "a KQL property DataScript does not support yet")
        } else {
            let mut d = Diagnostic::error(DS5004, format!("unknown render property `{name}`"))
                .primary(span, "not a render property");
            let suggestions = did_you_mean(name, Prop::ALL.iter().map(|p| p.name()));
            if let Some(best) = suggestions.single() {
                d = d.help_with(
                    format!("a property with a similar name exists: `{best}`"),
                    Suggestion::replace(span, best),
                );
            }
            d
        };
        self.diagnostics.push(d.note(props_note()));
    }

    /// `DS5006`: a known property on a kind it does not apply to.
    fn prop_not_for_kind(&mut self, p: Prop, kind: ChartKind, span: Span, values: &[ast::Expr]) {
        let is_map = p == Prop::Kind
            && matches!(kind, ChartKind::Piechart | ChartKind::Scatterchart)
            && matches!(values, [ast::Expr::NameRef(n)] if n.text().as_deref() == Some("map"));
        if is_map {
            self.diagnostics.push(
                Diagnostic::error(DS5006, "map charts are not supported yet")
                    .primary(span, "`kind = map` asks for a KQL map chart"),
            );
            return;
        }
        let accepted: Vec<&str> = Prop::ALL
            .into_iter()
            .filter(|q| q.applies_to(kind))
            .map(Prop::name)
            .collect();
        let note = if accepted.len() == 1 {
            format!("a `{kind}` accepts only `{}`", accepted[0])
        } else {
            let list: Vec<String> = accepted.iter().map(|n| format!("`{n}`")).collect();
            format!("a `{kind}` accepts {}", list.join(", "))
        };
        self.diagnostics.push(
            Diagnostic::error(
                DS5006,
                format!("`{}` does not apply to a `{kind}`", p.name()),
            )
            .primary(span, format!("not a `{kind}` property"))
            .note(note),
        );
    }

    /// Checks a property's value and stores it in `props`. Returns whether it was valid.
    fn prop_value(
        &mut self,
        p: Prop,
        kind: Option<ChartKind>,
        values: &[ast::Expr],
        columns: &Option<Vec<Column>>,
        props: &mut Props,
    ) -> bool {
        // A missing value is a syntax error, already reported.
        let Some(first) = values.first() else {
            return false;
        };
        let name = p.name();
        match p {
            Prop::XColumn | Prop::YColumns | Prop::Series => {
                let one = p == Prop::XColumn
                    || (p == Prop::YColumns && kind == Some(ChartKind::Piechart));
                let given = self.column_values(p, one, values, columns);
                let ok = !matches!(given, Given::Error);
                match p {
                    Prop::XColumn => props.x = given,
                    Prop::YColumns => props.y = given,
                    _ => props.series = given,
                }
                ok
            }
            Prop::Title | Prop::XTitle | Prop::YTitle => {
                if !self.one_value(name, values) {
                    return false;
                }
                let Some(text) = self.const_string(name, first) else {
                    return false;
                };
                match p {
                    Prop::Title => props.title = Some(text),
                    Prop::XTitle => props.xtitle = Some(text),
                    _ => props.ytitle = Some(text),
                }
                true
            }
            Prop::YMin | Prop::YMax => {
                if !self.one_value(name, values) {
                    return false;
                }
                let span = self.span(first);
                let Some(n) = self.const_num(name, first) else {
                    return false;
                };
                if p == Prop::YMin {
                    props.ymin = Some((n, span));
                } else {
                    props.ymax = Some((n, span));
                }
                true
            }
            Prop::Legend => {
                if !self.one_value(name, values) {
                    return false;
                }
                let word = self.bare_word(name, first, &["visible", "hidden"]);
                props.legend = match word {
                    Some("visible") => Some(Legend::Visible),
                    Some("hidden") => Some(Legend::Hidden),
                    _ => return false,
                };
                true
            }
            Prop::Kind => {
                if !self.one_value(name, values) {
                    return false;
                }
                let word = self.bare_word(
                    name,
                    first,
                    &["default", "unstacked", "stacked", "stacked100"],
                );
                props.stacking = match word {
                    Some("default" | "unstacked") => Some(Stacking::Default),
                    Some("stacked") => Some(Stacking::Stacked),
                    Some("stacked100") => Some(Stacking::Stacked100),
                    _ => return false,
                };
                true
            }
        }
    }

    /// `DS5005` unless there is exactly one value.
    fn one_value(&mut self, name: &str, values: &[ast::Expr]) -> bool {
        if values.len() <= 1 {
            return true;
        }
        let span = self.extra_values_span(values);
        self.diagnostics.push(
            Diagnostic::error(DS5005, format!("`{name}` takes one value"))
                .primary(span, "remove the extra values"),
        );
        false
    }

    /// From the start of the second value to the end of the last.
    fn extra_values_span(&self, values: &[ast::Expr]) -> Span {
        let second = self.span(&values[1]);
        let last = self.span(&values[values.len() - 1]);
        Span::new(self.file, second.start, last.end)
    }

    /// The columns named by `xcolumn`, `ycolumns` or `series`.
    fn column_values(
        &mut self,
        p: Prop,
        one: bool,
        values: &[ast::Expr],
        columns: &Option<Vec<Column>>,
    ) -> Given {
        let name = p.name();
        if one && values.len() > 1 {
            let span = self.extra_values_span(values);
            let (message, label) = if p == Prop::XColumn {
                (
                    "`xcolumn` takes one column".to_string(),
                    "a chart has one x-axis",
                )
            } else {
                (
                    "a `piechart` takes one column in `ycolumns`".to_string(),
                    "a pie chart has one value per slice",
                )
            };
            self.diagnostics.push(
                Diagnostic::error(DS5005, message)
                    .primary(span, label)
                    .help("remove the extra columns"),
            );
            return Given::Error;
        }
        let mut out: Vec<(String, Span)> = Vec::new();
        let mut ok = true;
        for value in values {
            let span = self.span(value);
            let column = match value {
                ast::Expr::NameRef(name_ref) => name_ref.text(),
                _ => None,
            };
            let Some(column) = column else {
                self.not_a_column(name, value, span);
                ok = false;
                continue;
            };
            if let Some((_, first)) = out.iter().find(|(c, _)| *c == column) {
                let first = *first;
                self.diagnostics.push(
                    Diagnostic::error(
                        DS5005,
                        format!("`{}` is listed twice in `{name}`", display_name(&column)),
                    )
                    .primary(span, "listed again here")
                    .secondary(first, "first listed here")
                    .help("remove one of them"),
                );
                ok = false;
                continue;
            }
            match columns {
                Some(cols) if !cols.iter().any(|c| c.name == column) => {
                    self.unknown_render_column(span, &column, cols);
                    ok = false;
                }
                Some(_) => {}
                // The columns are unknown because of an earlier error.
                None => ok = false,
            }
            out.push((column, span));
        }
        if ok {
            Given::Columns(out)
        } else {
            Given::Error
        }
    }

    /// `DS5005`: a column property's value is not a column name.
    fn not_a_column(&mut self, name: &str, value: &ast::Expr, span: Span) {
        let mut d = Diagnostic::error(DS5005, format!("`{name}` takes column names"))
            .primary(span, "this is not a column name");
        let quoted = match value {
            ast::Expr::Literal(lit) if lit.kind() == Some(LiteralKind::String) => lit
                .token()
                .and_then(|t| literal::string(t.text(), span).ok()),
            _ => None,
        };
        d = match quoted {
            Some(text) => {
                let bare = display_name(&text);
                d.help_with(
                    format!("a column name is written without string quotes: `{bare}`"),
                    Suggestion::replace(span, bare),
                )
            }
            None => d.help(
                "write the name of an output column, bare or quoted as `['name']`; render \
                 properties do not take expressions, so compute the column with `extend` first",
            ),
        };
        self.diagnostics.push(d);
    }

    /// `DS1001` over the output columns.
    fn unknown_render_column(&mut self, span: Span, name: &str, columns: &[Column]) {
        let names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
        let suggestions = did_you_mean(name, names.iter().copied());
        let mut d = Diagnostic::error(DS1001, format!("unknown column `{name}`"))
            .primary(span, format!("no column named `{name}` here"));
        if let Some(best) = suggestions.single() {
            d = d.help_with(
                format!("a column with a similar name exists: `{best}`"),
                Suggestion::replace(span, display_name(best)),
            );
        } else if !suggestions.is_empty() {
            let list: Vec<String> = suggestions
                .names()
                .iter()
                .map(|n| format!("`{n}`"))
                .collect();
            d = d.help(format!("similar names exist: {}", list.join(", ")));
        }
        let available = if names.is_empty() {
            "there are no columns at this point".to_string()
        } else {
            format!("columns available at this point: {}", names.join(", "))
        };
        self.diagnostics.push(d.note(available));
    }

    /// A constant expression: a literal, a negated literal, or a scalar `let`. `None` with no
    /// diagnostic when the value already has an error.
    fn const_value_of(&mut self, value: &ast::Expr) -> Result<Expr, ()> {
        if let ast::Expr::NameRef(name_ref) = value
            && let Some(name) = name_ref.text()
        {
            return match self.lets.get(&name).map(|(_, e)| e.clone()) {
                Some(LetEntry::Scalar(expr)) => Ok(expr),
                Some(LetEntry::Error) => Err(()),
                // A bare word or a table: not a constant; the caller reports it.
                _ => Ok(Expr {
                    ty: Type::Error,
                    kind: ExprKind::Column(name),
                    span: self.span(value),
                }),
            };
        }
        let expr = self.const_expr(value);
        if expr.ty.is_error() {
            return Err(());
        }
        Ok(expr)
    }

    /// `title`, `xtitle`, `ytitle`: a constant string.
    fn const_string(&mut self, name: &str, value: &ast::Expr) -> Option<String> {
        let expr = self.const_value_of(value).ok()?;
        if let Some(Value::String(s)) = const_value(&expr) {
            return Some(s.clone());
        }
        let span = self.span(value);
        let mut d = Diagnostic::error(DS5005, format!("`{name}` must be a constant string"))
            .primary(span, "this is not a constant string");
        d = match &expr.kind {
            ExprKind::Column(word) => {
                let quoted = format!("\"{word}\"");
                d.help_with(
                    format!("write the text in quotes: `{name} = {quoted}`"),
                    Suggestion::replace(span, quoted),
                )
            }
            _ => d.help(format!(
                "write a string literal such as `{name} = \"Revenue\"`, or the name of a `let` \
                 bound to one"
            )),
        };
        self.diagnostics.push(d);
        None
    }

    /// `ymin`, `ymax`: a constant number.
    fn const_num(&mut self, name: &str, value: &ast::Expr) -> Option<RenderNumber> {
        let expr = self.const_value_of(value).ok()?;
        if let Some(n) = const_number(&expr) {
            return Some(n);
        }
        let span = self.span(value);
        self.diagnostics.push(
            Diagnostic::error(DS5005, format!("`{name}` must be a constant number"))
                .primary(span, "this is not a constant number")
                .help(format!(
                    "write a number such as `{name} = 0`, or the name of a `let` bound to one"
                )),
        );
        None
    }

    /// `legend`, `kind`: one of `words`, written bare.
    fn bare_word(
        &mut self,
        name: &str,
        value: &ast::Expr,
        words: &[&'static str],
    ) -> Option<&'static str> {
        let span = self.span(value);
        let text = match value {
            ast::Expr::NameRef(name_ref) => name_ref.text(),
            _ => None,
        };
        let list: Vec<String> = words.iter().map(|w| format!("`{w}`")).collect();
        let expected = match list.split_last() {
            Some((last, rest)) if !rest.is_empty() => format!("{} or {last}", rest.join(", ")),
            _ => list.join(""),
        };
        let Some(text) = text else {
            self.diagnostics.push(
                Diagnostic::error(DS5005, format!("`{name}` must be {expected}"))
                    .primary(span, format!("expected {expected}"))
                    .help(format!(
                        "write the word bare, such as `{name} = {}`",
                        words[0]
                    )),
            );
            return None;
        };
        if let Some(word) = words.iter().find(|w| **w == text) {
            return Some(word);
        }
        let mut d = Diagnostic::error(DS5005, format!("unknown `{name}` value `{text}`"))
            .primary(span, format!("expected {expected}"));
        if let Some(best) = did_you_mean(&text, words.iter().copied()).single() {
            d = d.help_with(
                format!("a value with a similar name exists: `{best}`"),
                Suggestion::replace(span, best),
            );
        }
        self.diagnostics.push(d);
        None
    }

    /// Checks the roles and fills the defaults (LANGUAGE.md §6.2). `None` after an error, or
    /// when a role's property already had one.
    fn resolve_roles(
        &mut self,
        kind: ChartKind,
        kind_span: Span,
        cols: &[Column],
        props: &Props,
    ) -> Option<RenderSpec> {
        let column = |name: &str| cols.iter().find(|c| c.name == name);
        let given = |g: &Given| match g {
            Given::Columns(cs) => Some(cs.clone()),
            _ => None,
        };
        let xs = given(&props.x);
        let ys = given(&props.y);
        let ss = given(&props.series);

        // A column in two roles.
        let mut roles: Vec<(Role, String, Span)> = Vec::new();
        for (role, list) in [(Role::X, &xs), (Role::Y, &ys), (Role::Series, &ss)] {
            for (name, span) in list.iter().flatten() {
                roles.push((role, name.clone(), *span));
            }
        }
        let mut conflict = false;
        for (i, (role, name, span)) in roles.iter().enumerate() {
            if let Some((first_role, _, first_span)) =
                roles[..i].iter().find(|(r, n, _)| n == name && r != role)
            {
                self.diagnostics.push(
                    Diagnostic::error(
                        DS5007,
                        format!(
                            "`{}` is used for both {} and {}",
                            display_name(name),
                            first_role.text(),
                            role.text()
                        ),
                    )
                    .primary(*span, format!("used for {} here", role.text()))
                    .secondary(*first_span, format!("used for {} here", first_role.text()))
                    .help("a column can fill only one role; remove it from one of them"),
                );
                conflict = true;
            }
        }
        if conflict {
            return None;
        }

        // Explicit columns of the wrong type.
        let mut ok = true;
        for (role, name, span) in &roles {
            let Some(c) = column(name) else { continue };
            let (fits, needs) = match role {
                Role::X => (x_accepts(kind, c), x_needs(kind)),
                Role::Y => (y_accepts(c), "a numeric column"),
                Role::Series => (
                    series_accepts(c),
                    "`string`, `bool`, `int` or `long` columns",
                ),
            };
            if !fits {
                self.diagnostics.push(
                    Diagnostic::error(
                        DS5002,
                        format!(
                            "`{kind}` needs {needs} for {}, but `{}` {}",
                            role.text(),
                            display_name(name),
                            type_text(c)
                        ),
                    )
                    .primary(*span, format!("this column {}", type_text(c)))
                    .note(columns_note(cols)),
                );
                ok = false;
            }
        }
        let any_error = [&props.x, &props.y, &props.series]
            .iter()
            .any(|g| matches!(g, Given::Error));
        if !ok || any_error {
            return None;
        }

        let mut spec = RenderSpec::new(kind);
        if kind == ChartKind::Table {
            return Some(spec);
        }
        if kind == ChartKind::Card {
            let y: Vec<String> = cols
                .iter()
                .filter(|c| is_typed(c))
                .map(|c| c.name.clone())
                .collect();
            if y.is_empty() {
                self.diagnostics.push(
                    Diagnostic::error(
                        DS5002,
                        "`card` needs a column with a type, but the result has none",
                    )
                    .primary(kind_span, "nothing to show")
                    .note(columns_note(cols)),
                );
                return None;
            }
            spec.y = Some(y);
            return Some(spec);
        }

        let mut used: Vec<String> = roles.iter().map(|(_, n, _)| n.clone()).collect();
        let free = |used: &[String], c: &&Column| !used.contains(&c.name);

        // x
        let x = match &xs {
            Some(list) => list.first().map(|(n, _)| n.clone()),
            None => {
                let pick = |pred: &dyn Fn(&Column) -> bool| {
                    cols.iter()
                        .filter(|c| free(&used, c))
                        .find(|c| pred(c))
                        .map(|c| c.name.clone())
                };
                match kind {
                    ChartKind::Timechart => pick(&|c| is_typed(c) && c.ty == Type::DateTime),
                    ChartKind::Linechart | ChartKind::Areachart => {
                        pick(&|c| is_typed(c) && c.ty == Type::DateTime)
                            .or_else(|| pick(&y_accepts))
                    }
                    ChartKind::Scatterchart => pick(&y_accepts),
                    _ => pick(&|c| is_typed(c) && c.ty == Type::String),
                }
            }
        };
        let mut missing = false;
        match &x {
            Some(x) => {
                if xs.is_none() {
                    used.push(x.clone());
                }
            }
            None => {
                let needs = match kind {
                    ChartKind::Barchart | ChartKind::Columnchart | ChartKind::Piechart => {
                        "a `string` column"
                    }
                    _ => x_needs(kind),
                };
                let help = match kind {
                    ChartKind::Barchart | ChartKind::Columnchart | ChartKind::Piechart => {
                        "name the x-axis column with `xcolumn = ...`; it can have any type"
                    }
                    _ => "convert a column with `extend` before `render`",
                };
                self.missing_role(kind, kind_span, needs, Role::X, cols, &used, help);
                missing = true;
            }
        }

        // y
        let y: Vec<String> = match &ys {
            Some(list) => list.iter().map(|(n, _)| n.clone()).collect(),
            None => {
                let mut candidates = cols.iter().filter(|c| free(&used, c) && y_accepts(c));
                if kind == ChartKind::Piechart {
                    candidates
                        .next()
                        .map(|c| c.name.clone())
                        .into_iter()
                        .collect()
                } else {
                    candidates.map(|c| c.name.clone()).collect()
                }
            }
        };
        if y.is_empty() {
            self.missing_role(
                kind,
                kind_span,
                "a numeric column",
                Role::Y,
                cols,
                &used,
                "compute one with `extend` or `summarize` before `render`",
            );
            missing = true;
        }
        if missing {
            return None;
        }
        if ys.is_none() {
            used.extend(y.iter().cloned());
        }

        // series
        if kind.has_series() {
            let series = match &ss {
                Some(list) => list.iter().map(|(n, _)| n.clone()).collect(),
                None if matches!(
                    kind,
                    ChartKind::Timechart | ChartKind::Linechart | ChartKind::Areachart
                ) =>
                {
                    cols.iter()
                        .filter(|c| free(&used, c) && is_typed(c) && c.ty == Type::String)
                        .map(|c| c.name.clone())
                        .collect()
                }
                None => Vec::new(),
            };
            spec.series = Some(series);
        }
        spec.x = x;
        spec.y = Some(y);
        Some(spec)
    }

    /// `DS5002`: a required role that the defaults cannot fill.
    #[allow(clippy::too_many_arguments)]
    fn missing_role(
        &mut self,
        kind: ChartKind,
        kind_span: Span,
        needs: &str,
        role: Role,
        cols: &[Column],
        used: &[String],
        help: &str,
    ) {
        let fits = |c: &Column| match role {
            Role::Y => y_accepts(c),
            _ => match kind {
                ChartKind::Barchart | ChartKind::Columnchart | ChartKind::Piechart => {
                    is_typed(c) && c.ty == Type::String
                }
                _ => x_accepts(kind, c),
            },
        };
        let has_some = cols.iter().any(|c| fits(c) && used.contains(&c.name));
        let tail = if has_some {
            "every one already fills another role"
        } else {
            "the result has none"
        };
        self.diagnostics.push(
            Diagnostic::error(
                DS5002,
                format!("`{kind}` needs {needs} for {}, but {tail}", role.text()),
            )
            .primary(kind_span, format!("no column for {}", role.text()))
            .help(help.to_string())
            .note(columns_note(cols)),
        );
    }

    fn token_span(&self, token: &datascript_syntax::SyntaxToken) -> Span {
        let range = token.text_range();
        Span::new(self.file, range.start().into(), range.end().into())
    }
}

/// Where a misplaced `render`'s pipeline is, for `DS5008`.
fn render_place(pipeline: &SyntaxNode) -> &'static str {
    for node in pipeline.ancestors().skip(1) {
        match node.kind() {
            SyntaxKind::LET_STMT => return "in a tabular `let`",
            SyntaxKind::JOIN_OP => return "on the right side of `join`",
            SyntaxKind::UNION_OP => return "in a table of `union`",
            SyntaxKind::PIPELINE => return "in a pipeline in parentheses",
            _ => {}
        }
    }
    "in a pipeline in parentheses"
}
