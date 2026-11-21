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
    /// see [`Parser::statement_after_unclosed_at`].
    fn at_statement_after_unclosed(&self) -> bool {
        self.statement_after_unclosed_at(self.pos)
    }

    /// Whether the token at index `i` clearly starts a new statement even inside brackets, so an
    /// unclosed bracket does not swallow it: it can start an expression, starts a line with no
    /// indentation, and follows a complete expression (`f(a⏎U | take 1`). A continuation such
    /// as `f(a,⏎b)` or `(a⏎- b)` does not count.
    fn statement_after_unclosed_at(&self, i: usize) -> bool {
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
        if p.at(LET_KW) || p.at_new_statement() {
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
    let start = p.pos;
    if p.at(PIPE) {
        let d = Diagnostic::error(codes::DS0101, "expected a table before `|`")
            .primary(p.current_span(), "this pipe has nothing to read from")
            .help("start the query with a file path such as `'data.csv'`, or a table name");
        p.report(d);
    } else if let Some((from, end)) = sql_select_at(p) {
        p.sql_select(p.pos, from, end);
        p.recover(|p| p.at(SEMICOLON) || p.at(LET_KW));
        m.complete(p, PIPELINE);
        return;
    } else {
        expr(p);
    }
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

/// An expression, wrapped in a `PIPELINE` if a `|` follows. Used where the parser cannot tell
/// a table from a scalar: the right side of `let`, inside parentheses, and call arguments.
fn pipeline_or_expr(p: &mut Parser<'_>) -> bool {
    let start = p.pos;
    let Some(source) = expr(p) else {
        return false;
    };
    if p.at(PIPE) {
        let m = source.precede(p);
        pipeline_tail(p, start);
        m.complete(p, PIPELINE);
    }
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
        if p.at_new_statement() || (p.depth > 0 && p.at_statement_after_unclosed()) {
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
            p.at(PIPE) || p.at_pipeline_end() || p.at_operator_on_new_line() || p.at_new_statement()
        });
    }
    p.pipeline_start = saved_start;
    p.last_op = ops.last().and_then(|op| op.kind);
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

/// The operator a machine-applicable fix for an unknown operator name turns it into: the name,
/// how many tokens the fix replaces, and whether it includes `by`.
fn certain_operator(p: &Parser<'_>, d: &Diagnostic) -> Option<(&'static str, usize, bool)> {
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
    let saved = p.eq_compares;
    if p.at_set(NAME_START) && p.nth(1) == EQ {
        name(p);
        p.bump(); // =
        // After `a =`, another `=` can only be a mistyped comparison: `a = b = c` (DS0110).
        p.eq_compares = true;
    }
    // Unlike in call arguments, a `|` here belongs to the outer pipeline.
    expr(p);
    p.eq_compares = saved;
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
    right_table(p);
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
            Infix::In => {
                p.bump();
                let rhs_start = p.offset();
                if p.at(L_PAREN) {
                    arg_list(p, true);
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
        let op_len = match op {
            Infix::Binary(len) => len,
            _ => 1,
        };
        p.foreign_binary_operator(op_start, op_len, start, rhs);
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
    match p.current() {
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
                if p.at_set(NAME_START) {
                    field = Some((dot, p.pos));
                    name_ref(p);
                } else {
                    p.error_expected_name("a name after `.`");
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
    pipeline_or_expr(p);
    p.in_arg_list = saved;
    expect_closing(p, open);
    p.depth -= 1;
    m.complete(p, PAREN_EXPR)
}

/// `'(' [arg { ',' arg }] ')'`, where an argument is `[name '='] value`. The list after `in`
/// (`in_list`) needs at least one value.
fn arg_list(p: &mut Parser<'_>, in_list: bool) {
    let m = p.start();
    let open = p.pos;
    p.bump(); // (
    p.depth += 1;
    if in_list && p.at(R_PAREN) {
        let d = p
            .expected("a value in the list")
            .primary(p.current_span(), "the list after `in` is empty")
            .help("list at least one value: `x in (1, 2)`");
        p.report(d);
    }
    let saved = std::mem::replace(&mut p.in_arg_list, true);
    if !p.at(R_PAREN) {
        // `iff(x = 1, ...)` is a comparison: those functions take no named arguments.
        let named_args = !p.eq_compares;
        list(p, |p| {
            if named_args && p.at_set(NAME_START) && p.nth(1) == EQ {
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
