//! The grammar: LANGUAGE.md §8, one function per construct.
//!
//! Keywords other than the seven reserved words are contextual (D-013): the parser checks an
//! identifier's text where the grammar expects a keyword, and the keyword stays an `IDENT` token
//! in the tree. Operator names are recognized only right after `|`.

use datascript_diagnostics::{Diagnostic, codes};

use super::{CompletedMarker, Parser, TokenSet};
use crate::SyntaxKind::*;

/// The operators of the language today, in alphabetical order.
pub(crate) const OPERATORS: &[&str] = &[
    "count",
    "distinct",
    "extend",
    "getschema",
    "join",
    "limit",
    "order",
    "project",
    "project-away",
    "project-rename",
    "render",
    "sort",
    "summarize",
    "take",
    "top",
    "union",
    "where",
];

/// Operators that LANGUAGE.md plans for a later milestone (○). They parse as errors for now,
/// with a note saying so.
pub(crate) const LATER_OPERATORS: &[&str] = &[
    "as",
    "lookup",
    "make-series",
    "mv-expand",
    "parse",
    "print",
    "project-reorder",
    "sample",
];

/// Operator names written with a hyphen, joined from `ident` `-` `ident` (D-026).
const HYPHENATED_OPERATORS: &[&str] = &[
    "make-series",
    "mv-expand",
    "project-away",
    "project-rename",
    "project-reorder",
];

/// Type names that form a typed null with `(null)` (LANGUAGE.md §1). `datetime(null)` and
/// `timespan(null)` are literal tokens from the lexer.
const TYPED_NULL_TYPES: &[&str] = &[
    "bool", "decimal", "dynamic", "guid", "int", "long", "real", "string",
];

const JOIN_KINDS: &str = "`inner`, `leftouter`, `rightouter`, `fullouter`, `leftsemi`, \
                          `leftanti`, `rightsemi` and `rightanti`";

const RENDER_KINDS: &str = "`table`, `timechart`, `linechart`, `areachart`, `barchart`, \
                            `columnchart`, `piechart`, `scatterchart` and `card`";

const LITERALS: TokenSet = TokenSet::new(&[
    LONG, REAL, STRING, TIMESPAN, DATETIME, TRUE_KW, FALSE_KW, NULL_KW,
]);

/// Tokens that can start an expression (and so a source).
const EXPR_START: TokenSet = TokenSet::new(&[
    LONG,
    REAL,
    STRING,
    TIMESPAN,
    DATETIME,
    TRUE_KW,
    FALSE_KW,
    NULL_KW,
    IDENT,
    QUOTED_IDENT,
    DOLLAR_LEFT,
    DOLLAR_RIGHT,
    L_PAREN,
    MINUS,
    NOT_KW,
]);

const NAME_START: TokenSet = TokenSet::new(&[IDENT, QUOTED_IDENT]);

// Binding powers, LANGUAGE.md §4 (higher binds tighter).
const BP_OR: u8 = 1;
const BP_AND: u8 = 2;
const BP_NOT: u8 = 3;
const BP_CMP: u8 = 4;
const BP_ADD: u8 = 5;
const BP_MUL: u8 = 6;
const BP_NEG: u8 = 7;

