//! Targeted syntax diagnostics (DS0102–DS0149): habits from SQL, KQL and C-like languages, and
//! the mistakes people make most, each with a concrete fix where we can give one.
//!
//! The grammar decides *where* a mistake is; the functions here decide what to say about it.

use datascript_diagnostics::{
    Applicability, Diagnostic, Suggestion, TextEdit, codes, did_you_mean,
};

use super::grammar::{LATER_OPERATORS, OPERATORS, word_list};
use super::{Parser, TokenSet};
use crate::SyntaxKind::{self, *};

/// SQL words that people write as an operator, matched ignoring case. Written in any other case
/// than lowercase they are DS0105.
const SQL_WORDS: &[&str] = &[
    "distinct", "from", "group", "having", "join", "limit", "order", "select", "top", "union",
    "where",
];

/// Operator names from other languages, written in lowercase (DS0104), and what to say about
/// them: the DataScript operator, the help text, and whether replacing the name is certain.
const OPERATOR_ALIASES: &[(&str, Option<&str>, &str, bool)] = &[
    ("agg", None, GROUP_HELP, false),
    ("aggregate", None, GROUP_HELP, false),
    (
        "dedup",
        Some("distinct"),
        "to remove duplicate rows, use `distinct`",
        false,
    ),
    (
        "dedupe",
        Some("distinct"),
        "to remove duplicate rows, use `distinct`",
        false,
    ),
    (
        "drop",
        Some("project-away"),
        "to remove columns, use `project-away`",
        true,
    ),
    (
        "filter",
        Some("where"),
        "to keep the rows that match a condition, use `where`",
        true,
    ),
    ("from", None, FROM_HELP, false),
    ("group", None, GROUP_HELP, false),
    ("group_by", None, GROUP_HELP, false),
    ("groupby", None, GROUP_HELP, false),
    (
        "having",
        Some("where"),
        "to filter after `summarize`, use `where`",
        false,
    ),
    (
        "head",
        Some("take"),
        "to keep the first rows, use `take`",
        true,
    ),
    (
        "mutate",
        Some("extend"),
        "to add computed columns, use `extend`",
        true,
    ),
    (
        "order_by",
        Some("sort by"),
        "to sort rows, use `sort by`",
        true,
    ),
    (
        "orderby",
        Some("sort by"),
        "to sort rows, use `sort by`",
        true,
    ),
    (
        "rename",
        Some("project-rename"),
        "to rename columns, use `project-rename new = old`",
        false,
    ),
    (
        "select",
        Some("project"),
        "to choose or compute columns, use `project`",
        true,
    ),
    (
        "sort_by",
        Some("sort by"),
        "to sort rows, use `sort by`",
        true,
    ),
    (
        "sortby",
        Some("sort by"),
        "to sort rows, use `sort by`",
        true,
    ),
    (
        "uniq",
        Some("distinct"),
        "to remove duplicate rows, use `distinct`",
        false,
    ),
    (
        "unique",
        Some("distinct"),
        "to remove duplicate rows, use `distinct`",
        false,
    ),
];

const GROUP_HELP: &str = "to group rows, use `summarize ... by`: `summarize count() by customerId`";
const FROM_HELP: &str = "a query starts with its table, without `from`: `'orders.csv' | take 10`";

/// The join kinds (LANGUAGE.md §3.2), in the order the docs list them.
pub(super) const JOIN_KINDS: &[&str] = &[
    "inner",
    "leftouter",
    "rightouter",
    "fullouter",
    "leftsemi",
    "leftanti",
    "rightsemi",
    "rightanti",
];

/// Join kind spellings from SQL and KQL, and the DataScript kind.
const JOIN_KIND_ALIASES: &[(&str, &str)] = &[
    ("anti", "leftanti"),
    ("full", "fullouter"),
    ("left", "leftouter"),
    ("leftantisemi", "leftanti"),
    ("outer", "fullouter"),
    ("right", "rightouter"),
    ("rightantisemi", "rightanti"),
    ("semi", "leftsemi"),
];

/// Functions whose arguments are conditions and never named, so `iff(x = 1, ...)` is a mistyped
/// comparison, not a named argument.
pub(super) const PREDICATE_FUNCTIONS: &[&str] = &["case", "countif", "iff", "iif", "sumif"];

/// Rewrites quoted in help text are only shown when they stay this short.
const MAX_SNIPPET: usize = 48;

