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

/// SQL words that people write as an operator, matched ignoring case. Written in capitals they
/// are DS0105.
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

/// SQL aggregate functions that have the same name in DataScript, matched ignoring case.
const SQL_AGGREGATES: &[&str] = &["avg", "count", "max", "min", "sum"];

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

/// Tokens that another fix rewrites into a word (`&&` to `and`, `!` to `not(`), so a replacement
/// word next to them needs a space too: `a&&!b` becomes `a and not(b)`, not `a andnot(b)`.
const FIXED_TO_WORD: TokenSet = TokenSet::new(&[AMP2, PIPE2, BANG]);

/// Operators that take a list, where an item on the next line without `,` is likely a forgotten
/// comma rather than a new statement (DS0106).
const LIST_OPERATORS: &[SyntaxKind] = &[
    PROJECT_OP,
    PROJECT_AWAY_OP,
    PROJECT_RENAME_OP,
    EXTEND_OP,
    SUMMARIZE_OP,
    SORT_OP,
    TOP_OP,
    DISTINCT_OP,
    JOIN_OP,
];

impl Parser<'_> {
    /// Source text for a help message, or `None` if it spans lines or is too long to quote.
    fn snippet(&self, start: u32, end: u32) -> Option<&str> {
        let text = self.source(start, end);
        (!text.is_empty() && text.chars().count() <= MAX_SNIPPET && !text.contains(['\n', '\r']))
            .then_some(text)
    }

    /// An edit replacing the token at `i` with `word`, adding a space on a side where the token
    /// touches another word, so `a&&b` becomes `a and b`, not `aandb`. A token after it that
    /// another fix turns into a word counts as a word, so fixes applied together still read
    /// right. (Only the fix on the left pads, so two fixes never add two spaces.)
    fn word_edit(&self, i: usize, word: &str) -> TextEdit {
        self.words_edit(i, i, word)
    }

    /// [`Self::word_edit`] for the tokens `first..=last`.
    fn words_edit(&self, first: usize, last: usize, word: &str) -> TextEdit {
        let mut replacement = String::new();
        let word_start = word.starts_with(|c: char| c.is_alphanumeric() || c == '_');
        let word_end = word.ends_with(|c: char| c.is_alphanumeric() || c == '_');
        if word_start
            && first > 0
            && self.joined[first]
            && WORDLIKE.contains(self.tokens[first - 1].kind)
        {
            replacement.push(' ');
        }
        replacement.push_str(word);
        if word_end
            && self.joined.get(last + 1).copied().unwrap_or(false)
            && (WORDLIKE.contains(self.tokens[last + 1].kind)
                || FIXED_TO_WORD.contains(self.tokens[last + 1].kind))
        {
            replacement.push(' ');
        }
        let span = self.span(self.tokens[first].start, self.tokens[last].end());
        TextEdit::new(span, replacement)
    }

    /// Whether the tokens from index `i` to the end of the operator are a plain column list:
    /// names and commas, with no SQL word such as `DISTINCT` or `AS`.
    fn plain_column_list(&self, i: usize) -> bool {
        let rest: Vec<_> = self.tokens[i.min(self.tokens.len())..]
            .iter()
            .take_while(|t| !matches!(t.kind, PIPE | SEMICOLON | LET_KW | EOF))
            .collect();
        let mut expect_name = true;
        for t in &rest {
            let ok = if expect_name {
                t.kind == QUOTED_IDENT
                    || (t.kind == IDENT && {
                        let lower = t.text(self.text).to_lowercase();
                        !SQL_WORDS.contains(&lower.as_str()) && lower != "as"
                    })
            } else {
                t.kind == COMMA
            };
            if !ok {
                return false;
            }
            expect_name = !expect_name;
        }
        !rest.is_empty() && !expect_name
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
        .primary(span, "an operator name is one word");
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

        // SQL keywords in SQL's capitals (DS0105). `GROUP BY` and `ORDER BY` are one keyword.
        // Other mixed case (`Where`) is just the wrong case, below.
        if lower != name && name == name.to_uppercase() && SQL_WORDS.contains(&lower.as_str()) {
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
            .primary(span, "this is SQL");
            let has_by = written.contains(char::is_whitespace);
            return match lower.as_str() {
                // `| SELECT *` keeps every column, which the rows have already.
                "select"
                    if self.pos >= 2
                        && self.tokens[self.pos - 1].kind == PIPE
                        && self.nth(len) == STAR
                        && matches!(self.nth(len + 1), PIPE | SEMICOLON | LET_KW | EOF) =>
                {
                    let lead = self.tokens[self.pos - 2].end();
                    let star_end = self.tokens[self.pos + len].end();
                    d.help_with(
                        "every column is kept already: remove `| SELECT *`",
                        Suggestion::replace(self.span(lead, star_end), ""),
                    )
                }
                // `SELECT DISTINCT a` or `SELECT a AS b` still fail as `project`.
                "select" => d.help_with(
                    "choose or compute columns with `project`: `project a, b`",
                    Suggestion::new(
                        vec![TextEdit::new(span, "project")],
                        if self.plain_column_list(self.pos + len) {
                            Applicability::MachineApplicable
                        } else {
                            Applicability::MaybeIncorrect
                        },
                    ),
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
            OPERATOR_ALIASES.iter().find(|(alias, ..)| *alias == lower)
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

        let d = d.primary(span, "not an operator");
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
    /// `start` is the token index where the pipeline starts: `SELECT` itself at the start of a
    /// query, or the source before the pipe in `t | SELECT a FROM t`. The rewrite replaces it all.
    pub(super) fn sql_select(&mut self, start: usize, from: usize, end: usize) {
        let select = self.pos;
        // After operators (`t | where x | SELECT ...`) the rewrite would drop them.
        let pipes = self.tokens[start..select]
            .iter()
            .filter(|t| t.kind == PIPE)
            .count();
        let mut d = Diagnostic::error(codes::DS0105, "`SELECT ... FROM` is SQL, not DataScript")
            .primary(self.token_span(select), "this is SQL")
            .secondary(self.token_span(from), "in DataScript the table comes first");
        match self.sql_rewrite(select, from, end) {
            Some(rewrite) if pipes <= 1 => {
                let span = self.span(self.tokens[start].start, self.tokens[end - 1].end());
                d = d.help_with(
                    format!("start with the table and pipe it into operators: `{rewrite}`"),
                    Suggestion::new(
                        vec![TextEdit::new(span, rewrite)],
                        Applicability::MaybeIncorrect,
                    ),
                );
            }
            _ => {
                d = d.help(
                    "start with the table and pipe it into operators: \
                     `'orders.csv' | where amount > 10 | project customerId, amount`",
                );
            }
        }
        self.report(d);
    }

    /// `SELECT items FROM table [WHERE cond] [GROUP BY keys] [ORDER BY keys] [LIMIT n]` as a
    /// pipeline, when every part has a plain DataScript equivalent.
    fn sql_rewrite(&self, select: usize, from: usize, end: usize) -> Option<String> {
        let table = *self.tokens.get(from + 1)?;
        if from + 1 >= end || !matches!(table.kind, IDENT | QUOTED_IDENT | STRING) {
            return None;
        }
        // The clauses after the table, split at their keywords (outside brackets).
        let mut clauses: Vec<(&str, usize, usize)> = Vec::new();
        let mut i = from + 2;
        let mut depth = 0u32;
        let mut current: Option<(&str, usize)> = None;
        while i < end {
            let t = self.tokens[i];
            match t.kind {
                L_PAREN | L_BRACK => depth += 1,
                R_PAREN | R_BRACK => depth = depth.saturating_sub(1),
                _ => {}
            }
            let word = (t.kind == IDENT && depth == 0).then(|| t.text(self.text).to_lowercase());
            let by = |j: usize| {
                self.tokens.get(j).is_some_and(|t| {
                    t.kind == IDENT && t.text(self.text).eq_ignore_ascii_case("by")
                })
            };
            let clause = match word.as_deref() {
                Some("where") => Some(("where", 1)),
                Some("group") if by(i + 1) => Some(("group", 2)),
                Some("order") if by(i + 1) => Some(("order", 2)),
                Some("limit") => Some(("limit", 1)),
                Some("having" | "join" | "union" | "offset") => return None,
                _ => None,
            };
            match clause {
                Some((name, len)) => {
                    if let Some((prev, start)) = current {
                        clauses.push((prev, start, i));
                    }
                    if clauses.iter().any(|(c, ..)| *c == name) {
                        return None;
                    }
                    current = Some((name, i + len));
                    i += len;
                }
                None if current.is_none() => return None,
                None => i += 1,
            }
        }
        if let Some((name, start)) = current {
            clauses.push((name, start, end));
        }
        let clause = |name: &str| {
            clauses
                .iter()
                .find(|(c, ..)| *c == name)
                .map(|&(_, start, end)| (start, end))
        };

        // The select list: `*`, or items split at commas outside brackets.
        let cols = select + 1..from;
        let star = cols.len() == 1 && self.tokens[select + 1].kind == STAR;
        let mut items: Vec<(String, bool)> = Vec::new();
        if !star {
            let mut start = cols.start;
            let mut depth = 0u32;
            for i in cols.clone() {
                match self.tokens[i].kind {
                    L_PAREN => depth += 1,
                    R_PAREN => depth = depth.saturating_sub(1),
                    COMMA if depth == 0 => {
                        items.push(self.sql_item(start, i)?);
                        start = i + 1;
                    }
                    _ => {}
                }
            }
            items.push(self.sql_item(start, cols.end)?);
        }

        let mut rewrite = table.text(self.text).to_string();
        if let Some((start, end)) = clause("where") {
            rewrite = format!("{rewrite} | where {}", self.sql_expr(start, end)?);
        }
        let sort = match clause("order") {
            Some((start, end)) => {
                let keys = self.sql_list(start, end, true)?;
                Some(format!(" | sort by {}", keys.join(", ")))
            }
            None => None,
        };
        let take = match clause("limit") {
            Some((start, end)) if end == start + 1 && self.tokens[start].kind == LONG => {
                Some(format!(" | take {}", self.tokens[start].text(self.text)))
            }
            Some(_) => return None,
            None => None,
        };
        let aggregates: Vec<&str> = items
            .iter()
            .filter(|(_, agg)| *agg)
            .map(|(text, _)| text.as_str())
            .collect();
        match clause("group") {
            Some((start, end)) => {
                let keys = self.sql_list(start, end, false)?;
                // Every item that is not an aggregate must be a key.
                if star || items.iter().any(|(text, agg)| !agg && !keys.contains(text)) {
                    return None;
                }
                rewrite = if aggregates.is_empty() {
                    format!("{rewrite} | summarize by {}", keys.join(", "))
                } else {
                    format!(
                        "{rewrite} | summarize {} by {}",
                        aggregates.join(", "),
                        keys.join(", ")
                    )
                };
                rewrite.push_str(sort.as_deref().unwrap_or(""));
                rewrite.push_str(take.as_deref().unwrap_or(""));
            }
            None if !aggregates.is_empty() => {
                if aggregates.len() != items.len() || sort.is_some() {
                    return None;
                }
                rewrite = format!("{rewrite} | summarize {}", aggregates.join(", "));
                rewrite.push_str(take.as_deref().unwrap_or(""));
            }
            None => {
                // Sort before `project`, which may drop the sort keys.
                rewrite.push_str(sort.as_deref().unwrap_or(""));
                rewrite.push_str(take.as_deref().unwrap_or(""));
                if !star {
                    let names: Vec<&str> = items.iter().map(|(text, _)| text.as_str()).collect();
                    rewrite = format!("{rewrite} | project {}", names.join(", "));
                }
            }
        }
        (rewrite.chars().count() <= 2 * MAX_SNIPPET && !rewrite.contains(['\n', '\r']))
            .then_some(rewrite)
    }

    /// One item of a SQL select list, tokens `start..end`, as DataScript: `count(*)` becomes
    /// `count()` and `x AS y` becomes `y = x`. The flag says whether it is an aggregate.
    fn sql_item(&self, start: usize, end: usize) -> Option<(String, bool)> {
        if start >= end {
            return None;
        }
        let (value_end, alias) = match &self.tokens[start..end] {
            [.., kw, name]
                if end - start >= 3
                    && kw.kind == IDENT
                    && kw.text(self.text).eq_ignore_ascii_case("as")
                    && matches!(name.kind, IDENT | QUOTED_IDENT) =>
            {
                (end - 2, Some(name.text(self.text)))
            }
            _ => (end, None),
        };
        let value = self.sql_expr(start, value_end)?;
        let aggregate = self.tokens[start..value_end].windows(2).any(|pair| {
            pair[0].kind == IDENT
                && pair[1].kind == L_PAREN
                && SQL_AGGREGATES.contains(&pair[0].text(self.text).to_lowercase().as_str())
        });
        let text = match alias {
            Some(alias) => format!("{alias} = {value}"),
            None => value,
        };
        Some((text, aggregate))
    }

    /// Comma-separated SQL expressions, tokens `start..end`, as DataScript. With `directions`,
    /// each may end in `ASC` or `DESC`, written in lowercase.
    fn sql_list(&self, start: usize, end: usize, directions: bool) -> Option<Vec<String>> {
        let mut out = Vec::new();
        let mut item_start = start;
        let mut depth = 0u32;
        for i in start..=end {
            let at_end = i == end;
            if !at_end {
                match self.tokens[i].kind {
                    L_PAREN => depth += 1,
                    R_PAREN => depth = depth.saturating_sub(1),
                    _ => {}
                }
            }
            if at_end || (self.tokens[i].kind == COMMA && depth == 0) {
                let mut item_end = i;
                let mut direction = "";
                if directions && item_end > item_start {
                    let last = self.tokens[item_end - 1];
                    let word = last.text(self.text).to_lowercase();
                    if last.kind == IDENT && (word == "asc" || word == "desc") {
                        direction = if word == "asc" { " asc" } else { " desc" };
                        item_end -= 1;
                    }
                }
                out.push(format!(
                    "{}{direction}",
                    self.sql_expr(item_start, item_end)?
                ));
                item_start = i + 1;
            }
        }
        Some(out)
    }

    /// A SQL expression, tokens `start..end`, as DataScript text: `=` becomes `==`, `<>` becomes
    /// `!=`, `AND`/`OR`/`NOT` are lowercased and `count(*)` becomes `count()`. `None` if it uses
    /// SQL that has no direct equivalent (`LIKE`, `IS NULL`, a subquery, …).
    fn sql_expr(&self, start: usize, end: usize) -> Option<String> {
        if start >= end {
            return None;
        }
        let mut out = String::new();
        let mut at = self.tokens[start].start;
        let mut i = start;
        while i < end {
            let t = self.tokens[i];
            let text = t.text(self.text);
            let lower = text.to_lowercase();
            let replacement = match t.kind {
                EQ => Some("==".to_string()),
                LTGT => Some("!=".to_string()),
                AMP2 | PIPE2 | BANG | ERROR | SEMICOLON | PIPE => return None,
                IDENT if matches!(lower.as_str(), "and" | "or" | "not") => Some(lower.clone()),
                IDENT
                    if matches!(
                        lower.as_str(),
                        "like" | "ilike" | "is" | "between" | "case" | "exists" | "as"
                    ) || SQL_WORDS.contains(&lower.as_str()) =>
                {
                    return None;
                }
                IDENT
                    if lower == "count"
                        && self.tokens.get(i + 1).is_some_and(|t| t.kind == L_PAREN)
                        && self.tokens.get(i + 2).is_some_and(|t| t.kind == STAR)
                        && self.tokens.get(i + 3).is_some_and(|t| t.kind == R_PAREN)
                        && i + 3 < end =>
                {
                    out.push_str(self.source(at, t.start));
                    out.push_str("count()");
                    at = self.tokens[i + 3].end();
                    i += 4;
                    continue;
                }
                IDENT
                    if SQL_AGGREGATES.contains(&lower.as_str())
                        && self.tokens.get(i + 1).is_some_and(|t| t.kind == L_PAREN) =>
                {
                    Some(lower.clone())
                }
                STAR if i == start => return None,
                _ => None,
            };
            if let Some(replacement) = replacement {
                out.push_str(self.source(at, t.start));
                out.push_str(&replacement);
                at = t.end();
            }
            i += 1;
        }
        out.push_str(self.source(at, self.tokens[end - 1].end()));
        Some(out)
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
            .primary(
                self.span(render.0, render.1),
                "the chart is drawn at this point",
            )
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
        .primary(self.current_span(), "this is SQL")
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
                .primary(self.current_span(), "expected `on` here")
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
            .primary(span, "not a join kind");
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
    pub(super) fn foreign_binary_operator(
        &mut self,
        op: usize,
        len: usize,
        lhs_start: u32,
        rhs: (u32, u32),
    ) {
        let token = self.tokens[op];
        let last = op + len - 1;
        let written = self
            .source(token.start, self.tokens[last].end())
            .to_string();
        let triple = format!("`{written}` is not an operator");
        let sql = format!("SQL keyword `{written}` in capitals");
        let (code, message, label, word, verb): (_, &str, &str, &str, &str) = match token.kind {
            IDENT if written == "AND" => (
                codes::DS0105,
                &sql,
                "this is SQL",
                "and",
                "write `and` in lowercase",
            ),
            IDENT if written == "OR" => (
                codes::DS0105,
                &sql,
                "this is SQL",
                "or",
                "write `or` in lowercase",
            ),
            EQ2 if len == 2 => (
                codes::DS0110,
                &triple,
                "this is JavaScript's strict equality",
                "==",
                "use `==` to compare values",
            ),
            EQ if len == 2 => (
                codes::DS0110,
                &triple,
                "two `=` with a space between them",
                "==",
                "use `==` to compare values",
            ),
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
                "this is C's \"and\"",
                "and",
                "use `and` to require both conditions",
            ),
            PIPE2 => (
                codes::DS0111,
                "`||` is not an operator",
                "this is C's \"or\"",
                "or",
                "use `or` to accept either condition",
            ),
            _ => return,
        };
        let edit = self.words_edit(op, last, word);
        // Quote the rewrite only if the operands hold no other habit, which it would repeat.
        // Tokens are sorted by offset: look only at the operands' range, so that a file full of
        // `&&` stays linear.
        let op_last_start = self.tokens[last].start;
        let first = self.tokens.partition_point(|t| t.start < lhs_start);
        let past = self.tokens.partition_point(|t| t.end() <= rhs.1);
        let clean = self.tokens[first..past.max(first)]
            .iter()
            .filter(|t| t.start < token.start || t.start > op_last_start)
            .all(|t| {
                !matches!(t.kind, EQ | LTGT | AMP2 | PIPE2 | BANG)
                    && !(t.kind == IDENT && matches!(t.text(self.text), "AND" | "OR" | "NOT"))
            });
        let help = match self
            .snippet(lhs_start, token.start)
            .zip(self.snippet(edit.span.end, rhs.1))
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
            .primary(edit.span, label)
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
            .primary(self.token_span(bang), "this is C's \"not\"");
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

    // --- SQL habits in expressions and lists ---

    /// DS0105 for SQL's `NOT` in capitals. The current token is `NOT`.
    pub(super) fn sql_not(&mut self) {
        let span = self.current_span();
        let d = Diagnostic::error(codes::DS0105, "SQL keyword `NOT` in capitals")
            .primary(span, "this is SQL")
            .help_with(
                "keywords are lowercase: write `not`",
                Suggestion::new(
                    vec![self.word_edit(self.pos, "not")],
                    Applicability::MachineApplicable,
                ),
            );
        self.report(d);
    }

    /// DS0105 for SQL's `IN` or `NOT IN` (in any case) and for `not in`. `op` is the token index
    /// of the first word; the operator spans `len` tokens. Nothing for DataScript's own forms.
    pub(super) fn sql_in(&mut self, op: usize, len: usize) {
        let token = self.tokens[op];
        let last = op + len - 1;
        let written = self
            .source(token.start, self.tokens[last].end())
            .to_string();
        let replacement = match (len, written.as_str()) {
            (1, "IN") => "in",
            (2, _) => "!in",
            _ => return,
        };
        let edit = self.words_edit(op, last, replacement);
        let message = if len == 1 {
            "SQL keyword `IN` in capitals".to_string()
        } else {
            format!("`{written}` is SQL, not DataScript")
        };
        let help = if len == 1 {
            "keywords are lowercase: write `in`".to_string()
        } else {
            "write `!in` to test that a value is not in a list".to_string()
        };
        let d = Diagnostic::error(codes::DS0105, message)
            .primary(edit.span, "this is SQL")
            .help_with(
                help,
                Suggestion::new(vec![edit], Applicability::MachineApplicable),
            );
        self.errors.push(d);
    }

    /// DS0105 for SQL's `IS NULL` / `IS NOT NULL`. The operand starts at byte `lhs_start`; `op` is
    /// the token index of `is` and the test spans `len` tokens, all consumed.
    pub(super) fn sql_is_null(&mut self, lhs_start: u32, op: usize, len: usize) {
        let token = self.tokens[op];
        let end = self.tokens[op + len - 1].end();
        let written = self.source(token.start, end).to_string();
        let function = if len == 3 { "isnotnull" } else { "isnull" };
        let mut operand = self.source(lhs_start, token.start).trim_end().to_string();
        // `(a + 1) is null` becomes `isnull(a + 1)`, not `isnull((a + 1))`.
        let first = self.tokens.partition_point(|t| t.start < lhs_start);
        if self.tokens[first].kind == L_PAREN
            && self.matching_close(first) == Some(op - 1)
            && op - 1 > first
        {
            operand = self
                .source(self.tokens[first].end(), self.tokens[op - 1].start)
                .trim()
                .to_string();
        }
        let mut d = Diagnostic::error(codes::DS0105, format!("`{written}` is SQL, not DataScript"))
            .primary(self.span(token.start, end), "this is SQL's null test");
        if operand.is_empty() || operand.contains(['\n', '\r']) {
            d = d.help(format!("test for null with `{function}(...)`"));
        } else {
            let rewrite = format!("{function}({operand})");
            let help = if rewrite.chars().count() <= MAX_SNIPPET {
                format!("test for null with `{function}(...)`: `{rewrite}`")
            } else {
                format!("test for null with `{function}(...)`")
            };
            d = d.help_with(
                help,
                Suggestion::replace(self.span(lhs_start, end), rewrite),
            );
        }
        self.errors.push(d);
    }

    /// DS0105 for SQL's `LIKE` (also `NOT LIKE` and `ILIKE`). `op` is the token index of the
    /// first word, the operator spans `len` tokens and the pattern spans `rhs`.
    pub(super) fn sql_like(&mut self, op: usize, len: usize, rhs: Option<(u32, u32)>) {
        let token = self.tokens[op];
        let op_end = self.tokens[op + len - 1].end();
        let written = self.source(token.start, op_end).to_string();
        let negated = len == 2;
        let mut d = Diagnostic::error(codes::DS0105, format!("`{written}` is SQL, not DataScript"))
            .primary(
                self.span(token.start, op_end),
                "this is SQL's pattern match",
            );
        // A pattern that is one plain string: say which operator does the same thing.
        let pattern = rhs.and_then(|(start, end)| {
            let i = self.tokens.partition_point(|t| t.start < start);
            let t = self.tokens.get(i)?;
            (t.kind == STRING && t.end() == end).then_some(t.text(self.text))
        });
        let rewrite = pattern.and_then(|pattern| {
            let quote = pattern.chars().next().filter(|q| matches!(q, '\'' | '"'))?;
            let body = pattern.get(1..pattern.len() - 1)?;
            if body.contains(['_', '\\']) {
                return None;
            }
            let (op_name, value) = match (body.strip_prefix('%'), body.strip_suffix('%')) {
                (Some(rest), _) if rest.strip_suffix('%').is_some_and(|v| !v.contains('%')) => {
                    ("contains", rest.strip_suffix('%')?)
                }
                (Some(rest), _) if !rest.contains('%') => ("endswith", rest),
                (None, Some(rest)) if !rest.contains('%') => ("startswith", rest),
                (None, None) if !body.contains('%') => ("==", body),
                _ => return None,
            };
            if value.is_empty() {
                return None;
            }
            let op_name = match (op_name, negated) {
                ("==", true) => "!=".to_string(),
                (name, true) => format!("!{name}"),
                (name, false) => name.to_string(),
            };
            Some((op_name, format!("{quote}{value}{quote}")))
        });
        match (rewrite, rhs) {
            (Some((op_name, value)), Some((_, end))) => {
                let text = format!("{op_name} {value}");
                let left = self.snippet(self.pipeline_line_start(op), token.start);
                let help = match left {
                    Some(left) if text.chars().count() <= MAX_SNIPPET => {
                        format!("use `{op_name}`: `{}{text}`", left.trim_start())
                    }
                    _ => format!("use `{op_name}`"),
                };
                d = d.help_with(
                    help,
                    Suggestion::new(
                        vec![TextEdit::new(self.span(token.start, end), text)],
                        Applicability::MaybeIncorrect,
                    ),
                );
                if op_name.trim_start_matches('!') != "==" {
                    d = d.note(format!(
                        "`{0}` ignores case; `{0}_cs` matches case exactly",
                        op_name.trim_start_matches('!')
                    ));
                }
            }
            _ => {
                d = d.help(
                    "use `startswith`, `endswith` or `contains` for a fixed text, or \
                     `matches regex` for a pattern",
                );
            }
        }
        self.errors.push(d);
    }

    /// The token index of the bracket that closes the one at token index `open`.
    fn matching_close(&self, open: usize) -> Option<usize> {
        let mut depth = 0usize;
        for (i, t) in self.tokens.iter().enumerate().skip(open) {
            match t.kind {
                L_PAREN | L_BRACK => depth += 1,
                R_PAREN | R_BRACK => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                SEMICOLON | LET_KW => return None,
                _ => {}
            }
        }
        None
    }

    /// Where the operand before token `op` starts, for quoting it in help: the first token after
    /// the operator name or pipe that leads up to `op`, on the same line.
    fn pipeline_line_start(&self, op: usize) -> u32 {
        let mut i = op;
        while i > 0 {
            let prev = self.tokens[i - 1];
            if matches!(
                prev.kind,
                PIPE | SEMICOLON | L_PAREN | COMMA | AND_KW | OR_KW
            ) || self.line_start[i]
                || (prev.kind == IDENT && OPERATORS.contains(&prev.text(self.text)))
            {
                break;
            }
            i -= 1;
        }
        self.tokens[i].start
    }

    /// DS0105 for SQL's `a AS b`. The expression starts at byte `start`; the current token is
    /// `as` and the name follows it.
    pub(super) fn sql_as(&mut self, start: u32) {
        let as_token = self.tokens[self.pos];
        let name_token = self.tokens[self.pos + 1];
        let value_end = self.prev_end();
        let value = self.source(start, value_end).to_string();
        let name = name_token.text(self.text).to_string();
        let written = as_token.text(self.text).to_string();
        let rewrite = format!("{name} = {value}");
        let help = if rewrite.chars().count() <= MAX_SNIPPET && !rewrite.contains(['\n', '\r']) {
            format!("name a column with `=`, name first: `{rewrite}`")
        } else {
            "name a column with `=`, name first".to_string()
        };
        let d = Diagnostic::error(codes::DS0105, format!("`{written}` is SQL, not DataScript"))
            .primary(self.current_span(), "this names a column in SQL")
            .help_with(
                help,
                Suggestion::new(
                    vec![
                        TextEdit::new(self.span(start, start), format!("{name} = ")),
                        TextEdit::new(self.span(value_end, name_token.end()), ""),
                    ],
                    Applicability::MachineApplicable,
                ),
            );
        self.report(d);
    }

    /// DS0117 for SQL's `count(*)`. The current token is `count`, followed by `(*)`.
    pub(super) fn count_star(&mut self) {
        let star = self.token_span(self.pos + 2);
        let call = self.span(self.offset(), self.tokens[self.pos + 3].end());
        let d = Diagnostic::error(codes::DS0117, "`count(*)` is SQL, not DataScript")
            .primary(star, "`count()` counts rows without a `*`")
            .help_with("write `count()`", Suggestion::replace(call, "count()"));
        self.errors.push(d);
    }

    /// DS0117 for SQL's `project *`. The current token is `*`; `alone` says nothing follows it in
    /// the operator.
    pub(super) fn project_star(&mut self, op_start: u32, alone: bool) {
        let star = self.current_span();
        let mut d = Diagnostic::error(codes::DS0117, "`project *` is SQL's `SELECT *`")
            .primary(star, "`project` lists the columns to keep");
        // The `|` before `project`, so the fix removes the whole step.
        let op = self.tokens.partition_point(|t| t.start < op_start);
        let pipe = op
            .checked_sub(1)
            .filter(|&i| i > 0 && self.tokens[i].kind == PIPE);
        d = match (alone, pipe) {
            (true, Some(pipe)) => d.help_with(
                "every column is kept already: remove `| project *`",
                Suggestion::replace(self.span(self.tokens[pipe - 1].end(), star.end), ""),
            ),
            (true, None) => d.help("every column is kept already: remove `project *`"),
            (false, _) => {
                // `project *, x = 1` becomes `extend x = 1`.
                let rest = self.pos + 2;
                let end = self.tokens[rest.min(self.tokens.len())..]
                    .iter()
                    .take_while(|t| !matches!(t.kind, PIPE | SEMICOLON | LET_KW))
                    .last()
                    .map(|t| t.end());
                match (self.tokens.get(rest), end) {
                    (Some(first), Some(end)) if self.nth(1) == COMMA => {
                        let help = match self.snippet(first.start, end) {
                            Some(columns) => format!(
                                "to keep every column and add computed ones, use `extend`: \
                                 `extend {columns}`"
                            ),
                            None => "to keep every column and add computed ones, use `extend`"
                                .to_string(),
                        };
                        d.help_with(
                            help,
                            Suggestion::new(
                                vec![TextEdit::new(self.span(op_start, first.start), "extend ")],
                                Applicability::MaybeIncorrect,
                            ),
                        )
                    }
                    _ => d.help("to keep every column and add computed ones, use `extend`"),
                }
            }
        };
        self.report(d);
    }

    /// DS0117 for SQL's `'it''s'`. `first` is the token index of the first string; the strings
    /// after it, side by side, have been consumed.
    pub(super) fn doubled_quotes(&mut self, first: usize) {
        let strings = &self.tokens[first..self.pos];
        let start = strings[0].start;
        let end = strings[strings.len() - 1].end();
        let text = self.source(start, end).to_string();
        let mut d = Diagnostic::error(codes::DS0117, "`''` does not put a quote in a string");
        let quote = text.chars().find(|c| matches!(c, '\'' | '"'));
        let verbatim = text.starts_with('@');
        // Every string must use the same quote and start right where the one before ends.
        let same = quote.is_some_and(|q| {
            strings.iter().enumerate().all(|(i, t)| {
                let t = t.text(self.text);
                let body = if i == 0 && verbatim { &t[1..] } else { t };
                body.len() >= 2 && body.starts_with(q) && body.ends_with(q)
            })
        });
        let second = strings[1];
        d = d.primary(
            self.span(second.start - 1, second.start + 1),
            "this ends the string and starts another",
        );
        let Some(q) = quote.filter(|_| same && strings.len() >= 2) else {
            d = d.help("escape a quote inside a string with a backslash: `'it\\'s'`");
            self.errors.push(d);
            return;
        };
        let doubled = format!("{q}{q}");
        let other = if q == '\'' { '"' } else { '\'' };
        let (help, rewrite) = if verbatim {
            // No escapes in `@'...'`: use the other quote, if the text has none.
            let body = &text[2..text.len() - 1].replace(&doubled, &q.to_string());
            if body.contains(other) {
                (None, None)
            } else {
                let rewrite = format!("@{other}{body}{other}");
                (
                    Some(format!(
                        "a verbatim string has no escapes; use the other quote: `{rewrite}`"
                    )),
                    Some(rewrite),
                )
            }
        } else {
            let rewrite = text.replace(&doubled, &format!("\\{q}"));
            (
                Some(format!("escape the quote with a backslash: `{rewrite}`")),
                Some(rewrite),
            )
        };
        match (help, rewrite) {
            (Some(help), Some(rewrite)) => {
                let help = if rewrite.chars().count() <= MAX_SNIPPET {
                    help
                } else {
                    help.split(": `").next().unwrap_or_default().to_string()
                };
                d = d.help_with(help, Suggestion::replace(self.span(start, end), rewrite));
            }
            _ => d = d.help("escape a quote inside a string with a backslash: `'it\\'s'`"),
        }
        self.errors.push(d);
    }

    /// DS0003 for a number with more than one decimal point, `1.2.3`, read as `1.2` then `.3`.
    /// `first` is the token index of the number; the parts after it have been consumed.
    pub(super) fn dotted_number(&mut self, first: usize) {
        let start = self.tokens[first].start;
        let end = self.prev_end();
        let text = self.source(start, end).to_string();
        let mut d = Diagnostic::error(codes::DS0003, format!("invalid number `{text}`")).primary(
            self.span(start, end),
            "a number has at most one decimal point",
        );
        if text.chars().all(|c| c.is_ascii_digit() || c == '.') {
            d = d.help_with(
                format!("if this is a version or an address, write it as a string: `'{text}'`"),
                Suggestion::new(
                    vec![TextEdit::new(self.span(start, end), format!("'{text}'"))],
                    Applicability::MaybeIncorrect,
                ),
            );
        }
        self.errors.push(d);
    }

    // --- Names, lists and sources ---

    /// DS0126: a reserved word where a name belongs. The current token is the word.
    pub(super) fn reserved_word_as_name(&mut self) {
        let word = self.nth_text(0).to_string();
        let span = self.current_span();
        let quoted = format!("['{word}']");
        let d = Diagnostic::error(codes::DS0126, format!("`{word}` is a reserved word"))
            .primary(span, "a reserved word cannot be a name without quotes")
            .help_with(
                format!("to use it as a name, quote it: `{quoted}`"),
                Suggestion::replace(span, quoted),
            );
        self.report(d);
    }

    /// DS0127: a `,` with nothing after it at the end of a list. The current token is the comma.
    pub(super) fn trailing_comma(&mut self) {
        let comma = self.current_span();
        let after = match self.nth(1) {
            PIPE => "before `|`",
            SEMICOLON => "before `;`",
            R_PAREN => "before `)`",
            R_BRACK => "before `]`",
            _ => "at the end of the input",
        };
        // Before a closing bracket, remove the space after the comma too: `f(a, )` → `f(a)`.
        let removed = match self.tokens.get(self.pos + 1) {
            Some(next) if matches!(next.kind, R_PAREN | R_BRACK) => {
                self.span(comma.start, next.start)
            }
            _ => comma,
        };
        let d = Diagnostic::error(codes::DS0127, format!("trailing `,` {after}"))
            .primary(comma, "nothing follows this `,` in the list")
            .help_with("remove the `,`", Suggestion::replace(removed, ""));
        self.report(d);
    }

    /// DS0125: a query that starts with an operator, `where a > 1`. The current token is the
    /// operator name.
    pub(super) fn operator_without_source(&mut self) {
        let name = self.op_name_text();
        let span = self.span(self.offset(), self.tokens[self.pos + name.1 - 1].end());
        let end = self.tokens[self.pos..]
            .iter()
            .take_while(|t| !matches!(t.kind, SEMICOLON | LET_KW))
            .last()
            .map_or(span.end, |t| t.end());
        let help = match self.snippet(span.start, end) {
            Some(query) => format!(
                "start with the table and pipe it into `{0}`: `'data.csv' | {query}`",
                name.0
            ),
            None => format!(
                "start with the table and pipe it into `{0}`: `'data.csv' | {0} ...`",
                name.0
            ),
        };
        let d = Diagnostic::error(
            codes::DS0125,
            format!("a query starts with a table, not with `{}`", name.0),
        )
        .primary(span, format!("`{}` is an operator", name.0))
        .help(help);
        self.report(d);
    }

    /// The operator name at the current token, as written (hyphenated names joined), and how
    /// many tokens it spans.
    fn op_name_text(&self) -> (String, usize) {
        if self.nth(1) == MINUS && self.nth(2) == IDENT {
            let name = format!("{}-{}", self.nth_text(0), self.nth_text(2));
            if OPERATORS.contains(&name.as_str()) {
                return (name, 3);
            }
        }
        (self.nth_text(0).to_string(), 1)
    }

    /// DS0128: `in [1, 2]`. `open` is the token index of `[`; the list has been parsed.
    pub(super) fn in_brackets(&mut self, open: usize) {
        let open_span = self.token_span(open);
        let mut d = Diagnostic::error(codes::DS0128, "the list after `in` goes in parentheses")
            .primary(open_span, "expected `(`");
        let closed = self.tokens[self.pos - 1].kind == R_BRACK && self.pos - 1 > open;
        if closed {
            let close_span = self.token_span(self.pos - 1);
            let list = self.source(open_span.end, close_span.start);
            let op = match self.tokens[open - 1].kind {
                TILDE => format!("{}~", self.tokens[open - 2].text(self.text)),
                _ => self.tokens[open - 1].text(self.text).to_string(),
            };
            let help = match self.snippet(open_span.end, close_span.start) {
                Some(_) => format!("write the list in parentheses: `{op} ({list})`"),
                None => "write the list in parentheses".to_string(),
            };
            d = d.help_with(
                help,
                Suggestion::new(
                    vec![
                        TextEdit::new(open_span, "("),
                        TextEdit::new(close_span, ")"),
                    ],
                    Applicability::MachineApplicable,
                ),
            );
        } else {
            d = d.help("write the list in parentheses: `x in (1, 2)`");
        }
        if !self.errors_at(open) {
            self.errors.push(d);
        }
    }

    /// DS0128 for `in ['a']`, which the lexer reads as the quoted name `['a']`. The current
    /// token is that name.
    pub(super) fn in_quoted_brackets(&mut self) {
        let span = self.current_span();
        let text = self.nth_text(0);
        let list = &text[1..text.len() - 1];
        let op = self.prev_text().unwrap_or("in").to_string();
        let rewrite = format!("({list})");
        let d = Diagnostic::error(codes::DS0128, "the list after `in` goes in parentheses")
            .primary(span, "this is read as a quoted name")
            .help_with(
                format!("write the list in parentheses: `{op} {rewrite}`"),
                Suggestion::replace(span, rewrite),
            );
        self.report(d);
    }

    /// DS0129: `in ~` or `!in ~` with a space before `~`. `op` is the token index of the
    /// operator; the current token is `~`.
    pub(super) fn spaced_in_tilde(&mut self, op: usize) {
        let op_text = self.tokens[op].text(self.text).to_string();
        let span = self.span(self.tokens[op].start, self.current_span().end);
        let joined = format!("{op_text}~");
        let d = Diagnostic::error(
            codes::DS0129,
            format!("`{joined}` is written without a space"),
        )
        .primary(span, format!("read as `{op_text}` followed by `~`"))
        .help_with(
            format!("write `{joined}` to match the list ignoring case"),
            Suggestion::replace(span, joined.clone()),
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
        let d = Diagnostic::error(codes::DS0106, "missing `;` between statements");
        let list = self.last_op.filter(|kind| LIST_OPERATORS.contains(kind));
        let d = match list {
            // `| project a⏎  b`: more likely a forgotten comma than a second query.
            Some(_) if !self.at(LET_KW) => {
                let d = d
                    .primary(self.span(end, end), "expected `,` or `;` here")
                    .secondary(self.current_span(), "this is read as a new statement");
                let next = self.snippet(self.offset(), self.current_span().end);
                let help = match next {
                    Some(next) => format!("if `{next}` continues the list, add `,`"),
                    None => "if this continues the list, add `,`".to_string(),
                };
                d.help_with(
                    help,
                    Suggestion::new(
                        vec![TextEdit::new(self.span(end, end), ",")],
                        Applicability::MaybeIncorrect,
                    ),
                )
                .help_with(
                    "if it is a new statement, end the previous one with `;`",
                    Suggestion::new(
                        vec![TextEdit::new(self.span(end, end), ";")],
                        Applicability::MaybeIncorrect,
                    ),
                )
            }
            _ => d
                .primary(self.span(end, end), "expected `;` here")
                .secondary(self.current_span(), "a new statement starts here")
                .help_with(
                    "end the statement with `;`",
                    Suggestion::replace(self.span(end, end), ";"),
                ),
        };
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