impl Parser<'_> {
    /// The operator name at the current token and how many tokens it spans (3 for a hyphenated
    /// name whose tokens are adjacent).
    fn op_name(&self) -> Option<(String, usize)> {
        if !self.at(IDENT) {
            return None;
        }
        let first = self.nth_text(0);
        if self.nth(1) == MINUS && self.nth_joined(1) && self.nth(2) == IDENT && self.nth_joined(2)
        {
            let name = format!("{first}-{}", self.nth_text(2));
            if HYPHENATED_OPERATORS.contains(&name.as_str()) {
                return Some((name, 3));
            }
        }
        Some((first.to_string(), 1))
    }

    /// Whether the current token is a known operator name at the start of a line, where a `|`
    /// was probably forgotten. The parser resynchronizes there.
    fn at_operator_on_new_line(&self) -> bool {
        self.at_line_start()
            && self
                .op_name()
                .is_some_and(|(name, _)| OPERATORS.contains(&name.as_str()))
    }

    /// Whether the current token probably starts a new statement: something that can start an
    /// expression, first on its line, outside any brackets, and not an operator name.
    fn at_new_statement(&self) -> bool {
        self.depth == 0
            && self.at_line_start()
            && self.at_set(EXPR_START)
            && !self.at_operator_on_new_line()
    }

    /// Whether the current token ends a pipeline: `;`, `let`, the end, or (inside brackets) `)`
    /// or `,`.
    fn at_pipeline_end(&self) -> bool {
        match self.current() {
            SEMICOLON | LET_KW | EOF => true,
            R_PAREN | COMMA => self.depth > 0,
            _ => false,
        }
    }

    /// `by` as a keyword (not `by = ...`, which names an output column `by`).
    fn at_by(&self) -> bool {
        self.at_kw("by") && self.nth(1) != EQ
    }

    fn error_expected_expr(&mut self) {
        let what = format!("an expression{}", self.after_prev());
        self.error_expected(&what);
    }

    /// Reports a missing name, with help when the user wrote a reserved word.
    fn error_expected_name(&mut self, what: &str) {
        let mut d = self.expected(what);
        if self.current().is_reserved_word() {
            let word = self.nth_text(0);
            d = d.help(format!(
                "`{word}` is a reserved word; to use it as a name, quote it: `['{word}']`"
            ));
        }
        self.report(d);
    }
}

// --- Statements ---

pub(super) fn program(p: &mut Parser<'_>) {
    let m = p.start();
    loop {
        if p.at(EOF) {
            break;
        }
        if p.eat(SEMICOLON) {
            continue;
        }
        if !statement(p) {
            p.error_expected("a query or a `let` statement");
            p.recover(|p| p.at(SEMICOLON) || p.at(LET_KW) || p.at_new_statement());
            continue;
        }
        if p.eat(SEMICOLON) || p.at(EOF) {
            continue;
        }
        p.error_expected("`;` between statements");
        if !(p.at(LET_KW) || p.at_new_statement()) {
            p.recover(|p| p.at(SEMICOLON) || p.at(LET_KW) || p.at_new_statement());
        }
    }
    m.complete(p, PROGRAM);
}

/// Parses one statement. Returns `false`, consuming nothing, if no statement starts here.
fn statement(p: &mut Parser<'_>) -> bool {
    if p.at(LET_KW) {
        let_stmt(p);
    } else if p.at_set(EXPR_START) || p.at(PIPE) {
        query_stmt(p);
    } else {
        return false;
    }
    true
}

fn let_stmt(p: &mut Parser<'_>) {
    let m = p.start();
    p.bump(); // let
    if p.at_set(NAME_START) {
        name(p);
    } else {
        p.error_expected_name("a name after `let`");
    }
    if !p.eat(EQ) {
        let what = format!("`=`{}", p.after_prev());
        p.error_expected(&what);
    }
    pipeline_or_expr(p);
    m.complete(p, LET_STMT);
}

/// A query statement: always a `PIPELINE`, even without operators.
fn query_stmt(p: &mut Parser<'_>) {
    let m = p.start();
    if p.at(PIPE) {
        let d = Diagnostic::error(codes::DS0101, "expected a table before `|`")
            .primary(p.current_span(), "this pipe has nothing to read from")
            .help("start the query with a file path such as `'data.csv'`, or a table name");
        p.report(d);
    } else {
        expr(p);
    }
    pipeline_tail(p);
    m.complete(p, PIPELINE);
}

/// An expression, wrapped in a `PIPELINE` if a `|` follows. Used where the parser cannot tell
/// a table from a scalar: the right side of `let`, inside parentheses, and call arguments.
fn pipeline_or_expr(p: &mut Parser<'_>) -> bool {
    let Some(source) = expr(p) else {
        return false;
    };
    if p.at(PIPE) {
        let m = source.precede(p);
        pipeline_tail(p);
        m.complete(p, PIPELINE);
    }
    true
}

/// `{ '|' operator }` after a source, with recovery.
fn pipeline_tail(p: &mut Parser<'_>) {
    loop {
        if p.eat(PIPE) {
            operator(p);
            continue;
        }
        if p.at_pipeline_end() {
            break;
        }
        if p.at_operator_on_new_line() {
            p.error_expected("`|` before the next operator");
            operator(p);
            continue;
        }
        if p.at_new_statement() {
            break;
        }
        let what = if p.depth > 0 {
            "`|` or `)`"
        } else {
            "`|` or `;`"
        };
        p.error_expected(what);
        p.recover(|p| {
            p.at(PIPE) || p.at_pipeline_end() || p.at_operator_on_new_line() || p.at_new_statement()
        });
    }
}

