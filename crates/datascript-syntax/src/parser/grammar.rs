//! The grammar: LANGUAGE.md §8, one function per construct.
//!
//! Keywords other than the seven reserved words are contextual (D-013): the parser checks an
//! identifier's text where the grammar expects a keyword, and the keyword stays an `IDENT` token
//! in the tree. Operator names are recognized only right after `|`.

use datascript_diagnostics::{Diagnostic, codes};

use super::targeted::{JOIN_KINDS, PREDICATE_FUNCTIONS, is_close_bracket};
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

const RENDER_KINDS: &str = "`table`, `timechart`, `linechart`, `areachart`, `barchart`, \
                            `columnchart`, `piechart`, `scatterchart` and `card`";

const LITERALS: TokenSet = TokenSet::new(&[
    LONG, REAL, STRING, TIMESPAN, DATETIME, TRUE_KW, FALSE_KW, NULL_KW,
]);

/// Tokens that can start an expression (and so a source).
pub(super) const EXPR_START: TokenSet = TokenSet::new(&[
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

/// Words that continue an expression as an infix operator: after an operator name they mean the
/// name is a column (`take in (1, 2)`), not an operator.
const INFIX_WORDS: &[&str] = &[
    "between",
    "contains",
    "contains_cs",
    "endswith",
    "endswith_cs",
    "has",
    "has_cs",
    "in",
    "matches",
    "startswith",
    "startswith_cs",
];

const NAME_START: TokenSet = TokenSet::new(&[IDENT, QUOTED_IDENT]);

/// Tokens that can end a complete expression.
const ENDS_EXPR: TokenSet = TokenSet::new(&[
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
    R_PAREN,
    R_BRACK,
]);

// Binding powers, LANGUAGE.md §4 (higher binds tighter).
const BP_OR: u8 = 1;
const BP_AND: u8 = 2;
const BP_NOT: u8 = 3;
const BP_CMP: u8 = 4;
const BP_ADD: u8 = 5;
const BP_MUL: u8 = 6;
const BP_NEG: u8 = 7;

impl Parser<'_> {
    /// The operator name at the current token and how many tokens it spans: 3 for a hyphenated
    /// name. The flag says whether a hyphenated name has spaces in it (`project - away`, DS0104).
    fn op_name(&self) -> Option<(String, usize, bool)> {
        if !self.at(IDENT) {
            return None;
        }
        let first = self.nth_text(0);
        if self.nth(1) == MINUS && self.nth(2) == IDENT {
            let name = format!("{first}-{}", self.nth_text(2));
            if HYPHENATED_OPERATORS.contains(&name.as_str()) {
                let spaced = !(self.nth_joined(1) && self.nth_joined(2));
                return Some((name, 3, spaced));
            }
        }
        Some((first.to_string(), 1, false))
    }

    /// Whether the current token is a known operator name at the start of a line, where a `|`
    /// was probably forgotten. The parser resynchronizes there.
    fn at_operator_on_new_line(&self) -> bool {
        self.at_line_start() && self.at_known_operator()
    }

    /// Whether the current token is the name of an operator of the language today.
    fn at_known_operator(&self) -> bool {
        self.op_name()
            .is_some_and(|(name, _, spaced)| !spaced && OPERATORS.contains(&name.as_str()))
    }

    /// Whether `||` here is two pipes rather than a C-style `or`: an operator name follows it.
    fn at_double_pipe(&self) -> bool {
        self.at(PIPE2) && self.nth(1) == IDENT && {
            let name = self.nth_text(1);
            OPERATORS.contains(&name) || LATER_OPERATORS.contains(&name)
        }
    }

    /// Whether the current token probably starts a new statement: something that can start an
    /// expression, first on its line, outside any brackets, and not an operator name.
    fn at_new_statement(&self) -> bool {
        self.depth == 0
            && self.at_line_start()
            && self.at_set(EXPR_START)
            && !self.at_operator_on_new_line()
    }

    /// Whether the current token clearly starts a new statement although a bracket is still open:
    /// a line that looks like one ([`Parser::statement_after_unclosed_at`]) or the keyword `from`
    /// after a complete expression ([`Parser::query_start_at`]), however it is indented.
    fn at_statement_after_unclosed(&self) -> bool {
        self.statement_after_unclosed_at(self.pos) || self.query_start_at(self.pos)
    }

    /// Whether the token at index `i` is the keyword `from` right after a complete expression
    /// (D-037). `from` as a name cannot follow a complete expression, so the keyword there always
    /// starts a new statement, on any line and at any indentation. `f(a, from t)` and
    /// `range x from 1` are not this: a `,` or an unfinished generator comes before the `from`.
    fn query_start_at(&self, i: usize) -> bool {
        let (Some(token), Some(prev)) = (self.tokens.get(i), i.checked_sub(1)) else {
            return false;
        };
        let next = self.tokens.get(i + 1).map_or(EOF, |t| t.kind);
        token.kind == IDENT
            && token.text(self.text) == "from"
            && EXPR_START.contains(next)
            && !matches!(next, MINUS | NOT_KW)
            && ENDS_EXPR.contains(self.tokens[prev].kind)
    }

    /// Whether the current token is the keyword `from` in SQL's capitals at a pipeline head:
    /// `FROM orders` (DS0105).
    fn at_from_caps(&self) -> bool {
        self.at(IDENT)
            && self.nth_text(0) == "FROM"
            && EXPR_START.contains(self.nth(1))
            && !matches!(self.nth(1), MINUS | NOT_KW)
    }

    /// Whether the current token is a SQL keyword written where an operator goes, after a table
    /// but without `|`: `from t select a`, `from t GROUP BY a` (DS0153). Operator names of the
    /// language (`where`, `order`) are DS0102 instead, and `from` has its own rules.
    fn at_sql_clause(&self) -> bool {
        if !self.at(IDENT) || self.nth_line_start(1) {
            return false;
        }
        let lower = self.nth_text(0).to_lowercase();
        if lower == "from" || !super::targeted::SQL_WORDS.contains(&lower.as_str()) {
            return false;
        }
        let next = self.nth(1);
        let by = matches!(lower.as_str(), "group" | "order")
            && next == IDENT
            && self.nth_text(1).eq_ignore_ascii_case("by");
        let operand = next == STAR || (EXPR_START.contains(next) && !matches!(next, MINUS));
        // `t⏎SELECT a FROM u` is a whole SQL query on its own line (DS0105).
        (by || operand) && !(self.at_line_start() && sql_select_at(self).is_some())
    }

    /// Whether the token at index `i` clearly starts a new statement even inside brackets, so an
    /// unclosed bracket does not swallow it: it can start an expression, starts a line with no
    /// indentation, and follows a complete expression (`f(a⏎U | take 1`). A continuation such
    /// as `f(a,⏎b)` or `(a⏎- b)` does not count.
    fn statement_after_unclosed_at(&self, i: usize) -> bool {
        const CONTINUATIONS: &[&str] = &[
            "asc",
            "between",
            "by",
            "contains",
            "contains_cs",
            "desc",
            "endswith",
            "endswith_cs",
            "has",
            "has_cs",
            "in",
            "kind",
            "matches",
            "nulls",
            "on",
            "startswith",
            "startswith_cs",
            "with",
        ];
        let (Some(token), Some(prev)) = (self.tokens.get(i), i.checked_sub(1)) else {
            return false;
        };
        let unindented = token.start > 0
            && matches!(
                self.text.as_bytes()[token.start as usize - 1],
                b'\n' | b'\r'
            );
        unindented
            && EXPR_START.contains(token.kind)
            && !matches!(token.kind, MINUS | NOT_KW)
            && !(token.kind == IDENT && CONTINUATIONS.contains(&token.text(self.text)))
            && ENDS_EXPR.contains(self.tokens[prev].kind)
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

    /// Whether the current token is the keyword `from` at a pipeline head (D-036): the word
    /// `from` followed by something that can start an expression, other than `-` and `not`.
    /// Anywhere else `from` is an ordinary name (`let y = from - 1`, `where from > 3`).
    pub(super) fn at_from_kw(&self) -> bool {
        self.at_kw("from")
            && EXPR_START.contains(self.nth(1))
            && !matches!(self.nth(1), MINUS | NOT_KW)
    }

    /// Whether the current token is `from` with no table after it (DS0151): followed by `|`, or,
    /// where only a query can be (`statement`), also by `;`, `let` or the end. Elsewhere `from;`
    /// or `f(from)` reads a name `from`.
    fn at_empty_from(&self, statement: bool) -> bool {
        self.at_kw("from")
            && match self.nth(1) {
                PIPE => true,
                SEMICOLON | LET_KW | EOF => statement,
                _ => false,
            }
    }

    /// Whether the current token starts a row generator (`range x from 1 to 10 step 1`,
    /// `datatable(...) [...]`, `print x = 1`), which starts a query without `from` (D-036).
    /// `range(1, 10)` with no space is a call, and `range - 1` an expression.
    fn at_generator(&self) -> bool {
        if !self.at(IDENT) || self.nth_line_start(1) {
            return false;
        }
        let next = self.nth(1);
        match self.nth_text(0) {
            "datatable" => next == L_PAREN,
            "range" | "print" => {
                EXPR_START.contains(next)
                    && !matches!(next, MINUS | NOT_KW)
                    && !(next == L_PAREN && self.nth_joined(1))
            }
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

    /// Reports a missing name. Returns `true` if the user wrote a reserved word (DS0126): the
    /// caller then parses it as the name meant.
    fn error_expected_name(&mut self, what: &str) -> bool {
        if self.current().is_reserved_word() {
            self.reserved_word_as_name();
            return true;
        }
        self.error_expected(what);
        false
    }

    /// Whether a name followed by `=` starts here: a named column or argument. A reserved word
    /// counts (`extend null = 1`, DS0126).
    fn at_name_eq(&self) -> bool {
        (self.at_set(NAME_START) || self.current().is_reserved_word()) && self.nth(1) == EQ
    }

    /// Parses the name of `name =`, reporting a reserved word (DS0126).
    fn name_before_eq(&mut self) {
        if self.current().is_reserved_word() {
            self.reserved_word_as_name();
        }
        name(self);
    }

    /// Whether the current token is an operator name that starts a query with no table before
    /// it (`where a > 1`, DS0125): something follows it on the same line that can only be the
    /// operator's arguments.
    fn at_operator_without_source(&self) -> bool {
        let Some((name, len, spaced)) = self.op_name() else {
            return false;
        };
        if spaced || !OPERATORS.contains(&name.as_str()) || self.nth_line_start(len) {
            return false;
        }
        let next = self.nth(len);
        next == STAR
            || (EXPR_START.contains(next)
                && !(next == L_PAREN && self.nth_joined(len))
                && !(next == IDENT && INFIX_WORDS.contains(&self.nth_text(len))))
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
        p.last_op = None;
        if !statement(p) {
            if is_close_bracket(p.current()) {
                p.stray_close_bracket();
            } else {
                p.error_expected("a query or a `let` statement");
            }
            p.recover(|p| p.at(SEMICOLON) || p.at(LET_KW) || p.at_new_statement());
            continue;
        }
        if p.eat(SEMICOLON) || p.at(EOF) {
            continue;
        }
        if p.at(LET_KW) || p.at_new_statement() || p.at_from_kw() {
            p.missing_semicolon();
        } else {
            p.error_expected("`;` between statements");
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
    if p.at_set(NAME_START) || p.error_expected_name("a name after `let`") {
        name(p);
    }
    if !p.eat(EQ) {
        let what = format!("`=`{}", p.after_prev());
        p.error_expected(&what);
    }
    pipeline_or_expr(p, true);
    m.complete(p, LET_STMT);
}

/// A query statement: always a `PIPELINE`, even without operators.
fn query_stmt(p: &mut Parser<'_>) {
    if p.at_from_kw() || p.at_empty_from(true) {
        from_pipeline(p);
        return;
    }
    if p.at_from_caps() {
        p.sql_from_caps();
        from_pipeline(p);
        return;
    }
    if p.at_generator() {
        generator(p);
        return;
    }
    let m = p.start();
    let start = p.pos;
    if p.at(PIPE) {
        let d = Diagnostic::error(codes::DS0125, "a query starts with a table, not with `|`")
            .primary(p.current_span(), "this pipe has nothing to read from")
            .help(
                "start the query with `from` and a file path such as `from 'data.csv'`, or \
                 `from` and a table name",
            );
        p.report(d);
    } else if p.at_operator_without_source() {
        // `where a > 1`: parse the operator, so errors inside it show up too.
        p.operator_without_source();
        operator(p);
    } else if let Some((from, end)) = sql_select_at(p) {
        p.sql_select(p.pos, from, end);
        p.recover(|p| p.at(SEMICOLON) || p.at(LET_KW));
        m.complete(p, PIPELINE);
        return;
    } else {
        // A bare source (`orders | take 5`): DS0150, unless it is a pipeline in parentheses.
        let reported = p.errors_at_token(start);
        if let Some(source) = expr(p)
            && !reported
            && !is_paren_pipeline(p, source, start)
        {
            let head = head_kind(p, source, start);
            p.missing_from(start, head, p.at(PIPE) || p.at_double_pipe(), true);
        }
    }
    pipeline_tail(p, start);
    m.complete(p, PIPELINE);
}

/// What a pipeline written without `from` starts with, for how certain the DS0150 fix is.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Head {
    /// A string literal: a path.
    String,
    /// A name.
    Name,
    /// A call to a reader function (`read_csv(...)`, [`crate::READERS`]).
    Reader,
    /// A call to any other function.
    Call,
    /// Anything else.
    Other,
}

/// The kind of the expression `source`, which starts at token index `start`.
fn head_kind(p: &Parser<'_>, source: CompletedMarker, start: usize) -> Head {
    let first = p.tokens[start];
    match source.kind(p) {
        LITERAL if first.kind == STRING => Head::String,
        NAME_REF => Head::Name,
        CALL_EXPR
            if first.kind == IDENT
                && crate::READERS.contains(&first.text(p.text))
                && p.tokens.get(start + 1).is_some_and(|t| t.kind == L_PAREN) =>
        {
            Head::Reader
        }
        CALL_EXPR => Head::Call,
        _ => Head::Other,
    }
}

/// Whether `source`, which starts at token index `start`, is a pipeline in parentheses
/// (`(from t | where a)`): a table that needs no `from` of its own.
fn is_paren_pipeline(p: &Parser<'_>, source: CompletedMarker, start: usize) -> bool {
    source.kind(p) == PAREN_EXPR && p.paren_pipeline == Some(start)
}

/// A row generator at a pipeline head. None is available yet (M2-01 adds their grammar): the
/// generator is reported and skipped, and the operators after it parsed. The current token is
/// the generator's name.
fn generator(p: &mut Parser<'_>) {
    let m = p.start();
    let start = p.pos;
    p.generator_not_available();
    p.recover(|p| p.at(PIPE) || p.at_pipeline_end() || p.at_new_statement());
    pipeline_tail(p, start);
    m.complete(p, PIPELINE);
}

/// `SELECT ... FROM` at the start of a statement, in any case: the token index of `FROM` and the
/// index just past the statement (at `;`, `let` or the end, outside brackets).
fn sql_select_at(p: &Parser<'_>) -> Option<(usize, usize)> {
    if !(p.at(IDENT) && p.nth_text(0).eq_ignore_ascii_case("select")) {
        return None;
    }
    let mut from = None;
    let mut depth = 0u32;
    let mut i = p.pos + 1;
    while let Some(t) = p.tokens.get(i) {
        match t.kind {
            L_PAREN | L_BRACK => depth += 1,
            R_PAREN | R_BRACK => depth = depth.saturating_sub(1),
            SEMICOLON | LET_KW if depth == 0 => break,
            IDENT
                if depth == 0 && from.is_none() && t.text(p.text).eq_ignore_ascii_case("from") =>
            {
                from = Some(i);
            }
            _ => {}
        }
        i += 1;
    }
    from.map(|from| (from, i))
}

/// `from source { '|' operator }`: always a `PIPELINE`, even with no operators (D-036). The
/// `from` token is a direct child of the `PIPELINE` node, before the source. The current token
/// is `from`.
fn from_pipeline(p: &mut Parser<'_>) {
    let m = p.start();
    let start = p.pos;
    p.bump(); // from
    if p.at_set(EXPR_START) {
        expr(p);
    } else {
        p.empty_from();
    }
    pipeline_tail(p, start);
    m.complete(p, PIPELINE);
}

/// A pipeline or a scalar expression, where either may appear: the right side of `let`
/// (`in_let`), inside parentheses, and call arguments. `from` or a generator makes a pipeline
/// (D-036). Without them, an expression followed by `|` is a pipeline that forgot its `from`
/// (DS0150), and so is a reader call alone as a `let` value; anything else is a scalar.
/// Returns whether the result is a table: a `PIPELINE`, or one in parentheses.
fn pipeline_or_expr(p: &mut Parser<'_>, in_let: bool) -> bool {
    if p.at_from_kw() || p.at_empty_from(false) {
        from_pipeline(p);
        return true;
    }
    if p.at_from_caps() {
        p.sql_from_caps();
        from_pipeline(p);
        return true;
    }
    if p.at_generator() {
        generator(p);
        return true;
    }
    let start = p.pos;
    let reported = p.errors_at_token(start);
    let Some(source) = expr(p) else {
        return false;
    };
    let paren_pipeline = is_paren_pipeline(p, source, start);
    let piped = p.at(PIPE);
    let head = head_kind(p, source, start);
    if !(piped || (in_let && head == Head::Reader)) {
        return paren_pipeline;
    }
    // Inside a bracket that is never closed, DS0112 explains what went wrong.
    if !reported && !paren_pipeline && (p.depth == 0 || closing_ahead(p, R_PAREN)) {
        p.missing_from(start, head, piped, false);
    }
    let m = source.precede(p);
    pipeline_tail(p, start);
    m.complete(p, PIPELINE);
    true
}

/// One operator of a pipeline, for checks across operators (DS0120).
struct OpInfo {
    /// Where the text before the operator starts: the end of the previous token, so moving the
    /// operator takes the `|` and the space before it along.
    lead: u32,
    start: u32,
    end: u32,
    kind: Option<crate::SyntaxKind>,
}

/// `{ '|' operator }` after a source that starts at token index `start`, with recovery.
fn pipeline_tail(p: &mut Parser<'_>, start: usize) {
    let saved_start = std::mem::replace(&mut p.pipeline_start, start);
    let mut ops: Vec<OpInfo> = Vec::new();
    loop {
        let lead = p.prev_end();
        if p.at(PIPE) || p.at_double_pipe() {
            let pipe = p.pos;
            if p.at(PIPE2) {
                p.double_pipe_token();
            }
            p.bump();
            if p.at(PIPE) || p.at_pipeline_end() || p.at(R_PAREN) {
                p.pipe_without_operator(pipe);
                continue;
            }
            let start = p.offset();
            let kind = operator(p);
            ops.push(OpInfo {
                lead,
                start,
                end: p.prev_end(),
                kind,
            });
            continue;
        }
        if p.at_pipeline_end() {
            break;
        }
        if p.at_known_operator() {
            // A forgotten `|`, on a new line or the same one: `where x > 1 project a`.
            let name = p.nth_text(0).to_string();
            p.missing_pipe(&name);
            let start = p.offset();
            let kind = operator(p);
            ops.push(OpInfo {
                lead,
                start,
                end: p.prev_end(),
                kind,
            });
            continue;
        }
        if p.at_sql_clause() {
            // `from t select a`: SQL's clause order, without `|` (DS0153).
            let (name, len, _) = p.op_name().expect("at an identifier");
            let start = p.offset();
            let certain = p.sql_clause_without_pipe(&name, len);
            let kind = match certain {
                Some((name, len, by_included)) => Some(operator_named(p, name, len, by_included)),
                None => {
                    p.recover(|p| {
                        p.at(PIPE)
                            || p.at_pipeline_end()
                            || p.at_operator_on_new_line()
                            || p.at_new_statement()
                            || p.at_from_kw()
                    });
                    None
                }
            };
            ops.push(OpInfo {
                lead,
                start,
                end: p.prev_end(),
                kind,
            });
            continue;
        }
        // The keyword `from` after a complete expression starts a new statement wherever it is
        // (D-037); inside brackets, only if they are never closed.
        if p.at_new_statement()
            || (p.depth == 0 && p.at_from_kw())
            || (p.depth > 0 && p.at_statement_after_unclosed())
        {
            break;
        }
        if p.depth == 0 && is_close_bracket(p.current()) {
            p.stray_close_bracket();
            p.recover(|p| {
                p.at(PIPE)
                    || p.at_pipeline_end()
                    || p.at_operator_on_new_line()
                    || p.at_new_statement()
            });
            continue;
        }
        let what = if p.depth > 0 {
            "`|` or `)`"
        } else {
            "`|` or `;`"
        };
        p.error_expected(what);
        p.recover(|p| {
            p.at(PIPE)
                || p.at_pipeline_end()
                || p.at_operator_on_new_line()
                || p.at_new_statement()
                || (p.depth == 0 && p.query_start_at(p.pos))
        });
    }
    p.pipeline_start = saved_start;
    p.last_op = ops.last().and_then(|op| Some((op.kind?, op.start)));
    check_render_last(p, &ops);
}

/// DS0120: `render` must be the last operator of its pipeline.
fn check_render_last(p: &mut Parser<'_>, ops: &[OpInfo]) {
    let Some(i) = ops.iter().position(|op| op.kind == Some(RENDER_OP)) else {
        return;
    };
    let Some(next) = ops.get(i + 1) else {
        return;
    };
    let render = &ops[i];
    let renders = ops.iter().filter(|op| op.kind == Some(RENDER_OP)).count();
    let moved = (renders == 1).then_some((render.lead, render.end));
    let end = ops.last().map_or(render.end, |op| op.end);
    p.render_not_last(
        (render.start, render.end),
        (next.start, next.end),
        moved,
        end,
    );
}

// --- Operators ---

/// Parses one operator after `|` and returns its node kind (`None` for an unknown operator).
fn operator(p: &mut Parser<'_>) -> Option<crate::SyntaxKind> {
    let saved = std::mem::replace(&mut p.eq_compares, false);
    let kind = operator_inner(p);
    p.eq_compares = saved;
    kind
}

fn operator_inner(p: &mut Parser<'_>) -> Option<crate::SyntaxKind> {
    let known = p
        .op_name()
        .filter(|(name, _, _)| OPERATORS.contains(&name.as_str()));
    let Some((name, len, spaced)) = known else {
        unknown_operator(p);
        return None;
    };
    if spaced {
        p.spaced_operator_name(&name, len);
    }
    Some(operator_named(p, &name, len, false))
}

/// Parses an operator whose name, `name`, spans the next `len` tokens. `by_included` says the
/// name tokens already include `by` (`ORDER BY` read as `sort by`).
fn operator_named(
    p: &mut Parser<'_>,
    name: &str,
    len: usize,
    by_included: bool,
) -> crate::SyntaxKind {
    let op_start = p.offset();
    let m = p.start();
    for _ in 0..len {
        p.bump();
    }
    let kind = match name {
        "where" => {
            p.eq_compares = true;
            let start = p.offset();
            expr(p);
            where_commas(p, start);
            WHERE_OP
        }
        "project" => {
            // `project *` (DS0117): parse the columns after `*,` too.
            if !p.at(STAR) || project_star(p, op_start) {
                list(p, named_expr);
            }
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
            if by_included || p.eat_kw("by") {
                list(p, sort_key);
            } else {
                without_by(p, name, op_start, |p| list(p, sort_key));
            }
            SORT_OP
        }
        "top" => {
            expr(p);
            if p.eat_kw("by") {
                sort_key(p);
            } else {
                without_by(p, name, op_start, sort_key);
            }
            TOP_OP
        }
        "take" | "limit" => {
            expr(p);
            TAKE_OP
        }
        "count" => COUNT_OP,
        "getschema" => GETSCHEMA_OP,
        "distinct" => {
            let star = p.pos;
            let starred = p.eat(STAR);
            if !starred || p.at(COMMA) {
                if starred {
                    // `distinct *, a` (DS0116): parse the columns too.
                    p.distinct_star_with_columns(star);
                    p.bump();
                }
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
            list(p, |p| right_table(p, "union"));
            UNION_OP
        }
        "render" => {
            render_body(p);
            RENDER_OP
        }
        _ => unreachable!("every name in OPERATORS has a branch"),
    };
    m.complete(p, kind);
    kind
}

/// `where a, b` (DS0115): further conditions after commas, outside a call's arguments where the
/// comma starts the next argument. `start` is where the first condition starts.
fn where_commas(p: &mut Parser<'_>, start: u32) {
    if p.in_arg_list {
        return;
    }
    let mut commas = Vec::new();
    while p.at(COMMA) && EXPR_START.contains(p.nth(1)) {
        commas.push(p.pos);
        p.bump();
        expr(p);
    }
    if !commas.is_empty() {
        p.where_with_commas(&commas, start);
    }
}

/// `project *` (DS0117): SQL's `SELECT *`. Wraps `*` and a comma after it in an `ERROR` node, and
/// returns whether there was a comma, so more columns follow.
fn project_star(p: &mut Parser<'_>, op_start: u32) -> bool {
    let alone =
        matches!(p.nth(1), PIPE | SEMICOLON | LET_KW | EOF) || (p.nth(1) == R_PAREN && p.depth > 0);
    p.project_star(op_start, alone);
    let m = p.start();
    p.bump(); // *
    let comma = p.eat(COMMA);
    m.complete(p, ERROR);
    comma
}

/// `sort` or `top` without `by` (DS0121): parses the keys anyway if they are there.
fn without_by(p: &mut Parser<'_>, name: &str, op_start: u32, keys: impl Fn(&mut Parser<'_>)) {
    let at = p.pos;
    let has_keys = p.at_set(EXPR_START)
        && !p.at_line_start()
        && !["asc", "desc", "nulls"].contains(&p.nth_text(0));
    if has_keys {
        let start = p.offset();
        keys(p);
        let end = p.prev_end().max(start);
        p.missing_by(name, op_start, at, Some((start, end)));
    } else {
        p.missing_by(name, op_start, at, None);
    }
}

fn unknown_operator(p: &mut Parser<'_>) {
    if p.at(IDENT) && p.nth_text(0).eq_ignore_ascii_case("from") {
        from_after_pipe(p);
        return;
    }
    if let Some((from, end)) = sql_select_at(p) {
        // A whole SQL query after a pipe: `t | SELECT a FROM t`.
        p.sql_select(p.pipeline_start, from, end);
        p.recover(|p| p.at(SEMICOLON) || p.at(LET_KW));
        return;
    }
    let d = match p.op_name() {
        Some((name, len, spaced)) if spaced && LATER_OPERATORS.contains(&name.as_str()) => {
            p.spaced_operator_name(&name, len);
            None
        }
        Some((name, len, _)) => Some(p.unknown_operator_diagnostic(&name, len)),
        None => Some(
            p.expected("an operator after `|`")
                .note(format!("the operators are {}", word_list(OPERATORS))),
        ),
    };
    if let Some(d) = d {
        let certain = certain_operator(p, &d);
        p.report(d);
        if let Some((name, len, by_included)) = certain {
            // The fix is certain, so parse the rest as the operator meant: errors inside it
            // show up now, not after the fix.
            operator_named(p, name, len, by_included);
            return;
        }
    }
    // A pipe, `;` or the end right after `|` is a missing operator: leave it for the caller.
    if p.at(PIPE) || p.at_pipeline_end() || p.at(R_PAREN) {
        return;
    }
    p.recover(|p| p.at(PIPE) || p.at_pipeline_end() || p.at_operator_on_new_line());
}

/// `| from x` (DS0152): `from` is not an operator. The current token is `from`, in any case.
fn from_after_pipe(p: &mut Parser<'_>) {
    let before_operator = p.nth(1) == IDENT && OPERATORS.contains(&p.nth_text(1));
    p.report_from_after_pipe(before_operator);
    if before_operator {
        // `| from where x`: the fix removes `from`, so parse the operator after it.
        let m = p.start();
        p.bump(); // from
        m.complete(p, ERROR);
        operator(p);
        return;
    }
    if p.at_from_kw() && p.depth == 0 {
        // `| from x` most likely starts a new query: leave it for the statement loop, which
        // does not report the missing `;` again.
        return;
    }
    p.recover(|p| p.at(PIPE) || p.at_pipeline_end() || p.at_operator_on_new_line());
}

/// The operator a machine-applicable fix for an unknown operator name turns it into: the name,
/// how many tokens the fix replaces, and whether it includes `by`.
pub(super) fn certain_operator(
    p: &Parser<'_>,
    d: &Diagnostic,
) -> Option<(&'static str, usize, bool)> {
    let suggestion = d.help.first()?.suggestion.as_ref()?;
    let [edit] = suggestion.edits.as_slice() else {
        return None;
    };
    if suggestion.applicability != datascript_diagnostics::Applicability::MachineApplicable
        || edit.span.start != p.offset()
    {
        return None;
    }
    let (name, by_included) = match edit.replacement.strip_suffix(" by") {
        Some(name) => (name, true),
        None => (edit.replacement.as_str(), false),
    };
    let name = OPERATORS.iter().copied().find(|op| *op == name)?;
    let len = p.tokens[p.pos..]
        .iter()
        .take_while(|t| t.end() <= edit.span.end)
        .count();
    // `operator_named` bumps `len` tokens into the node; with none it would be empty.
    (len > 0).then_some((name, len, by_included))
}

/// `item { ',' item }`. A trailing `,` before the end of the list is reported (DS0127).
fn list(p: &mut Parser<'_>, item: impl Fn(&mut Parser<'_>)) {
    loop {
        item(p);
        if !p.at(COMMA) {
            break;
        }
        if matches!(p.nth(1), PIPE | SEMICOLON | EOF | R_PAREN | R_BRACK) {
            p.trailing_comma();
            p.bump();
            break;
        }
        p.bump();
    }
}

/// `[name '='] expr`
fn named_expr(p: &mut Parser<'_>) {
    let m = p.start();
    let saved = p.eq_compares;
    let named = p.at_name_eq();
    if named {
        p.name_before_eq();
        p.bump(); // =
        // After `a =`, another `=` can only be a mistyped comparison: `a = b = c` (DS0110).
        p.eq_compares = true;
    }
    // Unlike in call arguments, a `|` here belongs to the outer pipeline.
    let start = p.offset();
    let value = expr(p);
    // SQL's `a AS b` (DS0105): parsed as the name it gives.
    if !named
        && value.is_some()
        && p.at(IDENT)
        && p.nth_text(0).eq_ignore_ascii_case("as")
        && NAME_START.contains(p.nth(1))
        && !p.at_line_start()
    {
        p.sql_as(start);
        p.bump(); // as
        name(p);
    }
    p.eq_compares = saved;
    m.complete_if_nonempty(p, NAMED_EXPR);
}

/// A column name with `*` wildcards, written without spaces: `a`, `col*`, `*_id`.
fn column_pattern(p: &mut Parser<'_>) {
    const PARTS: TokenSet = TokenSet::new(&[IDENT, QUOTED_IDENT, STAR, LONG]);
    if !(p.at_set(NAME_START) || p.at(STAR)) {
        let what = format!("a column name{}", p.after_prev());
        if !p.error_expected_name(&what) {
            return;
        }
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
    let what = format!("a new column name{}", p.after_prev());
    if p.at_set(NAME_START) || p.error_expected_name(&what) {
        name(p);
    }
    if !p.eat(EQ) {
        let what = format!("`=`{}", p.after_prev());
        p.error_expected(&what);
    }
    if p.at_set(NAME_START) || p.error_expected_name("the column to rename after `=`") {
        name_ref(p);
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
    if p.at(IDENT) && ["asc", "desc"].contains(&p.nth_text(0).to_lowercase().as_str()) {
        p.check_sort_direction_case();
        p.bump();
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
        if p.at(IDENT) {
            p.check_join_kind();
            p.bump();
        } else {
            let d = p
                .expected("a join kind after `kind =`")
                .note(format!("the join kinds are {}", word_list(JOIN_KINDS)));
            p.report(d);
        }
        m.complete(p, JOIN_KIND);
    }
    let right_start = p.offset();
    right_table(p, "join");
    let right = (p.prev_end() > right_start).then(|| (right_start, p.prev_end()));
    let on = if p.eat_kw("on") {
        true
    } else {
        p.missing_on(right);
        // Parse the conditions after a wrong word (`using x`) or a missing `on` (`t x`).
        let wrong_word =
            p.at(IDENT) && matches!(p.nth_text(0).to_lowercase().as_str(), "using" | "by" | "on");
        if wrong_word {
            p.bump();
        }
        wrong_word || (p.at_set(EXPR_START) && !p.at_line_start())
    };
    if on {
        p.eq_compares = true;
        list(p, |p| {
            expr(p);
        });
        p.eq_compares = false;
    }
}

/// The table on the right of `join` or `union`: a name, a path string, or a parenthesized
/// pipeline. Any expression parses here; the binder rejects the ones that are not tables.
/// `from` is not written here (D-036): it is reported, and the table after it parsed.
fn right_table(p: &mut Parser<'_>, op: &str) {
    let from = p.at(IDENT)
        && p.nth_text(0).eq_ignore_ascii_case("from")
        && EXPR_START.contains(p.nth(1))
        && !matches!(p.nth(1), MINUS | NOT_KW);
    if from {
        p.report_from_before_right_table(op);
        let m = p.start();
        p.bump(); // from
        m.complete(p, ERROR);
    }
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
    if !p.at_name_eq() {
        p.error_expected("a property such as `title = \"...\"`");
        return;
    }
    let m = p.start();
    p.name_before_eq();
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
    /// `in` and its forms, spelled with this many tokens (SQL's `NOT IN` has two).
    In(usize),
    Between,
    /// SQL's `LIKE` or `NOT LIKE` (DS0105), spelled with this many tokens.
    Like(usize),
    /// SQL's `IS NULL` or `IS NOT NULL` (DS0105), spelled with this many tokens.
    IsNull(usize),
}

/// Whether the `n`th token is the word `word` in any case.
fn nth_word(p: &Parser<'_>, n: usize, word: &str) -> bool {
    matches!(p.nth(n), IDENT | NOT_KW | NULL_KW) && p.nth_text(n).eq_ignore_ascii_case(word)
}

/// How many tokens SQL's `IS [NOT] NULL` takes, if it starts at the current token.
fn is_null_len(p: &Parser<'_>) -> Option<usize> {
    if !nth_word(p, 0, "is") {
        return None;
    }
    if nth_word(p, 1, "null") {
        Some(2)
    } else if nth_word(p, 1, "not") && nth_word(p, 2, "null") {
        Some(3)
    } else {
        None
    }
}

/// The infix operator at the current token and its binding power.
fn infix(p: &Parser<'_>) -> Option<(u8, Infix)> {
    let op = match p.current() {
        OR_KW => (BP_OR, Infix::Binary(1)),
        AND_KW => (BP_AND, Infix::Binary(1)),
        // Habits from other languages parse like the operator meant, and are reported.
        PIPE2 if !p.at_double_pipe() => (BP_OR, Infix::Binary(1)),
        AMP2 => (BP_AND, Infix::Binary(1)),
        // `===` and `= =`: one mistyped `==` (DS0110).
        EQ2 | EQ
            if (p.current() == EQ2 || p.eq_compares) && p.nth(1) == EQ && !p.nth_line_start(1) =>
        {
            (BP_CMP, Infix::Binary(2))
        }
        EQ if p.eq_compares => (BP_CMP, Infix::Binary(1)),
        LTGT => (BP_CMP, Infix::Binary(1)),
        EQ2 | NEQ | LT | LTEQ | GT | GTEQ | EQ_TILDE | BANG_TILDE => (BP_CMP, Infix::Binary(1)),
        IN_TILDE => (BP_CMP, Infix::In(1)),
        // SQL's `not in` and `not like`, in any case.
        NOT_KW if nth_word(p, 1, "in") => (BP_CMP, Infix::In(2)),
        NOT_KW if nth_word(p, 1, "like") => (BP_CMP, Infix::Like(2)),
        NEGATED_OP => match p.nth_text(0) {
            "!in" | "!in~" => (BP_CMP, Infix::In(1)),
            "!between" => (BP_CMP, Infix::Between),
            _ => (BP_CMP, Infix::Binary(1)),
        },
        IDENT => match p.nth_text(0) {
            "in" => (BP_CMP, Infix::In(1)),
            // SQL keywords in capitals (DS0105) parse like the operator meant.
            "AND" => (BP_AND, Infix::Binary(1)),
            "OR" => (BP_OR, Infix::Binary(1)),
            "IN" => (BP_CMP, Infix::In(1)),
            "NOT" if nth_word(p, 1, "in") => (BP_CMP, Infix::In(2)),
            "NOT" if nth_word(p, 1, "like") => (BP_CMP, Infix::Like(2)),
            word if word.eq_ignore_ascii_case("like") || word.eq_ignore_ascii_case("ilike") => {
                (BP_CMP, Infix::Like(1))
            }
            _ if is_null_len(p).is_some() => (BP_CMP, Infix::IsNull(is_null_len(p)?)),
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

/// One comparison of a chain (`a < b < c` has two), for the "cannot be chained" error.
struct Link {
    /// Token index of the operator.
    op: usize,
    /// How many tokens the operator spans (`matches regex` and `===` have two).
    len: usize,
    /// Byte range of the right operand.
    rhs: (u32, u32),
    /// Whether the help may quote this link: a plain operator with its right operand present.
    simple: bool,
}

/// How many comparisons a chain may have for the help to spell out every pair.
const MAX_CHAIN_HELP: usize = 6;

/// How deeply expressions may nest. The parser is recursive; this keeps it well inside the stack
/// (including wasm32's 1 MiB default and 2 MiB test threads) on input such as 100,000 `(`.
///
/// The budget counts tree depth, not only recursion: each operator folded into a chain
/// (`a + b + c`, `a.b.c`) wraps the tree built so far in one more node, so it costs one level
/// too. Otherwise a long flat chain builds a tree deep enough to overflow recursive walkers and
/// rowan's own drop.
const MAX_NESTING: u32 = 256;

fn expr_bp(p: &mut Parser<'_>, min_bp: u8) -> Option<CompletedMarker> {
    if p.nesting >= MAX_NESTING {
        return too_deep(p, false);
    }
    // Folds in this frame (`expr_bp_inner`, `postfix`) add to `nesting`; all of it is given back
    // here, once the frame's subtree is finished.
    let saved = p.nesting;
    p.nesting += 1;
    let result = expr_bp_inner(p, min_bp);
    p.nesting = saved;
    result
}

/// Spends one level of the depth budget on an operator about to wrap the tree built so far.
/// Past the limit, reports DS0100 and skips the rest of the statement; returns `false`.
/// Checked one level early, so that the operand which follows (`a + …`, `a[…]`) still fits and
/// the error points at the operator.
fn fold_level(p: &mut Parser<'_>) -> bool {
    if p.nesting + 1 >= MAX_NESTING {
        too_deep(p, true);
        return false;
    }
    p.nesting += 1;
    true
}

/// Reports DS0100 and skips the rest of the statement without recursing. Stopping at a `|` would
/// let the innermost parenthesis take the outer pipeline's operators, and nest too deeply again.
/// `at_operator` is set when the limit is hit by an operator in a chain rather than by a bracket
/// or prefix, which needs a different explanation.
fn too_deep(p: &mut Parser<'_>, at_operator: bool) -> Option<CompletedMarker> {
    let d = if at_operator {
        Diagnostic::error(codes::DS0100, "expression is too long")
            .primary(
                p.current_span(),
                format!("more than {MAX_NESTING} levels deep at this operator"),
            )
            .note("each operator in a chain like `a + b + c` counts as one level of nesting")
    } else {
        Diagnostic::error(codes::DS0100, "expression is nested too deeply").primary(
            p.current_span(),
            format!("more than {MAX_NESTING} levels deep here"),
        )
    }
    .help("split it into smaller parts with `let`");
    p.report(d);
    p.too_deep = true;
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
    let mut lhs = prefix_or_primary(p, min_bp)?;
    // The comparisons folded one after another into `lhs`: one error for the whole chain.
    let mut chain: Vec<Link> = Vec::new();
    while let Some((bp, op)) = infix(p) {
        if bp < min_bp || !fold_level(p) {
            break;
        }
        let op_start = p.pos;
        if bp != BP_CMP || p.last_error_at == Some(op_start) {
            // `a < b and c`: the comparison is over, and the next one compares the `and`. After
            // a missing operand (`a == b and == c`), the error already reported explains it.
            report_chained_comparison(p, start, &chain);
            chain.clear();
        }
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
            Infix::In(len) => {
                for _ in 0..len {
                    p.bump();
                }
                if p.at(TILDE) && matches!(p.prev_text(), Some("in" | "!in")) {
                    // `in ~ (...)`: `in~` with a space in it (DS0129).
                    p.spaced_in_tilde(op_start);
                    p.bump();
                }
                let rhs_start = p.offset();
                if p.at(L_PAREN) {
                    arg_list(p, true);
                } else if p.at(L_BRACK) {
                    // `in [1, 2]` (DS0128): parse the list as if it were in parentheses.
                    let open = p.pos;
                    arg_list(p, true);
                    p.in_brackets(open);
                } else if p.at(QUOTED_IDENT) && p.nth_text(0).starts_with("['") {
                    // `in ['a']` is read as the quoted name `['a']` (DS0128).
                    p.in_quoted_brackets();
                    let m = p.start();
                    p.bump();
                    m.complete(p, ARG_LIST);
                } else {
                    let what = format!("`(` and a list of values{}", p.after_prev());
                    p.error_expected(&what);
                }
                (IN_EXPR, (rhs_start, p.prev_end().max(rhs_start)))
            }
            Infix::Like(len) => {
                for _ in 0..len {
                    p.bump();
                }
                let rhs_start = p.offset();
                let rhs = expr_bp(p, bp + 1);
                let rhs_range = (rhs_start, p.prev_end().max(rhs_start));
                p.sql_like(op_start, len, rhs.map(|_| rhs_range));
                (BIN_EXPR, rhs_range)
            }
            Infix::IsNull(len) => {
                for _ in 0..len {
                    p.bump();
                }
                p.sql_is_null(start, op_start, len);
                (BIN_EXPR, (p.prev_end(), p.prev_end()))
            }
            Infix::Between => {
                p.bump();
                let rhs_start = p.offset();
                between_bounds(p);
                (BETWEEN_EXPR, (rhs_start, p.prev_end().max(rhs_start)))
            }
        };
        lhs = m.complete(p, kind);
        let op_len = match op {
            Infix::Binary(len) | Infix::In(len) | Infix::Like(len) | Infix::IsNull(len) => len,
            Infix::Between => 1,
        };
        match op {
            Infix::Binary(_) => p.foreign_binary_operator(op_start, op_len, start, rhs),
            Infix::In(_) => p.sql_in(op_start, op_len),
            _ => {}
        }
        if bp == BP_CMP {
            // The help rewrites the chain only for plain operators with both operands present.
            let simple = matches!(op, Infix::Binary(_)) && rhs.0 < rhs.1;
            chain.push(Link {
                op: op_start,
                len: op_len,
                rhs,
                simple,
            });
        }
    }
    report_chained_comparison(p, start, &chain);
    Some(lhs)
}

/// `a < b < c`: comparisons do not chain (LANGUAGE.md §4). One error for the whole chain, which
/// starts at byte `start`; nothing for a single comparison.
fn report_chained_comparison(p: &mut Parser<'_>, start: u32, chain: &[Link]) {
    let [first, second, ..] = chain else {
        return;
    };
    let op_text = |link: &Link| {
        let op = p.tokens[link.op];
        (op.start, p.tokens[link.op + link.len - 1].end())
    };
    let (op_start, op_end) = op_text(second);
    let mut d = Diagnostic::error(codes::DS0101, "comparison operators cannot be chained")
        .primary(p.span(op_start, op_end), "a second comparison")
        .secondary(
            p.span(start, first.rhs.1),
            "the result of this comparison is not a value to compare again",
        );
    let mut pairs = vec![p.source(start, first.rhs.1).to_string()];
    for pair in chain.windows(2) {
        let (op_start, op_end) = op_text(&pair[1]);
        pairs.push(format!(
            "{} {} {}",
            p.source(pair[0].rhs.0, pair[0].rhs.1),
            p.source(op_start, op_end),
            p.source(pair[1].rhs.0, pair[1].rhs.1)
        ));
    }
    let rewrite = pairs.join(" and ");
    let quotable = chain.len() <= MAX_CHAIN_HELP
        && chain.iter().all(|link| link.simple)
        && !rewrite.contains(['\n', '\r']);
    if quotable {
        d = d.help(format!("compare each pair separately: `{rewrite}`"));
    } else {
        d = d.help("compare each pair separately and combine them with `and`");
    }
    if chain.len() > 2 {
        d = d.note(format!("this chain has {} comparisons", chain.len()));
    }
    // The operator token parsed fine, so it has no error yet: report without suppression.
    p.errors.push(d);
}

/// A prefix operator and its operand, or a primary expression with its postfix operators.
/// `min_bp` is the binding power of the operator this is the operand of.
fn prefix_or_primary(p: &mut Parser<'_>, min_bp: u8) -> Option<CompletedMarker> {
    let start = p.offset();
    // SQL's `NOT` in capitals (DS0105) parses like `not`.
    let sql_not = p.at(IDENT) && p.nth_text(0) == "NOT" && EXPR_START.contains(p.nth(1));
    if sql_not {
        p.sql_not();
    }
    let current = if sql_not { NOT_KW } else { p.current() };
    match current {
        NOT_KW if p.nth(1) == L_PAREN => {
            // `not(x)` reads like a call and binds like one: `not(x) == y` is `(not x) == y`.
            let m = p.start();
            p.bump();
            paren_expr(p);
            let not = m.complete(p, PREFIX_EXPR);
            Some(postfix(p, not, start))
        }
        NOT_KW => {
            let m = p.start();
            let not = p.pos;
            p.bump();
            let operand_start = p.offset();
            let operand = expr_bp(p, BP_NOT);
            if min_bp > BP_NOT && operand.is_some() {
                // `1 + not x`: `not` binds more loosely than the operator before it (DS0114).
                p.not_needs_parens(not, (operand_start, p.prev_end()));
            }
            Some(m.complete(p, PREFIX_EXPR))
        }
        MINUS => {
            let m = p.start();
            p.bump();
            expr_bp(p, BP_NEG);
            Some(m.complete(p, PREFIX_EXPR))
        }
        BANG => {
            // C's `!x`: parsed as `not(x)` and reported (DS0111).
            let m = p.start();
            let bang = p.pos;
            p.bump();
            let start = p.offset();
            let operand = expr_bp(p, BP_NEG);
            let paren = operand.is_some_and(|o| o.kind(p) == PAREN_EXPR);
            let range = operand.map(|_| (start, p.prev_end()));
            p.bang_not(bang, range, paren);
            Some(m.complete(p, PREFIX_EXPR))
        }
        _ => {
            let primary = primary(p)?;
            Some(postfix(p, primary, start))
        }
    }
}

fn primary(p: &mut Parser<'_>) -> Option<CompletedMarker> {
    let kind = p.current();
    if LITERALS.contains(kind) {
        let m = p.start();
        let first = p.pos;
        p.bump();
        match kind {
            // `1.2.3`: a number with more than one decimal point, read as `1.2` then `.3`.
            LONG | REAL if p.at(REAL) && p.nth_joined(0) && p.nth_text(0).starts_with('.') => {
                while p.at(REAL) && p.nth_joined(0) && p.nth_text(0).starts_with('.') {
                    p.bump();
                }
                p.dotted_number(first);
            }
            // SQL's `'it''s'`: two strings side by side.
            STRING if p.at(STRING) && p.nth_joined(0) => {
                while p.at(STRING) && p.nth_joined(0) {
                    p.bump();
                }
                p.doubled_quotes(first);
            }
            _ => {}
        }
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
        IDENT
            if p.nth(1) == L_PAREN
                && p.nth(2) == STAR
                && p.nth(3) == R_PAREN
                && p.nth_text(0).eq_ignore_ascii_case("count") =>
        {
            // SQL's `count(*)` (DS0117): the `*` is wrapped in an `ERROR` node, so the call
            // reads as `count()`.
            p.count_star();
            let callee = name_ref(p);
            let m = callee.precede(p);
            let args = p.start();
            p.bump(); // (
            let star = p.start();
            p.bump(); // *
            star.complete(p, ERROR);
            p.bump(); // )
            args.complete(p, ARG_LIST);
            Some(m.complete(p, CALL_EXPR))
        }
        IDENT if p.nth(1) == L_PAREN => {
            let predicates = PREDICATE_FUNCTIONS.contains(&p.nth_text(0));
            let callee = name_ref(p);
            let m = callee.precede(p);
            // Inside other calls `name = value` is a named argument, even in a `where`.
            let saved = std::mem::replace(&mut p.eq_compares, predicates);
            arg_list(p, false);
            p.eq_compares = saved;
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

/// Member access `a.b` and indexing `a[0]`. `start` is the byte offset where `lhs` starts.
fn postfix(p: &mut Parser<'_>, mut lhs: CompletedMarker, start: u32) -> CompletedMarker {
    // The `.` and the name of the last step, if it was a member access: `a.b(` is a method call.
    let mut field: Option<(usize, usize)> = None;
    loop {
        let method_call = p.at(L_PAREN) && field.is_some();
        if (method_call || matches!(p.current(), DOT | L_BRACK)) && !fold_level(p) {
            return lhs;
        }
        let step = p.current();
        match step {
            L_PAREN if method_call => {
                // `a.b.c(1)`: DataScript has no methods (DS0113). The node is a call with no
                // name, so the arguments are still checked.
                let (dot, name) = field.take().expect("checked above");
                let m = lhs.precede(p);
                let open = p.pos;
                arg_list(p, false);
                p.method_call(start, dot, name, open);
                lhs = m.complete(p, CALL_EXPR);
            }
            DOT => {
                let m = lhs.precede(p);
                let dot = p.pos;
                p.bump();
                if p.at_set(NAME_START) || p.error_expected_name("a name after `.`") {
                    field = Some((dot, p.pos));
                    name_ref(p);
                }
                lhs = m.complete(p, FIELD_EXPR);
                continue;
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
        field = None;
    }
}

fn paren_expr(p: &mut Parser<'_>) -> CompletedMarker {
    let m = p.start();
    let open = p.pos;
    p.bump(); // (
    p.depth += 1;
    let saved = std::mem::replace(&mut p.in_arg_list, false);
    let tabular = pipeline_or_expr(p, false);
    p.in_arg_list = saved;
    expect_closing(p, open);
    p.depth -= 1;
    let paren = m.complete(p, PAREN_EXPR);
    p.paren_pipeline = tabular.then_some(open);
    paren
}

/// `'(' [arg { ',' arg }] ')'`, where an argument is `[name '='] value`. The list after `in`
/// (`in_list`) needs at least one value.
fn arg_list(p: &mut Parser<'_>, in_list: bool) {
    let m = p.start();
    let open = p.pos;
    let close = if p.at(L_BRACK) { R_BRACK } else { R_PAREN };
    p.bump(); // ( or the `[` of `in [...]`
    p.depth += 1;
    if in_list && p.at(close) {
        let d = p
            .expected("a value in the list")
            .primary(p.current_span(), "the list after `in` is empty")
            .help("list at least one value: `x in (1, 2)`");
        p.report(d);
    }
    let saved = std::mem::replace(&mut p.in_arg_list, true);
    if !p.at(close) {
        // `iff(x = 1, ...)` is a comparison: those functions take no named arguments.
        let named_args = !p.eq_compares;
        list(p, |p| {
            if named_args && p.at_name_eq() {
                let m = p.start();
                p.name_before_eq();
                p.bump(); // =
                pipeline_or_expr(p, false);
                m.complete(p, NAMED_EXPR);
            } else {
                pipeline_or_expr(p, false);
            }
        });
    }
    p.in_arg_list = saved;
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
    if !closing_ahead(p, close) {
        p.unclosed_bracket(open);
        if p.at(EOF) || p.at(SEMICOLON) || p.at(LET_KW) {
            return;
        }
        // Skip the rest of the statement quietly: the one error above explains it. Stop where
        // the next statement clearly starts, so it is not lost.
        if !p.at_statement_after_unclosed() {
            p.last_error_at = Some(p.pos);
            p.recover(|p| p.at(SEMICOLON) || p.at(LET_KW) || p.at_statement_after_unclosed());
        }
        // No DS0106 for the missing `;` there: the unclosed bracket is the root cause.
        p.last_error_at = Some(p.pos);
        return;
    }
    let close_text = if close == R_BRACK { "]" } else { ")" };
    let open_text = if close == R_BRACK { "[" } else { "(" };
    let d = p.expected(&format!("`{close_text}`")).secondary(
        p.span(open_token.start, open_token.end()),
        format!("to close this `{open_text}`"),
    );
    p.report(d);
    p.recover(|p| p.at(close) || p.at(SEMICOLON) || p.at(LET_KW));
    p.eat(close);
}

/// Whether a `close` bracket that is not inside another bracket comes before the end of the
/// statement.
fn closing_ahead(p: &Parser<'_>, close: crate::SyntaxKind) -> bool {
    let mut stack = Vec::new();
    for (i, t) in p.tokens.iter().enumerate().skip(p.pos) {
        if i > p.pos && p.statement_after_unclosed_at(i) {
            return false;
        }
        match t.kind {
            SEMICOLON | LET_KW => return false,
            L_PAREN => stack.push(R_PAREN),
            L_BRACK => stack.push(R_BRACK),
            kind if stack.is_empty() && kind == close => return true,
            kind if stack.last() == Some(&kind) => {
                stack.pop();
            }
            _ => {}
        }
    }
    false
}

/// "`a`, `b` and `c`"
pub(super) fn word_list(words: &[&str]) -> String {
    let quoted: Vec<String> = words.iter().map(|w| format!("`{w}`")).collect();
    match quoted.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
        Some((last, _)) => last.clone(),
        None => String::new(),
    }
}