/// Tokens that may end a word, so a replacement word next to them needs a space.
const WORDLIKE: TokenSet = TokenSet::new(&[
    IDENT,
    QUOTED_IDENT,
    LONG,
    REAL,
    STRING,
    TIMESPAN,
    DATETIME,
    TRUE_KW,
    FALSE_KW,
    NULL_KW,
    AND_KW,
    OR_KW,
    NOT_KW,
    LET_KW,
    DOLLAR_LEFT,
    DOLLAR_RIGHT,
    NEGATED_OP,
    IN_TILDE,
]);

impl Parser<'_> {
    /// Source text for a help message, or `None` if it spans lines or is too long to quote.
    fn snippet(&self, start: u32, end: u32) -> Option<&str> {
        let text = self.source(start, end);
        (!text.is_empty() && text.chars().count() <= MAX_SNIPPET && !text.contains(['\n', '\r']))
            .then_some(text)
    }

    /// An edit replacing the token at `i` with `word`, adding a space on a side where the token
    /// touches another word, so `a&&b` becomes `a and b`, not `aandb`.
    fn word_edit(&self, i: usize, word: &str) -> TextEdit {
        let mut replacement = String::new();
        let word_start = word.starts_with(|c: char| c.is_alphanumeric() || c == '_');
        let word_end = word.ends_with(|c: char| c.is_alphanumeric() || c == '_');
        if word_start && i > 0 && self.joined[i] && WORDLIKE.contains(self.tokens[i - 1].kind) {
            replacement.push(' ');
        }
        replacement.push_str(word);
        if word_end
            && self.joined.get(i + 1).copied().unwrap_or(false)
            && WORDLIKE.contains(self.tokens[i + 1].kind)
        {
            replacement.push(' ');
        }
        TextEdit::new(self.token_span(i), replacement)
    }

    // --- Pipes and operators ---

    /// DS0102: an operator name where a `|` was forgotten. The current token is the name.
    pub(super) fn missing_pipe(&mut self, name: &str) {
        let at = self.offset();
        let d = Diagnostic::error(codes::DS0102, format!("missing `|` before `{name}`"))
            .primary(
                self.current_span(),
                format!("`{name}` starts a new operator"),
            )
            .help_with(
                format!("insert `|` to pipe into `{name}`: `| {name}`"),
                Suggestion::replace(self.span(at, at), "| "),
            );
        self.report(d);
    }

    /// DS0103: the `|` at token index `pipe` has no operator after it. The current token is
    /// what follows it.
    pub(super) fn pipe_without_operator(&mut self, pipe: usize) {
        if self.last_error_at == Some(pipe) {
            // `|` with no table before it already has its error.
            return;
        }
        let pipe_span = self.token_span(pipe);
        if self.at(PIPE) {
            // `| |`: remove the second pipe and the space before it.
            let second = self.current_span();
            let d = Diagnostic::error(codes::DS0103, "two `|` in a row")
                .primary(pipe_span, "no operator after this `|`")
                .help_with(
                    "remove the extra `|`",
                    Suggestion::replace(self.span(pipe_span.end, second.end), ""),
                );
            self.report(d);
            return;
        }
        let gap_start = if pipe > 0 {
            self.tokens[pipe - 1].end()
        } else {
            pipe_span.start
        };
        let d = Diagnostic::error(codes::DS0103, "`|` with no operator after it")
            .primary(pipe_span, "expected an operator after this `|`")
            .help_with(
                "remove the `|`, or add an operator after it, such as `take 10`",
                Suggestion::new(
                    vec![TextEdit::new(self.span(gap_start, pipe_span.end), "")],
                    Applicability::MaybeIncorrect,
                ),
            );
        // Report at the pipe, so the token after it can still get its own error.
        self.errors.push(d);
    }

    /// DS0103 for `||` used as two pipes: `t || take 5`. The current token is `||`.
    pub(super) fn double_pipe_token(&mut self) {
        let d = Diagnostic::error(codes::DS0103, "two `|` in a row")
            .primary(self.current_span(), "one `|` is enough")
            .help_with(
                "remove the extra `|`",
                Suggestion::replace(self.current_span(), "|"),
            );
        self.report(d);
    }

    /// DS0104 for a hyphenated operator name written with spaces: `project - away`. The current
    /// token is the first word; the name spans `len` tokens.
    pub(super) fn spaced_operator_name(&mut self, name: &str, len: usize) {
        let span = self.span(self.offset(), self.tokens[self.pos + len - 1].end());
        let written = self.source(span.start, span.end).to_string();
        let mut d = Diagnostic::error(
            codes::DS0104,
            format!("operator name `{written}` has spaces in it"),
        )
        .primary(span, "");
        if LATER_OPERATORS.contains(&name) {
            d = d.note(format!("`{name}` is planned but not available yet"));
        } else {
            d = d.help_with(
                format!("write `{name}`, with no spaces"),
                Suggestion::replace(span, name),
            );
        }
        self.report(d);
    }

    /// DS0104 / DS0105 for a word after `|` that is not an operator. `name` and `len` come from
    /// `op_name`. Returns the diagnostic; the caller reports it and recovers.
    pub(super) fn unknown_operator_diagnostic(&self, name: &str, len: usize) -> Diagnostic {
        let mut span = self.span(self.offset(), self.tokens[self.pos + len - 1].end());
        if LATER_OPERATORS.contains(&name) {
            return Diagnostic::error(
                codes::DS0104,
                format!("operator `{name}` is not available yet"),
            )
            .primary(span, "planned for a later version");
        }
        let lower = name.to_lowercase();

        // SQL keywords in SQL's uppercase (DS0105). `GROUP BY` and `ORDER BY` are one keyword.
        if lower != name && SQL_WORDS.contains(&lower.as_str()) {
            let mut written = name.to_string();
            if matches!(lower.as_str(), "group" | "order")
                && self.nth(len) == IDENT
                && self.nth_text(len).eq_ignore_ascii_case("by")
            {
                span.end = self.tokens[self.pos + len].end();
                written = self.source(span.start, span.end).to_string();
            }
            let d = Diagnostic::error(
                codes::DS0105,
                format!("SQL keyword `{written}` is not an operator"),
            )
            .primary(span, "");
            let has_by = written.contains(char::is_whitespace);
            return match lower.as_str() {
                "select" => d.help_with(
                    "choose or compute columns with `project`: `project a, b`",
                    Suggestion::replace(span, "project"),
                ),
                "group" => d.help(GROUP_HELP),
                "order" if has_by => d.help_with(
                    "sort rows with `sort by`: `sort by amount desc`",
                    Suggestion::replace(span, "sort by"),
                ),
                "having" => d.help_with(
                    "filter after `summarize` with `where`",
                    Suggestion::new(
                        vec![TextEdit::new(span, "where")],
                        Applicability::MaybeIncorrect,
                    ),
                ),
                "from" => d.help(FROM_HELP),
                _ => d.help_with(
                    format!("operator names are lowercase: write `{lower}`"),
                    Suggestion::replace(span, lower.clone()),
                ),
            };
        }

        let d = Diagnostic::error(codes::DS0104, format!("unknown operator `{name}`"));

        // Operator names from other languages.
        if let Some(&(_, replacement, help, certain)) =
            OPERATOR_ALIASES.iter().find(|(alias, ..)| *alias == name)
        {
            let d = d.primary(span, "not a DataScript operator");
            return match replacement {
                Some(op) => {
                    let applicability = if certain {
                        Applicability::MachineApplicable
                    } else {
                        Applicability::MaybeIncorrect
                    };
                    d.help_with(
                        help,
                        Suggestion::new(vec![TextEdit::new(span, op)], applicability),
                    )
                }
                None => d.help(help),
            };
        }

        // The right name in the wrong case: `Take`.
        if OPERATORS.contains(&lower.as_str()) {
            return d
                .primary(span, "operator names are case-sensitive")
                .help_with(
                    format!("write `{lower}`"),
                    Suggestion::replace(span, lower.clone()),
                );
        }

        let d = d.primary(span, "");
        let close = did_you_mean(name, OPERATORS.iter().copied());
        // A misspelled name from another language (`filtr`): say what it is called here, but
        // leave the fix to the user.
        let alias = did_you_mean(&lower, OPERATOR_ALIASES.iter().map(|(a, ..)| *a));
        let alias = alias
            .single()
            .and_then(|a| OPERATOR_ALIASES.iter().find(|(name, ..)| *name == a));
        match (close.single(), close.names()) {
            (Some(op), _) => d.help_with(
                format!("an operator with a similar name exists: `{op}`"),
                Suggestion::replace(span, op),
            ),
            (None, []) => match alias {
                Some(&(_, Some(op), help, _)) => d.help_with(
                    help,
                    Suggestion::new(vec![TextEdit::new(span, op)], Applicability::MaybeIncorrect),
                ),
                Some(&(_, None, help, _)) => d.help(help),
                None => d.note(format!("the operators are {}", word_list(OPERATORS))),
            },
            (None, names) => {
                let names: Vec<&str> = names.iter().map(String::as_str).collect();
                d.help(format!(
                    "operators with similar names exist: {}",
                    word_list(&names)
                ))
            }
        }
    }

    /// DS0105 for a whole SQL query: `SELECT a, b FROM t WHERE x > 1`. The current token is
    /// `SELECT`; `from` is the token index of `FROM` and `end` the index just past the statement.
    pub(super) fn sql_select(&mut self, from: usize, end: usize) {
        let select = self.pos;
        let mut d = Diagnostic::error(codes::DS0105, "`SELECT ... FROM` is SQL, not DataScript")
            .primary(self.token_span(select), "this is SQL")
            .secondary(self.token_span(from), "in DataScript the table comes first");
        match self.sql_rewrite(select, from, end) {
            Some(rewrite) => {
                let span = self.span(self.tokens[select].start, self.tokens[end - 1].end());
                d = d.help_with(
                    format!("start with the table and pipe it into operators: `{rewrite}`"),
                    Suggestion::new(
                        vec![TextEdit::new(span, rewrite)],
                        Applicability::MaybeIncorrect,
                    ),
                );
            }
            None => {
                d = d.help(
                    "start with the table and pipe it into operators: \
                     `'orders.csv' | where amount > 10 | project customerId, amount`",
                );
            }
        }
        self.report(d);
    }

    /// `SELECT cols FROM table [WHERE cond]` as a pipeline, when it is that simple.
    fn sql_rewrite(&self, select: usize, from: usize, end: usize) -> Option<String> {
        let cols = &self.tokens[select + 1..from];
        let star = matches!(cols, [t] if t.kind == STAR);
        let names = !cols.is_empty()
            && cols
                .iter()
                .all(|t| matches!(t.kind, IDENT | QUOTED_IDENT | COMMA));
        if !(star || names) || from + 1 >= end {
            return None;
        }
        let table = self.tokens[from + 1];
        if !matches!(table.kind, IDENT | QUOTED_IDENT | STRING) {
            return None;
        }
        let mut rewrite = table.text(self.text).to_string();
        let rest = &self.tokens[from + 2..end];
        if let Some((kw, cond)) = rest.split_first() {
            let is_where = kw.kind == IDENT && kw.text(self.text).eq_ignore_ascii_case("where");
            let plain = !cond.is_empty()
                && cond.iter().all(|t| {
                    !matches!(t.kind, EQ | LTGT | AMP2 | PIPE2 | BANG | ERROR)
                        && !(t.kind == IDENT
                            && SQL_WORDS.contains(&t.text(self.text).to_lowercase().as_str()))
                });
            if !(is_where && plain) {
                return None;
            }
            let cond_text = self.source(cond[0].start, cond[cond.len() - 1].end());
            rewrite = format!("{rewrite} | where {cond_text}");
        }
        if names {
            let cols_text = self.source(cols[0].start, cols[cols.len() - 1].end());
            rewrite = format!("{rewrite} | project {cols_text}");
        }
        (rewrite.chars().count() <= 2 * MAX_SNIPPET && !rewrite.contains(['\n', '\r']))
            .then_some(rewrite)
    }

    /// DS0120: `render` at token range `render.0..render.1` is followed by `next`. `moved` is
    /// the text from the end of the operator before `render` to the end of `render`, which the
    /// fix moves to `end`, the end of the pipeline. `None` when the fix would not help.
    pub(super) fn render_not_last(
        &mut self,
        render: (u32, u32),
        next: (u32, u32),
        moved: Option<(u32, u32)>,
        end: u32,
    ) {
        let text = self.snippet(render.0, render.1).unwrap_or("render ...");
        let mut d = Diagnostic::error(codes::DS0120, "`render` must be the last operator")
            .primary(self.span(render.0, render.1), "")
            .secondary(
                self.span(next.0, next.1),
                "this operator comes after `render`",
            );
        match moved {
            Some((start, stop)) => {
                let moved_text = self.source(start, stop).to_string();
                d = d.help_with(
                    format!("move `{text}` to the end of the query"),
                    Suggestion::new(
                        vec![
                            TextEdit::new(self.span(start, stop), ""),
                            TextEdit::new(self.span(end, end), moved_text),
                        ],
                        Applicability::MachineApplicable,
                    ),
                );
            }
            None => d = d.help("a query draws one chart: keep only the last `render`"),
        }
        self.errors.push(d);
    }

    /// DS0121: `sort` or `top` with no `by`. `at` is the token index where `by` belongs, and
    /// `rest` the source range of what follows it (the sort keys), if anything.
    pub(super) fn missing_by(
        &mut self,
        op: &str,
        op_start: u32,
        at: usize,
        rest: Option<(u32, u32)>,
    ) {
        let d = Diagnostic::error(codes::DS0121, format!("missing `by` after `{op}`"));
        let d = match rest {
            Some((start, end)) => {
                let d = d.primary(self.token_span(at), "expected `by` before this");
                let insert = self.span(start, start);
                let help = match self.snippet(op_start, start).zip(self.snippet(start, end)) {
                    Some((head, keys)) => format!("insert `by`: `{head}by {keys}`"),
                    None => "insert `by`".to_string(),
                };
                d.help_with(help, Suggestion::replace(insert, "by "))
            }
            None => {
                let help = if op == "top" {
                    "say which column to rank by: `top 10 by amount desc`"
                } else {
                    "say which columns to sort by: `sort by amount desc`"
                };
                d.primary(self.current_span(), "expected `by` here")
                    .help(help)
            }
        };
        if self.tokens.get(at).is_some_and(|t| t.kind == ERROR) {
            return;
        }
        self.last_error_at = Some(at);
        self.errors.push(d);
    }

    /// DS0105 for `ASC` / `DESC` in SQL's uppercase. The current token is the word.
    pub(super) fn check_sort_direction_case(&mut self) {
        let word = self.nth_text(0);
        let lower = word.to_lowercase();
        if word == lower {
            return;
        }
        let d = Diagnostic::error(
            codes::DS0105,
            format!("SQL keyword `{word}` is not a sort direction"),
        )
        .primary(self.current_span(), "")
        .help_with(
            format!("keywords are lowercase: write `{lower}`"),
            Suggestion::replace(self.current_span(), lower.clone()),
        );
        self.report(d);
    }

    /// DS0122: `join` without `on`. `right` is the source range of the right table.
    pub(super) fn missing_on(&mut self, right: Option<(u32, u32)>) {
        // `using x` (SQL) or `by x`: the right idea with the wrong word.
        if self.at(IDENT)
            && matches!(
                self.nth_text(0).to_lowercase().as_str(),
                "using" | "by" | "on"
            )
        {
            let word = self.nth_text(0).to_string();
            let d = Diagnostic::error(codes::DS0122, format!("`join` uses `on`, not `{word}`"))
                .primary(self.current_span(), "")
                .help_with(
                    "write `on` before the columns to match",
                    Suggestion::replace(self.current_span(), "on"),
                );
            self.report(d);
            return;
        }
        let table = right
            .and_then(|(s, e)| self.snippet(s, e))
            .unwrap_or("customers");
        let mut d = Diagnostic::error(codes::DS0122, "`join` needs `on` and the columns to match");
        if self.at_set(super::grammar::EXPR_START) && !self.at_line_start() {
            // `join customers customerId`: only `on` is missing.
            let at = self.offset();
            let help = match right.and_then(|(s, e)| self.snippet(s, e)) {
                Some(table) => format!("insert `on`: `join {table} on {}`", self.nth_text(0)),
                None => "insert `on`".to_string(),
            };
            d = d
                .primary(self.current_span(), "expected `on` before this")
                .help_with(help, Suggestion::replace(self.span(at, at), "on "));
        } else {
            let label_span = match right {
                Some((s, e)) => self.span(s, e),
                None => self.current_span(),
            };
            d = d
                .primary(label_span, "expected `on` after this table")
                .help(format!(
                    "say which columns to match: `join {table} on customerId`"
                ));
        }
        self.report(d);
    }

    /// DS0123 / DS0124 for the word after `kind =`, if it is not a join kind. The current token
    /// is the word.
    pub(super) fn check_join_kind(&mut self) {
        let word = self.nth_text(0).to_string();
        if JOIN_KINDS.contains(&word.as_str()) {
            return;
        }
        let span = self.current_span();
        if word.eq_ignore_ascii_case("innerunique") {
            let d = Diagnostic::error(codes::DS0124, "`innerunique` is not supported")
                .primary(span, "this is KQL's default join kind")
                .note(
                    "unlike KQL, DataScript never drops duplicate keys from the left side; \
                     the default join kind is `inner`",
                )
                .help_with(
                    "use `kind = inner`; to keep one left row per key, as KQL does, remove the \
                     duplicates first, for example with `distinct`",
                    Suggestion::new(
                        vec![TextEdit::new(span, "inner")],
                        Applicability::MaybeIncorrect,
                    ),
                );
            self.report(d);
            return;
        }
        let d = Diagnostic::error(codes::DS0123, format!("unknown join kind `{word}`"))
            .primary(span, "");
        let lower = word.to_lowercase();
        let alias = JOIN_KIND_ALIASES
            .iter()
            .find(|(a, _)| *a == lower)
            .map(|&(_, kind)| kind);
        let close = did_you_mean(&word, JOIN_KINDS.iter().copied());
        let d = match alias.or(close.single()) {
            Some(kind) => d.help_with(
                format!("write `kind = {kind}`"),
                Suggestion::replace(span, kind),
            ),
            None => d.note(format!("the join kinds are {}", word_list(JOIN_KINDS))),
        };
        self.report(d);
    }

    // --- Expressions ---

    /// DS0107 / DS0110 / DS0111 for a binary operator from another language. `op` is its token
    /// index, `lhs_start` where the left operand starts and `rhs` the right operand's range.
    pub(super) fn foreign_binary_operator(&mut self, op: usize, lhs_start: u32, rhs: (u32, u32)) {
        let token = self.tokens[op];
        let (code, message, label, word, verb): (_, &str, &str, &str, &str) = match token.kind {
            EQ => (
                codes::DS0110,
                "`=` is assignment, not comparison",
                "assignment is not allowed here",
                "==",
                "use `==` to compare values",
            ),
            LTGT => (
                codes::DS0107,
                "`<>` is not an operator",
                "this is SQL's \"not equal\"",
                "!=",
                "use `!=` to test that values differ",
            ),
            AMP2 => (
                codes::DS0111,
                "`&&` is not an operator",
                "",
                "and",
                "use `and` to require both conditions",
            ),
            PIPE2 => (
                codes::DS0111,
                "`||` is not an operator",
                "",
                "or",
                "use `or` to accept either condition",
            ),
            _ => return,
        };
        let edit = self.word_edit(op, word);
        // Quote the rewrite only if the operands hold no other habit, which it would repeat.
        // Tokens are sorted by offset: look only at the operands' range, so that a file full of
        // `&&` stays linear.
        let first = self.tokens.partition_point(|t| t.start < lhs_start);
        let last = self.tokens.partition_point(|t| t.end() <= rhs.1);
        let clean = self.tokens[first..last.max(first)]
            .iter()
            .filter(|t| t.start != token.start)
            .all(|t| !matches!(t.kind, EQ | LTGT | AMP2 | PIPE2 | BANG));
        let help = match self
            .snippet(lhs_start, token.start)
            .zip(self.snippet(token.end(), rhs.1))
        {
            Some((left, right)) if clean && rhs.0 < rhs.1 => {
                // Rebuild the text with the fix applied in place.
                format!(
                    "{verb}: `{left}{}{right}`",
                    with_gaps(&edit.replacement, left, right)
                )
            }
            _ => verb.to_string(),
        };
        let d = Diagnostic::error(code, message)
            .primary(self.token_span(op), label)
            .help_with(
                help,
                Suggestion::new(vec![edit], Applicability::MachineApplicable),
            );
        self.errors.push(d);
    }

    /// DS0111 for `!x`. `bang` is the token index of `!`; the operand spans `operand` and is a
    /// parenthesized expression if `paren`.
    pub(super) fn bang_not(&mut self, bang: usize, operand: Option<(u32, u32)>, paren: bool) {
        let mut d = Diagnostic::error(codes::DS0111, "`!` is not an operator")
            .primary(self.token_span(bang), "");
        let Some((start, end)) = operand else {
            d = d.help("negate a condition with `not(...)`: `not(x > 1)`");
            self.report(d);
            return;
        };
        let operand_text = self.source(start, end).to_string();
        let edits = if paren {
            vec![self.word_edit(bang, "not")]
        } else {
            vec![
                self.word_edit(bang, "not("),
                TextEdit::new(self.span(end, end), ")"),
            ]
        };
        let rewrite = if paren {
            format!("not{operand_text}")
        } else {
            format!("not({operand_text})")
        };
        let help = match self.snippet(start, end) {
            Some(_) => format!("negate it with `not(...)`: `{rewrite}`"),
            None => "negate it with `not(...)`".to_string(),
        };
        d = d.help_with(
            help,
            Suggestion::new(edits, Applicability::MachineApplicable),
        );
        // `!has_error` reads like the negated operator `!has` followed by `_error`.
        let operand_token = self.tokens[bang + 1];
        if operand_token.kind == IDENT && !paren {
            let name = operand_token.text(self.text);
            if let Some(op) = ["has", "contains", "startswith", "endswith", "between", "in"]
                .iter()
                .find(|op| name.len() > op.len() && name.starts_with(*op))
            {
                d = d.note(format!(
                    "`!{name}` is `!` followed by the name `{name}`; the negated operator \
                     `!{op}` goes between two values: `a !{op} b`"
                ));
            }
        }
        self.errors.push(d);
    }

    /// DS0114: prefix `not` (token index `not`) as the operand of an operator that binds more
    /// tightly, such as `1 + not x` or `a == not b`. The operand of `not` spans `operand`.
    pub(super) fn not_needs_parens(&mut self, not: usize, operand: (u32, u32)) {
        if self.errors_at(not) {
            return;
        }
        let before = not
            .checked_sub(1)
            .map_or("", |i| self.tokens[i].text(self.text));
        let not_token = self.tokens[not];
        let edits = vec![
            TextEdit::new(self.span(not_token.start, operand.0), "not("),
            TextEdit::new(self.span(operand.1, operand.1), ")"),
        ];
        let help = match self.snippet(operand.0, operand.1) {
            Some(text) => format!("write it as a call to say what it negates: `not({text})`"),
            None => "write it as a call to say what it negates: `not(...)`".to_string(),
        };
        let d = Diagnostic::error(codes::DS0114, "`not` needs parentheses here")
            .primary(
                self.token_span(not),
                format!("`not` binds more loosely than `{before}`"),
            )
            .help_with(help, Suggestion::new(edits, Applicability::MaybeIncorrect))
            .note("without parentheses, `not` negates everything up to the next `and` or `or`");
        self.errors.push(d);
    }

    /// DS0113: a method-style call `base.name(args)`. The member access starts at byte `start`;
    /// `dot` and `name` are token indexes; the argument list opened at `open` has been parsed.
    pub(super) fn method_call(&mut self, start: u32, dot: usize, name: usize, open: usize) {
        if self.errors_at(open) {
            return;
        }
        let name_text = self.tokens[name].text(self.text).to_string();
        let base = self.source(start, self.tokens[dot].start).to_string();
        let end = self.prev_end();
        let close_start = if self.tokens[self.pos - 1].kind == R_PAREN {
            self.tokens[self.pos - 1].start
        } else {
            end
        };
        let args = self
            .source(self.tokens[open].end(), close_start)
            .trim()
            .to_string();
        let mut d = Diagnostic::error(codes::DS0113, "DataScript has no method calls").primary(
            self.span(self.tokens[dot].start, self.tokens[open].end()),
            format!("`{name_text}` is called like a method of `{base}`"),
        );
        let string_operator = [
            "contains",
            "contains_cs",
            "endswith",
            "endswith_cs",
            "has",
            "has_cs",
            "startswith",
            "startswith_cs",
        ]
        .contains(&name_text.as_str());
        let (rewrite, help) = if string_operator && !args.is_empty() && !args.contains(',') {
            (
                format!("{base} {name_text} {args}"),
                format!("`{name_text}` is an operator that goes between two values"),
            )
        } else if args.is_empty() {
            (
                format!("{name_text}({base})"),
                "call it as a function".to_string(),
            )
        } else {
            (
                format!("{name_text}({base}, {args})"),
                format!("call it as a function, with `{base}` as the first argument"),
            )
        };
        let help = if rewrite.chars().count() <= MAX_SNIPPET && !rewrite.contains(['\n', '\r']) {
            format!("{help}: `{rewrite}`")
        } else {
            help
        };
        d = d.help_with(
            help,
            Suggestion::new(
                vec![TextEdit::new(self.span(start, end), rewrite)],
                Applicability::MaybeIncorrect,
            ),
        );
        self.errors.push(d);
    }

    /// DS0115: `where a, b`. `commas` are the token indexes of the commas between conditions,
    /// which start at byte `start`; the conditions have been parsed.
    pub(super) fn where_with_commas(&mut self, commas: &[usize], start: u32) {
        if self.errors_at(commas[0]) {
            return;
        }
        let end = self.prev_end();
        let edits: Vec<TextEdit> = commas.iter().map(|&i| self.word_edit(i, "and")).collect();
        // `a or b, c` would become `a or b and c`, which groups as `a or (b and c)`.
        let first = self.tokens.partition_point(|t| t.start < start);
        let last = self.tokens.partition_point(|t| t.end() <= end);
        let has_or = self.tokens[first..last]
            .iter()
            .any(|t| matches!(t.kind, OR_KW | PIPE2));
        let applicability = if has_or {
            Applicability::MaybeIncorrect
        } else {
            Applicability::MachineApplicable
        };
        let help = match self.snippet(start, end) {
            Some(_) => {
                let mut rewrite = String::new();
                let mut at = start;
                for edit in &edits {
                    rewrite.push_str(self.source(at, edit.span.start));
                    rewrite.push_str(&edit.replacement);
                    at = edit.span.end;
                }
                rewrite.push_str(self.source(at, end));
                format!("combine the conditions with `and`: `{rewrite}`")
            }
            None => "combine the conditions with `and`".to_string(),
        };
        let d = Diagnostic::error(codes::DS0115, "`where` takes one condition")
            .primary(
                self.token_span(commas[0]),
                "a comma does not join conditions",
            )
            .help_with(help, Suggestion::new(edits, applicability));
        self.errors.push(d);
    }

    /// DS0116: `distinct *, a`. `star` is the token index of `*`; the current token is the comma.
    pub(super) fn distinct_star_with_columns(&mut self, star: usize) {
        let star_start = self.tokens[star].start;
        let next_start = self
            .tokens
            .get(self.pos + 1)
            .map_or(self.prev_end(), |t| t.start);
        let d = Diagnostic::error(codes::DS0116, "`distinct *` cannot list other columns")
            .primary(self.token_span(star), "`*` already means every column")
            .help_with(
                "remove `*` to keep the distinct values of the listed columns, or remove the \
                 columns to keep distinct rows",
                Suggestion::new(
                    vec![TextEdit::new(self.span(star_start, next_start), "")],
                    Applicability::MaybeIncorrect,
                ),
            );
        self.report(d);
    }

    /// Whether the token at index `i` already has an error from the parser.
    fn errors_at(&self, i: usize) -> bool {
        let token = self.tokens[i];
        self.tokens[i].kind == ERROR
            || self.errors.iter().any(|d| {
                d.primary
                    .as_ref()
                    .is_some_and(|l| l.span.start == token.start && l.span.end >= token.end())
            })
    }

    // --- Brackets and statements ---

    /// DS0112: the bracket at token index `open` is never closed. The current token is where the
    /// parser gave up looking (the end of the input, `;` or `let`).
    pub(super) fn unclosed_bracket(&mut self, open: usize) {
        if self.too_deep {
            return;
        }
        let open_token = self.tokens[open];
        let (open_text, close_text) = if open_token.kind == L_BRACK {
            ("[", "]")
        } else {
            ("(", ")")
        };
        // Several brackets left open at the same place: one error, a label for each.
        if let Some((pos, index)) = self.last_unclosed
            && pos == self.pos
        {
            let label = format!("this `{open_text}` is never closed either");
            self.errors[index] = self.errors[index]
                .clone()
                .secondary(self.token_span(open), label);
            return;
        }
        self.last_unclosed = Some((self.pos, self.errors.len()));
        let end_label = if self.at(EOF) {
            format!("the input ends before a `{close_text}`")
        } else {
            format!("expected `{close_text}` before this")
        };
        let d = Diagnostic::error(codes::DS0112, format!("unclosed `{open_text}`"))
            .primary(
                self.token_span(open),
                format!("this `{open_text}` is never closed"),
            )
            .secondary(self.current_span(), end_label)
            .help(format!("add `{close_text}` where the bracketed part ends"));
        self.errors.push(d);
    }

    /// DS0112 for a `)` or `]` with nothing to close. The current token is the bracket.
    pub(super) fn stray_close_bracket(&mut self) {
        let (open_text, close_text) = if self.at(R_BRACK) {
            ("[", "]")
        } else {
            ("(", ")")
        };
        let d = Diagnostic::error(codes::DS0112, format!("unmatched `{close_text}`"))
            .primary(
                self.current_span(),
                format!("there is no `{open_text}` for this to close"),
            )
            .help_with(
                format!("remove it, or add the missing `{open_text}`"),
                Suggestion::new(
                    vec![TextEdit::new(self.current_span(), "")],
                    Applicability::MaybeIncorrect,
                ),
            );
        self.report(d);
    }

    /// DS0106: a new statement starts without `;` after the previous one.
    pub(super) fn missing_semicolon(&mut self) {
        let end = self.prev_end();
        let d = Diagnostic::error(codes::DS0106, "missing `;` between statements")
            .primary(self.span(end, end), "expected `;` here")
            .secondary(self.current_span(), "a new statement starts here")
            .help_with(
                "end the statement with `;`",
                Suggestion::replace(self.span(end, end), ";"),
            );
        self.report(d);
    }
}

/// `word` as it reads between `left` and `right` in a help message: with a space on each side
/// unless the operand text already ends or starts with one.
fn with_gaps(word: &str, left: &str, right: &str) -> String {
    let word = word.trim();
    let before = if left.ends_with(char::is_whitespace) {
        ""
    } else {
        " "
    };
    let after = if right.starts_with(char::is_whitespace) {
        ""
    } else {
        " "
    };
    format!("{before}{word}{after}")
}

/// Whether `kind` closes a bracket.
pub(super) fn is_close_bracket(kind: SyntaxKind) -> bool {
    matches!(kind, R_PAREN | R_BRACK)
}