// --- Operators ---

fn operator(p: &mut Parser<'_>) {
    let known = p
        .op_name()
        .filter(|(name, _)| OPERATORS.contains(&name.as_str()));
    let Some((name, len)) = known else {
        unknown_operator(p);
        return;
    };
    let m = p.start();
    for _ in 0..len {
        p.bump();
    }
    let kind = match name.as_str() {
        "where" => {
            expr(p);
            WHERE_OP
        }
        "project" => {
            list(p, named_expr);
            PROJECT_OP
        }
        "extend" => {
            list(p, named_expr);
            EXTEND_OP
        }
        "project-away" => {
            list(p, column_pattern);
            PROJECT_AWAY_OP
        }
        "project-rename" => {
            list(p, rename_item);
            PROJECT_RENAME_OP
        }
        "summarize" => {
            summarize_body(p);
            SUMMARIZE_OP
        }
        "sort" | "order" => {
            if !p.eat_kw("by") {
                p.error_expected(&format!("`by` after `{name}`"));
            }
            list(p, sort_key);
            SORT_OP
        }
        "top" => {
            expr(p);
            if !p.eat_kw("by") {
                let what = format!("`by`{}", p.after_prev());
                p.error_expected(&what);
            }
            sort_key(p);
            TOP_OP
        }
        "take" | "limit" => {
            expr(p);
            TAKE_OP
        }
        "count" => COUNT_OP,
        "getschema" => GETSCHEMA_OP,
        "distinct" => {
            if !p.eat(STAR) {
                list(p, |p| {
                    expr(p);
                });
            }
            DISTINCT_OP
        }
        "join" => {
            join_body(p);
            JOIN_OP
        }
        "union" => {
            list(p, right_table);
            UNION_OP
        }
        "render" => {
            render_body(p);
            RENDER_OP
        }
        _ => unreachable!("every name in OPERATORS has a branch"),
    };
    m.complete(p, kind);
}

fn unknown_operator(p: &mut Parser<'_>) {
    let mut d = p.expected("an operator after `|`");
    if let Some((name, len)) = p.op_name() {
        if len > 1 {
            // A hyphenated name: report it whole.
            let start = p.tokens[p.pos].start;
            let end = p.tokens[p.pos + len - 1].end();
            d = Diagnostic::error(
                codes::DS0101,
                format!("expected an operator after `|`, found `{name}`"),
            )
            .primary(p.span(start, end), "");
        }
        if LATER_OPERATORS.contains(&name.as_str()) {
            d = d.note(format!("`{name}` is planned but not available yet"));
        } else {
            d = d.note(format!("the operators are {}", word_list(OPERATORS)));
        }
    }
    p.report(d);
    // A pipe, `;` or the end right after `|` is a missing operator: leave it for the caller.
    if p.at(PIPE) || p.at_pipeline_end() || p.at(R_PAREN) {
        return;
    }
    p.recover(|p| p.at(PIPE) || p.at_pipeline_end() || p.at_operator_on_new_line());
}

/// `item { ',' item }`
fn list(p: &mut Parser<'_>, item: impl Fn(&mut Parser<'_>)) {
    loop {
        item(p);
        if !p.eat(COMMA) {
            break;
        }
    }
}

/// `[name '='] expr`
fn named_expr(p: &mut Parser<'_>) {
    let m = p.start();
    if p.at_set(NAME_START) && p.nth(1) == EQ {
        name(p);
        p.bump(); // =
    }
    // Unlike in call arguments, a `|` here belongs to the outer pipeline.
    expr(p);
    m.complete_if_nonempty(p, NAMED_EXPR);
}

/// A column name with `*` wildcards, written without spaces: `a`, `col*`, `*_id`.
fn column_pattern(p: &mut Parser<'_>) {
    const PARTS: TokenSet = TokenSet::new(&[IDENT, QUOTED_IDENT, STAR, LONG]);
    if !(p.at_set(NAME_START) || p.at(STAR)) {
        let what = format!("a column name{}", p.after_prev());
        p.error_expected_name(&what);
        return;
    }
    let m = p.start();
    p.bump();
    while p.at_set(PARTS) && p.nth_joined(0) {
        p.bump();
    }
    m.complete(p, COLUMN_PATTERN);
}

/// `new = old` in `project-rename`.
fn rename_item(p: &mut Parser<'_>) {
    let m = p.start();
    if p.at_set(NAME_START) {
        name(p);
    } else {
        let what = format!("a new column name{}", p.after_prev());
        p.error_expected_name(&what);
    }
    if !p.eat(EQ) {
        let what = format!("`=`{}", p.after_prev());
        p.error_expected(&what);
    }
    if p.at_set(NAME_START) {
        name_ref(p);
    } else {
        p.error_expected_name("the column to rename after `=`");
    }
    m.complete_if_nonempty(p, RENAME_ITEM);
}

/// `[agg { ',' agg }] ['by' key { ',' key }]`
fn summarize_body(p: &mut Parser<'_>) {
    if !p.at_by() && p.at_set(EXPR_START) {
        list(p, |p| {
            if p.at_by() {
                p.error_expected_expr();
            } else {
                named_expr(p);
            }
        });
    }
    if p.at_by() {
        let m = p.start();
        p.bump(); // by
        list(p, named_expr);
        m.complete(p, BY_CLAUSE);
    }
}

/// `expr ['asc' | 'desc'] ['nulls' ('first' | 'last')]`
fn sort_key(p: &mut Parser<'_>) {
    let m = p.start();
    expr(p);
    if !p.eat_kw("asc") {
        p.eat_kw("desc");
    }
    if p.eat_kw("nulls") && !p.eat_kw("first") && !p.eat_kw("last") {
        p.error_expected("`first` or `last` after `nulls`");
    }
    m.complete_if_nonempty(p, SORT_KEY);
}

/// `['kind' '=' ident] right_table 'on' cond { ',' cond }`
fn join_body(p: &mut Parser<'_>) {
    if p.at_kw("kind") && p.nth(1) == EQ {
        let m = p.start();
        p.bump(); // kind
        p.bump(); // =
        if !p.eat(IDENT) {
            let d = p
                .expected("a join kind after `kind =`")
                .note(format!("the join kinds are {JOIN_KINDS}"));
            p.report(d);
        }
        m.complete(p, JOIN_KIND);
    }
    right_table(p);
    if p.eat_kw("on") {
        list(p, |p| {
            expr(p);
        });
    } else {
        p.error_expected("`on` and the columns to join on");
    }
}

/// The table on the right of `join` or `union`: a name, a path string, or a parenthesized
/// pipeline. Any expression parses here; the binder rejects the ones that are not tables.
fn right_table(p: &mut Parser<'_>) {
    if p.at_set(EXPR_START) {
        expr(p);
    } else {
        let what = format!("a table{}", p.after_prev());
        p.error_expected(&what);
    }
}

/// `kind ['with' '(' prop { ',' prop } ')']`
fn render_body(p: &mut Parser<'_>) {
    let at_with = p.at_kw("with") && p.nth(1) == L_PAREN;
    if p.at(IDENT) && !at_with {
        p.bump();
    } else {
        let d = p
            .expected("a chart kind after `render`")
            .note(format!("the chart kinds are {RENDER_KINDS}"));
        p.report(d);
    }
    if !p.at_kw("with") {
        return;
    }
    let m = p.start();
    p.bump(); // with
    if p.at(L_PAREN) {
        let open = p.pos;
        p.bump();
        p.depth += 1;
        if !p.at(R_PAREN) {
            list(p, render_prop);
        }
        expect_closing(p, open);
        p.depth -= 1;
    } else {
        p.error_expected("`(` after `with`");
    }
    m.complete(p, RENDER_PROPS);
}

/// `name '=' value { ',' value }`. Extra values are for list properties such as `ycolumns`;
/// a `name =` after a comma starts the next property.
fn render_prop(p: &mut Parser<'_>) {
    if !(p.at_set(NAME_START) && p.nth(1) == EQ) {
        p.error_expected("a property such as `title = \"...\"`");
        return;
    }
    let m = p.start();
    name(p);
    p.bump(); // =
    expr(p);
    while p.at(COMMA) && !(NAME_START.contains(p.nth(1)) && p.nth(2) == EQ) && p.nth(1) != R_PAREN {
        p.bump(); // ,
        expr(p);
    }
    m.complete(p, RENDER_PROP);
}

// --- Expressions ---

fn name(p: &mut Parser<'_>) {
    let m = p.start();
    p.bump();
    m.complete(p, NAME);
}

fn name_ref(p: &mut Parser<'_>) -> CompletedMarker {
    let m = p.start();
    p.bump();
    m.complete(p, NAME_REF)
}

/// A scalar expression (LANGUAGE.md §4). Reports an error and returns `None`, consuming nothing,
/// if no expression starts here.
fn expr(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    expr_bp(p, BP_OR)
}

#[derive(Clone, Copy)]
enum Infix {
    /// A binary operator spelled with this many tokens (`matches regex` has two).
    Binary(usize),
    In,
    Between,
}

/// The infix operator at the current token and its binding power.
fn infix(p: &Parser<'_>) -> Option<(u8, Infix)> {
    let op = match p.current() {
        OR_KW => (BP_OR, Infix::Binary(1)),
        AND_KW => (BP_AND, Infix::Binary(1)),
        EQ2 | NEQ | LT | LTEQ | GT | GTEQ | EQ_TILDE | BANG_TILDE => (BP_CMP, Infix::Binary(1)),
        IN_TILDE => (BP_CMP, Infix::In),
        NEGATED_OP => match p.nth_text(0) {
            "!in" | "!in~" => (BP_CMP, Infix::In),
            "!between" => (BP_CMP, Infix::Between),
            _ => (BP_CMP, Infix::Binary(1)),
        },
        IDENT => match p.nth_text(0) {
            "in" => (BP_CMP, Infix::In),
            "between" => (BP_CMP, Infix::Between),
            "has" | "has_cs" | "contains" | "contains_cs" | "startswith" | "startswith_cs"
            | "endswith" | "endswith_cs" => (BP_CMP, Infix::Binary(1)),
            "matches" if p.nth_at_kw(1, "regex") => (BP_CMP, Infix::Binary(2)),
            _ => return None,
        },
        PLUS | MINUS => (BP_ADD, Infix::Binary(1)),
        STAR | SLASH | PERCENT => (BP_MUL, Infix::Binary(1)),
        _ => return None,
    };
    Some(op)
}

/// A comparison already parsed in the current chain, for the "cannot be chained" help.
struct Comparison {
    /// Byte range of the whole left operand and of the right operand.
    start: u32,
    rhs: (u32, u32),
    simple: bool,
}

/// How deeply expressions may nest. The parser is recursive; this keeps it well inside the stack
/// (including wasm32's 1 MiB default and 2 MiB test threads) on input such as 100,000 `(`.
const MAX_NESTING: u32 = 256;

fn expr_bp(p: &mut Parser<'_>, min_bp: u8) -> Option<CompletedMarker> {
    if p.nesting >= MAX_NESTING {
        return too_deep(p);
    }
    p.nesting += 1;
    let result = expr_bp_inner(p, min_bp);
    p.nesting -= 1;
    result
}

/// Reports DS0100 and skips the rest of the statement without recursing. Stopping at a `|` would
/// let the innermost parenthesis take the outer pipeline's operators, and nest too deeply again.
fn too_deep(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let d = Diagnostic::error(codes::DS0100, "expression is nested too deeply")
        .primary(
            p.current_span(),
            format!("more than {MAX_NESTING} levels deep here"),
        )
        .help("split it into smaller parts with `let`");
    p.report(d);
    if p.at(EOF) {
        return None;
    }
    let m = p.start();
    while !(p.at(EOF) || p.at(SEMICOLON) || p.at(LET_KW)) {
        p.bump();
    }
    // Every enclosing bracket is now unclosed; one error is enough.
    p.last_error_at = Some(p.pos);
    Some(m.complete(p, ERROR))
}

fn expr_bp_inner(p: &mut Parser<'_>, min_bp: u8) -> Option<CompletedMarker> {
    let start = p.offset();
    let mut lhs = prefix_or_primary(p)?;
    let mut previous: Option<Comparison> = None;
    while let Some((bp, op)) = infix(p) {
        if bp < min_bp {
            break;
        }
        let op_start = p.pos;
        let m = lhs.precede(p);
        let (kind, rhs) = match op {
            Infix::Binary(len) => {
                for _ in 0..len {
                    p.bump();
                }
                let rhs_start = p.offset();
                expr_bp(p, bp + 1);
                (BIN_EXPR, (rhs_start, p.prev_end().max(rhs_start)))
            }
            Infix::In => {
                p.bump();
                let rhs_start = p.offset();
                if p.at(L_PAREN) {
                    arg_list(p);
                } else {
                    let what = format!("`(` and a list of values{}", p.after_prev());
                    p.error_expected(&what);
                }
                (IN_EXPR, (rhs_start, p.prev_end().max(rhs_start)))
            }
            Infix::Between => {
                p.bump();
                let rhs_start = p.offset();
                between_bounds(p);
                (BETWEEN_EXPR, (rhs_start, p.prev_end().max(rhs_start)))
            }
        };
        lhs = m.complete(p, kind);
        if bp == BP_CMP {
            // The help rewrites the chain only for plain operators with both operands present.
            let simple = matches!(op, Infix::Binary(_)) && rhs.0 < rhs.1;
            if let Some(first) = &previous {
                report_chained_comparison(p, first, op_start, rhs, simple);
            }
            previous = Some(Comparison { start, rhs, simple });
        }
    }
    Some(lhs)
}

/// `a < b < c`: comparisons do not chain (LANGUAGE.md §4).
fn report_chained_comparison(
    p: &mut Parser<'_>,
    first: &Comparison,
    op_index: usize,
    rhs: (u32, u32),
    simple: bool,
) {
    let op = p.tokens[op_index];
    let op_end = if op.kind == IDENT && op.text(p.text) == "matches" {
        p.tokens[op_index + 1].end()
    } else {
        op.end()
    };
    let mut d = Diagnostic::error(codes::DS0101, "comparison operators cannot be chained")
        .primary(p.span(op.start, op_end), "a second comparison")
        .secondary(
            p.span(first.start, first.rhs.1),
            "the result of this comparison is not a value to compare again",
        );
    if first.simple && simple {
        let src = p.text;
        let left = &src[first.start as usize..first.rhs.1 as usize];
        let middle = &src[first.rhs.0 as usize..first.rhs.1 as usize];
        let op_text = &src[op.start as usize..op_end as usize];
        let right = &src[rhs.0 as usize..rhs.1 as usize];
        d = d.help(format!(
            "compare each pair separately: `{left} and {middle} {op_text} {right}`"
        ));
    } else {
        d = d.help("combine separate comparisons with `and`");
    }
    // The operator token parsed fine, so it has no error yet: report without suppression.
    p.errors.push(d);
}

fn prefix_or_primary(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    match p.current() {
        NOT_KW if p.nth(1) == L_PAREN => {
            // `not(x)` reads like a call and binds like one: `not(x) == y` is `(not x) == y`.
            let m = p.start();
            p.bump();
            paren_expr(p);
            let not = m.complete(p, PREFIX_EXPR);
            Some(postfix(p, not))
        }
        NOT_KW => {
            let m = p.start();
            p.bump();
            expr_bp(p, BP_NOT);
            Some(m.complete(p, PREFIX_EXPR))
        }
        MINUS => {
            let m = p.start();
            p.bump();
            expr_bp(p, BP_NEG);
            Some(m.complete(p, PREFIX_EXPR))
        }
        _ => {
            let primary = primary(p)?;
            Some(postfix(p, primary))
        }
    }
}

fn primary(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let kind = p.current();
    if LITERALS.contains(kind) {
        let m = p.start();
        p.bump();
        return Some(m.complete(p, LITERAL));
    }
    match kind {
        IDENT if at_typed_null(p) => {
            let m = p.start();
            for _ in 0..4 {
                p.bump(); // type ( null )
            }
            Some(m.complete(p, TYPED_NULL))
        }
        IDENT if p.nth(1) == L_PAREN => {
            let callee = name_ref(p);
            let m = callee.precede(p);
            arg_list(p);
            Some(m.complete(p, CALL_EXPR))
        }
        IDENT | QUOTED_IDENT | DOLLAR_LEFT | DOLLAR_RIGHT => Some(name_ref(p)),
        L_PAREN => Some(paren_expr(p)),
        _ => {
            p.error_expected_expr();
            None
        }
    }
}

/// `long(null)`: a type name directly followed by `(`, then `null` and `)`.
fn at_typed_null(p: &Parser<'_>) -> bool {
    TYPED_NULL_TYPES.contains(&p.nth_text(0))
        && p.nth(1) == L_PAREN
        && p.nth_joined(1)
        && p.nth(2) == NULL_KW
        && p.nth(3) == R_PAREN
}

/// Member access `a.b` and indexing `a[0]`.
fn postfix(p: &mut Parser<'_>, mut lhs: CompletedMarker) -> CompletedMarker {
    loop {
        match p.current() {
            DOT => {
                let m = lhs.precede(p);
                p.bump();
                if p.at_set(NAME_START) {
                    name_ref(p);
                } else {
                    p.error_expected_name("a name after `.`");
                }
                lhs = m.complete(p, FIELD_EXPR);
            }
            L_BRACK => {
                let m = lhs.precede(p);
                let open = p.pos;
                p.bump();
                p.depth += 1;
                expr(p);
                expect_closing(p, open);
                p.depth -= 1;
                lhs = m.complete(p, INDEX_EXPR);
            }
            _ => return lhs,
        }
    }
}

fn paren_expr(p: &mut Parser<'_>) -> CompletedMarker {
    let m = p.start();
    let open = p.pos;
    p.bump(); // (
    p.depth += 1;
    pipeline_or_expr(p);
    expect_closing(p, open);
    p.depth -= 1;
    m.complete(p, PAREN_EXPR)
}

/// `'(' [arg { ',' arg }] ')'`, where an argument is `[name '='] value`.
fn arg_list(p: &mut Parser<'_>) {
    let m = p.start();
    let open = p.pos;
    p.bump(); // (
    p.depth += 1;
    if !p.at(R_PAREN) {
        list(p, |p| {
            if p.at_set(NAME_START) && p.nth(1) == EQ {
                let m = p.start();
                name(p);
                p.bump(); // =
                pipeline_or_expr(p);
                m.complete(p, NAMED_EXPR);
            } else {
                pipeline_or_expr(p);
            }
        });
    }
    expect_closing(p, open);
    p.depth -= 1;
    m.complete(p, ARG_LIST);
}

/// `'(' lo '..' hi ')'` after `between`.
fn between_bounds(p: &mut Parser<'_>) {
    if !p.at(L_PAREN) {
        let what = format!("`(`{}", p.after_prev());
        let d = p
            .expected(&what)
            .help("write the range in parentheses: `x between (1 .. 10)`");
        p.report(d);
        return;
    }
    let open = p.pos;
    p.bump();
    p.depth += 1;
    expr(p);
    if !p.eat(DOT2) {
        let d = p
            .expected("`..` between the two bounds")
            .help("write the range as `(low .. high)`");
        p.report(d);
    }
    expr(p);
    expect_closing(p, open);
    p.depth -= 1;
}

/// Expects the bracket that closes the one at token index `open`. If something else is there,
/// reports it, skips to the closing bracket if there is one, and consumes it.
fn expect_closing(p: &mut Parser<'_>, open: usize) {
    let open_token = p.tokens[open];
    let close = match open_token.kind {
        L_BRACK => R_BRACK,
        _ => R_PAREN,
    };
    if p.eat(close) {
        return;
    }
    let close_text = if close == R_BRACK { "]" } else { ")" };
    let open_text = if close == R_BRACK { "[" } else { "(" };
    let d = p.expected(&format!("`{close_text}`")).secondary(
        p.span(open_token.start, open_token.end()),
        format!("to close this `{open_text}`"),
    );
    p.report(d);
    if p.at(EOF) || p.at(SEMICOLON) || p.at(LET_KW) {
        return;
    }
    p.recover(|p| p.at(close) || p.at(SEMICOLON) || p.at(LET_KW));
    p.eat(close);
}

/// "`a`, `b` and `c`"
fn word_list(words: &[&str]) -> String {
    let quoted: Vec<String> = words.iter().map(|w| format!("`{w}`")).collect();
    match quoted.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
        Some((last, _)) => last.clone(),
        None => String::new(),
    }
}
