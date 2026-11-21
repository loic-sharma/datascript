//! The lexer: source text to tokens, trivia included.
//!
//! The lexer is context-free (D-026): what a piece of text lexes to never depends on the tokens
//! before it. It never fails: text it cannot make sense of becomes an [`ERROR`] token with a
//! diagnostic, and the tokens always cover the whole input, so concatenating their texts gives the
//! input back.
//!
//! [`ERROR`]: SyntaxKind::ERROR

use datascript_diagnostics::{Applicability, Diagnostic, FileId, Span, Suggestion, codes};

use crate::SyntaxKind::{self, *};
use crate::literal;

/// One token: its kind and where it is in the input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Token {
    /// What the token is.
    pub kind: SyntaxKind,
    /// Byte offset of the first byte.
    pub start: u32,
    /// Length in bytes. Never zero.
    pub len: u32,
}

impl Token {
    /// Byte offset just past the last byte.
    pub fn end(&self) -> u32 {
        self.start + self.len
    }

    /// The byte range of the token.
    pub fn range(&self) -> std::ops::Range<usize> {
        self.start as usize..self.end() as usize
    }

    /// The token's text in `source`, the text that was lexed.
    pub fn text<'a>(&self, source: &'a str) -> &'a str {
        &source[self.range()]
    }
}

/// The output of [`lex`]: the tokens and the lexical diagnostics (DS0001–DS0019).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Lexed {
    /// Tokens in source order. They cover the input exactly, with no gaps or overlaps.
    pub tokens: Vec<Token>,
    /// Diagnostics in source order.
    pub diagnostics: Vec<Diagnostic>,
}

/// Lexes `text`, the contents of file `file`. Diagnostic spans point into that file.
///
/// ```
/// use datascript_diagnostics::FileId;
/// use datascript_syntax::{lex, SyntaxKind};
///
/// let source = "x | project-away y";
/// let lexed = lex(source, FileId(0));
/// let kinds: Vec<SyntaxKind> = lexed
///     .tokens
///     .iter()
///     .map(|t| t.kind)
///     .filter(|k| !k.is_trivia())
///     .collect();
/// use SyntaxKind::*;
/// assert_eq!(kinds, [IDENT, PIPE, IDENT, MINUS, IDENT, IDENT]);
/// assert!(lexed.diagnostics.is_empty());
/// ```
pub fn lex(text: &str, file: FileId) -> Lexed {
    let mut lexer = Lexer {
        src: text,
        pos: 0,
        file,
        out: Lexed::default(),
    };
    while lexer.pos < text.len() {
        let start = lexer.pos;
        let kind = lexer.token();
        debug_assert!(lexer.pos > start, "the lexer must always make progress");
        lexer.out.tokens.push(Token {
            kind,
            start: start as u32,
            len: (lexer.pos - start) as u32,
        });
    }
    lexer.out
}

/// Words that `!` turns into a negated operator (LANGUAGE.md §1).
const NEGATABLE: &[&str] = &[
    "in",
    "between",
    "has",
    "contains",
    "startswith",
    "endswith",
    "has_cs",
    "contains_cs",
    "startswith_cs",
    "endswith_cs",
];

struct Lexer<'a> {
    src: &'a str,
    pos: usize,
    file: FileId,
    out: Lexed,
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_ident_continue(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Characters that make a word an identifier-like run. Non-ASCII letters are included so that
/// `naïve` is one bad name rather than `na`, an error, and `ve`.
fn is_word_char(c: char) -> bool {
    is_ident_continue(c) || (!c.is_ascii() && c.is_alphanumeric())
}

fn is_line_end(c: char) -> bool {
    c == '\n' || c == '\r'
}

impl Lexer<'_> {
    fn rest(&self) -> &str {
        &self.src[self.pos..]
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn peek_at(&self, n: usize) -> Option<char> {
        self.rest().chars().nth(n)
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.pos += c.len_utf8();
            true
        } else {
            false
        }
    }

    fn eat_while(&mut self, f: impl Fn(char) -> bool) {
        while self.peek().is_some_and(&f) {
            self.bump();
        }
    }

    fn span(&self, start: usize, end: usize) -> Span {
        Span::new(self.file, start as u32, end as u32)
    }

    fn report(&mut self, diagnostics: Vec<Diagnostic>) {
        self.out.diagnostics.extend(diagnostics);
    }

    /// Lexes one token starting at `self.pos` and returns its kind.
    fn token(&mut self) -> SyntaxKind {
        let start = self.pos;
        let c = self.peek().expect("token() is only called before the end");
        let next = self.peek_at(1);
        if c.is_whitespace() || c == '\u{FEFF}' {
            self.eat_while(|c| c.is_whitespace() || c == '\u{FEFF}');
            return WHITESPACE;
        }
        if c.is_ascii_digit() || (c == '.' && next.is_some_and(|n| n.is_ascii_digit())) {
            return self.number(start);
        }
        if is_word_char(c) {
            return self.word(start);
        }
        self.bump();
        match c {
            '/' if next == Some('/') => {
                self.eat_while(|c| !is_line_end(c));
                LINE_COMMENT
            }
            '/' if next == Some('*') => self.block_comment(start),
            '/' => SLASH,
            '"' | '\'' => self.string(start, c),
            '@' if matches!(next, Some('"' | '\'')) => {
                let quote = self.bump().unwrap_or('"');
                self.string(start, quote)
            }
            '[' => self.quoted_ident(start),
            '!' => self.bang(),
            '=' if self.eat('=') => EQ2,
            '=' if self.eat('~') => EQ_TILDE,
            '=' => EQ,
            '<' if self.eat('=') => LTEQ,
            '<' if self.eat('>') => LTGT,
            '<' => LT,
            '>' if self.eat('=') => GTEQ,
            '>' => GT,
            '|' if self.eat('|') => PIPE2,
            '|' => PIPE,
            '&' if self.eat('&') => AMP2,
            '.' if self.eat('.') => DOT2,
            '.' => DOT,
            '$' => self.dollar(start),
            ';' => SEMICOLON,
            ',' => COMMA,
            '(' => L_PAREN,
            ')' => R_PAREN,
            ']' => R_BRACK,
            '{' => L_BRACE,
            '}' => R_BRACE,
            ':' => COLON,
            '+' => PLUS,
            '-' => MINUS,
            '*' => STAR,
            '%' => PERCENT,
            '~' => TILDE,
            '`' => self.backtick_name(start),
            _ => self.unexpected(start, c),
        }
    }

    fn block_comment(&mut self, start: usize) -> SyntaxKind {
        self.bump(); // `*`
        match self.rest().find("*/") {
            Some(i) => self.pos += i + 2,
            None => {
                self.pos = self.src.len();
                let d = Diagnostic::error(codes::DS0006, "unterminated block comment")
                    .primary(self.span(start, start + 2), "this comment is never closed")
                    .help("close it with `*/`")
                    .note("block comments do not nest");
                self.report(vec![d]);
            }
        }
        BLOCK_COMMENT
    }

    /// Numbers: `42`, `1_000`, `3.14`, `.5`, `1e-3`, and timespans `1.5h` (a number with a unit
    /// suffix). `1.` is `1` then `.`, and `1..10` is `1` `..` `10`.
    fn number(&mut self, start: usize) -> SyntaxKind {
        let digit_or_underscore = |c: char| c.is_ascii_digit() || c == '_';
        let mut is_real = false;
        self.eat_while(digit_or_underscore);
        if self.peek() == Some('.') && self.peek_at(1).is_some_and(|c| c.is_ascii_digit()) {
            self.bump();
            self.eat_while(digit_or_underscore);
            is_real = true;
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            let exponent = match self.peek_at(1) {
                Some(c) if c.is_ascii_digit() => true,
                Some('+' | '-') => self.peek_at(2).is_some_and(|c| c.is_ascii_digit()),
                _ => false,
            };
            if exponent {
                self.bump();
                if matches!(self.peek(), Some('+' | '-')) {
                    self.bump();
                }
                self.eat_while(digit_or_underscore);
                is_real = true;
            }
        }
        let kind = if self.peek().is_some_and(|c| is_word_char(c) || c == 'µ') {
            self.eat_while(|c| is_word_char(c) || c == 'µ');
            TIMESPAN
        } else if is_real {
            REAL
        } else {
            LONG
        };
        let text = &self.src[start..self.pos];
        let span = self.span(start, self.pos);
        let result = match kind {
            LONG => literal::long(text, span).map(drop),
            REAL => literal::real(text, span).map(drop),
            _ => literal::timespan(text, span).map(drop),
        };
        if let Err(diagnostics) = result {
            self.report(diagnostics);
        }
        kind
    }

    /// Identifiers, reserved words, `in~`, and `datetime(...)` / `timespan(...)` literals.
    fn word(&mut self, start: usize) -> SyntaxKind {
        self.eat_while(is_word_char);
        let word = &self.src[start..self.pos];
        if !word.is_ascii() || !word.starts_with(is_ident_start) {
            return self.bad_name(start);
        }
        match word {
            "datetime" | "timespan" if self.peek() == Some('(') => {
                let kind = if word == "datetime" {
                    DATETIME
                } else {
                    TIMESPAN
                };
                self.bump();
                self.eat_while(|c| c != ')' && !is_line_end(c));
                self.eat(')');
                let text = &self.src[start..self.pos];
                let span = self.span(start, self.pos);
                let result = if kind == DATETIME {
                    literal::datetime(text, span).map(drop)
                } else {
                    literal::timespan(text, span).map(drop)
                };
                if let Err(diagnostics) = result {
                    self.report(diagnostics);
                }
                kind
            }
            "in" if self.eat('~') => IN_TILDE,
            _ => SyntaxKind::from_reserved_word(word).unwrap_or(IDENT),
        }
    }

    /// A word with non-ASCII letters, which must be quoted to be a name.
    fn bad_name(&mut self, start: usize) -> SyntaxKind {
        let word = &self.src[start..self.pos];
        let span = self.span(start, self.pos);
        let quoted = if word.contains('\'') {
            format!("[\"{word}\"]")
        } else {
            format!("['{word}']")
        };
        let d = Diagnostic::error(
            codes::DS0001,
            format!("names with non-ASCII characters must be quoted: `{word}`"),
        )
        .primary(span, "unquoted name with non-ASCII characters")
        .help_with(
            format!("quote the name: `{quoted}`"),
            Suggestion::new(
                vec![datascript_diagnostics::TextEdit::new(span, quoted.clone())],
                Applicability::MaybeIncorrect,
            ),
        )
        .note("unquoted names use only the letters A-Z and a-z, digits and `_`");
        self.report(vec![d]);
        ERROR
    }

    /// The end of a string starting at `start` with the opening quote already consumed: just past
    /// the closing quote, or before the end of the line if it is never closed.
    fn scan_string(&mut self, quote: char, verbatim: bool) {
        while let Some(c) = self.peek() {
            if is_line_end(c) {
                return;
            }
            self.bump();
            if c == quote {
                return;
            }
            if c == '\\' && !verbatim && self.peek().is_some_and(|n| !is_line_end(n)) {
                self.bump();
            }
        }
    }

    fn string(&mut self, start: usize, quote: char) -> SyntaxKind {
        let verbatim = self.src[start..].starts_with('@');
        self.scan_string(quote, verbatim);
        let text = &self.src[start..self.pos];
        if let Err(diagnostics) = literal::string(text, self.span(start, self.pos)) {
            self.report(diagnostics);
        }
        STRING
    }

    /// `['name']` or `["name"]` as one token; any other `[` is just `[`.
    fn quoted_ident(&mut self, start: usize) -> SyntaxKind {
        let Some(quote @ ('"' | '\'')) = self.peek() else {
            return L_BRACK;
        };
        let after_bracket = self.pos;
        self.bump();
        self.scan_string(quote, false);
        let closed = self.pos - after_bracket >= 2 && self.src[..self.pos].ends_with(quote);
        if !(closed && self.eat(']')) {
            self.pos = after_bracket;
            return L_BRACK;
        }
        let text = &self.src[start..self.pos];
        if let Err(diagnostics) = literal::quoted_ident(text, self.span(start, self.pos)) {
            self.report(diagnostics);
        }
        QUOTED_IDENT
    }

    /// `!=`, `!~`, negated operators (`!has`, `!in~`), or a lone `!`.
    fn bang(&mut self) -> SyntaxKind {
        if self.eat('=') {
            return NEQ;
        }
        if self.eat('~') {
            return BANG_TILDE;
        }
        let word_len = self
            .rest()
            .find(|c: char| !is_word_char(c))
            .unwrap_or(self.rest().len());
        let word = &self.rest()[..word_len];
        if NEGATABLE.contains(&word) {
            let is_in = word == "in";
            self.pos += word_len;
            if is_in {
                self.eat('~');
            }
            return NEGATED_OP;
        }
        BANG
    }

    fn dollar(&mut self, start: usize) -> SyntaxKind {
        let word_len = self
            .rest()
            .find(|c: char| !is_word_char(c))
            .unwrap_or(self.rest().len());
        match &self.rest()[..word_len] {
            "left" => {
                self.pos += word_len;
                DOLLAR_LEFT
            }
            "right" => {
                self.pos += word_len;
                DOLLAR_RIGHT
            }
            _ => self.unexpected(start, '$'),
        }
    }

    /// `` `name` ``, the SQL way of quoting a name, as one error token with a fix.
    fn backtick_name(&mut self, start: usize) -> SyntaxKind {
        let line = self.rest().split(is_line_end).next().unwrap_or("");
        let Some(close) = line.find('`').filter(|&i| i > 0) else {
            return self.unexpected(start, '`');
        };
        let name = line[..close].to_string();
        self.pos += close + 1;
        let span = self.span(start, self.pos);
        let quoted = if name.contains('\'') {
            format!("[\"{name}\"]")
        } else {
            format!("['{name}']")
        };
        let d = Diagnostic::error(codes::DS0001, "names are not quoted with backticks")
            .primary(span, "backticks are not a quote")
            .help_with(
                format!("quote the name with brackets: `{quoted}`"),
                Suggestion::new(
                    vec![datascript_diagnostics::TextEdit::new(span, quoted.clone())],
                    Applicability::MachineApplicable,
                ),
            );
        self.report(vec![d]);
        ERROR
    }

    /// A character that starts no token. Runs of the same character are reported once.
    fn unexpected(&mut self, start: usize, c: char) -> SyntaxKind {
        if !matches!(c, '$' | '@' | '&') {
            self.eat_while(|n| n == c);
        }
        let run = &self.src[start..self.pos];
        let span = self.span(start, self.pos);
        let shown = describe(c);
        let message = if run.chars().count() > 1 {
            format!("unexpected characters {shown}")
        } else {
            format!("unexpected character {shown}")
        };
        let mut d = Diagnostic::error(codes::DS0001, message).primary(span, "not valid here");
        if let Some(help) = help_for(c) {
            d = d.help(help);
        }
        self.report(vec![d]);
        ERROR
    }
}

/// How a character is shown in a message: in backticks, or as `U+XXXX` when it is invisible.
fn describe(c: char) -> String {
    if c == '`' {
        "`` ` ``".to_string()
    } else if c.is_control() || is_invisible(c) {
        format!("U+{:04X}", c as u32)
    } else {
        format!("`{c}`")
    }
}

fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}' | '\u{200B}'..='\u{200F}' | '\u{2028}'..='\u{202E}' | '\u{2060}'..='\u{2064}'
    )
}

fn help_for(c: char) -> Option<&'static str> {
    Some(match c {
        '#' => "comments start with `//`",
        '`' => "to quote a name, write `['name']`",
        '\\' => "a backslash is only allowed inside a string",
        '^' => "to raise to a power, use `pow(x, y)`",
        '&' => "to combine conditions, use `and`",
        '$' => "`$` is only used in `$left` and `$right`",
        '@' => "a verbatim string is `@` followed directly by a quote: `@'C:\\data'`",
        '?' => "for a conditional value, use `iff(condition, then, else)`",
        '\u{201C}' | '\u{201D}' | '\u{2018}' | '\u{2019}' => {
            "this is a typographic quote; use a plain `\"` or `'`"
        }
        c if c.is_control() || is_invisible(c) => {
            "delete it; invisible characters are often pasted in by accident"
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<(SyntaxKind, &str)> {
        let lexed = lex(text, FileId(0));
        lexed
            .tokens
            .iter()
            .filter(|t| !t.kind.is_trivia())
            .map(|t| (t.kind, t.text(text)))
            .collect()
    }

    fn only(text: &str) -> SyntaxKind {
        let tokens = kinds(text);
        assert_eq!(tokens.len(), 1, "{text:?} lexed as {tokens:?}");
        assert!(
            lex(text, FileId(0)).diagnostics.is_empty(),
            "{text:?} has diagnostics"
        );
        tokens[0].0
    }

    fn errors(text: &str) -> Vec<Diagnostic> {
        lex(text, FileId(0)).diagnostics
    }

    #[test]
    fn trivia() {
        let text = "a // note\n/* block\n */ b";
        let lexed = lex(text, FileId(0));
        let all: Vec<_> = lexed.tokens.iter().map(|t| t.kind).collect();
        assert_eq!(
            all,
            [
                IDENT,
                WHITESPACE,
                LINE_COMMENT,
                WHITESPACE,
                BLOCK_COMMENT,
                WHITESPACE,
                IDENT
            ]
        );
        assert_eq!(lexed.tokens[2].text(text), "// note");
        assert_eq!(only("\u{FEFF} \t\r\n\u{00A0}x"), IDENT);
    }

    #[test]
    fn unterminated_block_comment() {
        let d = &errors("x /* never closed")[0];
        assert_eq!(d.code, codes::DS0006);
        assert_eq!(d.primary.as_ref().unwrap().span.range(), 2..4);
        assert_eq!(kinds("x /* a"), [(IDENT, "x")]);
    }

    #[test]
    fn block_comments_do_not_nest() {
        assert_eq!(
            kinds("/* a /* b */ c */"),
            [(IDENT, "c"), (STAR, "*"), (SLASH, "/")]
        );
    }

    #[test]
    fn punctuation() {
        let text =
            "| ; , ( ) [ ] { } . .. : = == != < <= > >= <> =~ !~ + - * / % ! ~ && || $left $right";
        let expected = [
            PIPE,
            SEMICOLON,
            COMMA,
            L_PAREN,
            R_PAREN,
            L_BRACK,
            R_BRACK,
            L_BRACE,
            R_BRACE,
            DOT,
            DOT2,
            COLON,
            EQ,
            EQ2,
            NEQ,
            LT,
            LTEQ,
            GT,
            GTEQ,
            LTGT,
            EQ_TILDE,
            BANG_TILDE,
            PLUS,
            MINUS,
            STAR,
            SLASH,
            PERCENT,
            BANG,
            TILDE,
            AMP2,
            PIPE2,
            DOLLAR_LEFT,
            DOLLAR_RIGHT,
        ];
        let got: Vec<_> = kinds(text).into_iter().map(|(k, _)| k).collect();
        assert_eq!(got, expected);
        assert!(errors(text).is_empty());
    }

    #[test]
    fn reserved_words_and_contextual_keywords() {
        for (word, kind) in [
            ("let", LET_KW),
            ("true", TRUE_KW),
            ("false", FALSE_KW),
            ("null", NULL_KW),
            ("and", AND_KW),
            ("or", OR_KW),
            ("not", NOT_KW),
        ] {
            assert_eq!(only(word), kind);
        }
        for word in [
            "where", "count", "order", "by", "kind", "in", "has", "Let", "TRUE", "_x1",
        ] {
            assert_eq!(only(word), IDENT, "{word}");
        }
    }

    #[test]
    fn quoted_identifiers() {
        assert_eq!(kinds("['order id']"), [(QUOTED_IDENT, "['order id']")]);
        assert_eq!(kinds(r#"["x"]"#), [(QUOTED_IDENT, r#"["x"]"#)]);
        // Not a quoted identifier: a list of strings, or a space inside the brackets.
        assert_eq!(
            kinds("['a', 'b']"),
            [
                (L_BRACK, "["),
                (STRING, "'a'"),
                (COMMA, ","),
                (STRING, "'b'"),
                (R_BRACK, "]")
            ]
        );
        assert_eq!(
            kinds("[ 'a' ]"),
            [(L_BRACK, "["), (STRING, "'a'"), (R_BRACK, "]")]
        );
        assert_eq!(
            kinds("x[0]"),
            [(IDENT, "x"), (L_BRACK, "["), (LONG, "0"), (R_BRACK, "]")]
        );
    }

    #[test]
    fn numbers() {
        assert_eq!(only("42"), LONG);
        assert_eq!(only("1_000_000"), LONG);
        assert_eq!(only("9223372036854775808"), LONG);
        assert_eq!(only("3.14"), REAL);
        assert_eq!(only("1e-3"), REAL);
        assert_eq!(only("1E+3"), REAL);
        assert_eq!(only("2e10"), REAL);
        assert_eq!(only(".5"), REAL);
        assert_eq!(kinds("1."), [(LONG, "1"), (DOT, ".")]);
        assert_eq!(kinds("1.x"), [(LONG, "1"), (DOT, "."), (IDENT, "x")]);
        assert_eq!(kinds("-7"), [(MINUS, "-"), (LONG, "7")]);
        assert_eq!(kinds("a-1"), [(IDENT, "a"), (MINUS, "-"), (LONG, "1")]);
    }

    #[test]
    fn ranges() {
        assert_eq!(kinds("1..10"), [(LONG, "1"), (DOT2, ".."), (LONG, "10")]);
        assert_eq!(kinds("1.5..2"), [(REAL, "1.5"), (DOT2, ".."), (LONG, "2")]);
        assert_eq!(kinds("x .. .5"), [(IDENT, "x"), (DOT2, ".."), (REAL, ".5")]);
    }

    #[test]
    fn number_errors() {
        let d = &errors("99999999999999999999")[0];
        assert_eq!(d.code, codes::DS0003);
        assert_eq!(d.message, "number is too large for a `long`");
        let d = &errors("1_000_")[0];
        assert_eq!(d.primary.as_ref().unwrap().span.range(), 5..6);
    }

    #[test]
    fn timespans() {
        for text in [
            "1d", "2h", "30m", "10s", "250ms", "5us", "5µs", "3tick", "1.5h", ".5d",
        ] {
            assert_eq!(only(text), TIMESPAN, "{text}");
        }
        assert_eq!(only("timespan(1.02:03:04)"), TIMESPAN);
        assert_eq!(only("timespan(null)"), TIMESPAN);
        // `timespan` not directly followed by `(` is a name.
        assert_eq!(
            kinds("timespan (1)"),
            [
                (IDENT, "timespan"),
                (L_PAREN, "("),
                (LONG, "1"),
                (R_PAREN, ")")
            ]
        );
    }

    #[test]
    fn bad_timespan_unit() {
        let text = "x > 1x";
        let lexed = lex(text, FileId(0));
        assert_eq!(lexed.tokens.last().unwrap().kind, TIMESPAN);
        let d = &lexed.diagnostics[0];
        assert_eq!(d.code, codes::DS0003);
        assert_eq!(d.message, "unknown timespan unit `x`");
        assert_eq!(&text[d.primary.as_ref().unwrap().span.range()], "x");
        let help = &d.help[0].message;
        for unit in ["`d`", "`h`", "`m`", "`s`", "`ms`", "`us`", "`µs`", "`tick`"] {
            assert!(help.contains(unit), "{help} lacks {unit}");
        }
    }

    #[test]
    fn datetimes() {
        assert_eq!(
            kinds("datetime(2024-01-31)"),
            [(DATETIME, "datetime(2024-01-31)")]
        );
        assert_eq!(only("datetime(2024-01-31T12:00:00Z)"), DATETIME);
        assert_eq!(only("datetime(null)"), DATETIME);
        assert_eq!(
            kinds("datetime(2024-01-31) + 1d"),
            [
                (DATETIME, "datetime(2024-01-31)"),
                (PLUS, "+"),
                (TIMESPAN, "1d")
            ]
        );
        // Not followed directly by `(`: an ordinary name, e.g. a column.
        assert_eq!(kinds("datetime"), [(IDENT, "datetime")]);
        assert_eq!(
            kinds("todatetime(x)"),
            [
                (IDENT, "todatetime"),
                (L_PAREN, "("),
                (IDENT, "x"),
                (R_PAREN, ")")
            ]
        );
    }

    #[test]
    fn unclosed_datetime_stops_at_end_of_line() {
        let text = "datetime(2024-01-31\n| take 1";
        let lexed = lex(text, FileId(0));
        assert_eq!(lexed.tokens[0].text(text), "datetime(2024-01-31");
        assert_eq!(lexed.diagnostics[0].message, "`datetime(` is never closed");
    }

    #[test]
    fn strings() {
        assert_eq!(only(r#""text""#), STRING);
        assert_eq!(only("'text'"), STRING);
        assert_eq!(only(r#"@"C:\raw""#), STRING);
        assert_eq!(only(r"@'C:\raw'"), STRING);
        assert_eq!(only(r#"'\\ \" \' \n \r \t \0 \u{1F600}'"#), STRING);
        assert_eq!(kinds(r"'a\'b' c"), [(STRING, r"'a\'b'"), (IDENT, "c")]);
    }

    #[test]
    fn unterminated_string_spans_to_end_of_line() {
        let text = "x == 'abc\n| take 1";
        let lexed = lex(text, FileId(0));
        let d = &lexed.diagnostics[0];
        assert_eq!(d.code, codes::DS0002);
        assert_eq!(&text[d.primary.as_ref().unwrap().span.range()], "'abc");
        assert!(d.help[0].message.contains("closing `'`"));
        // The next line lexes normally.
        assert_eq!(
            kinds(text)[3..],
            [(PIPE, "|"), (IDENT, "take"), (LONG, "1")]
        );
    }

    #[test]
    fn windows_path_escape() {
        let text = r"'C:\data\x.csv'";
        let diagnostics = errors(text);
        assert_eq!(diagnostics.len(), 1);
        let d = &diagnostics[0];
        assert_eq!(d.code, codes::DS0005);
        assert_eq!(d.message, r"unknown escape sequence `\d`");
        let help = &d.help[0].message;
        assert!(help.contains(r"@'C:\data\x.csv'"), "{help}");
        assert!(help.contains("'C:/data/x.csv'"), "{help}");
        assert!(help.contains("forward slashes"), "{help}");
    }

    #[test]
    fn bang_forms() {
        for op in NEGATABLE {
            let text = format!("!{op}");
            assert_eq!(kinds(&text), [(NEGATED_OP, text.as_str())]);
        }
        assert_eq!(kinds("!has"), [(NEGATED_OP, "!has")]);
        assert_eq!(kinds("!has_error"), [(BANG, "!"), (IDENT, "has_error")]);
        assert_eq!(kinds("!x"), [(BANG, "!"), (IDENT, "x")]);
        assert_eq!(kinds("! has"), [(BANG, "!"), (IDENT, "has")]);
        assert_eq!(kinds("!in~"), [(NEGATED_OP, "!in~")]);
        assert_eq!(kinds("!in ~"), [(NEGATED_OP, "!in"), (TILDE, "~")]);
        assert_eq!(kinds("!between"), [(NEGATED_OP, "!between")]);
        assert_eq!(kinds("!=x"), [(NEQ, "!="), (IDENT, "x")]);
        assert_eq!(kinds("!~"), [(BANG_TILDE, "!~")]);
    }

    #[test]
    fn in_tilde() {
        assert_eq!(
            kinds("x in~ ('a')"),
            [
                (IDENT, "x"),
                (IN_TILDE, "in~"),
                (L_PAREN, "("),
                (STRING, "'a'"),
                (R_PAREN, ")")
            ]
        );
        assert_eq!(kinds("in ~"), [(IDENT, "in"), (TILDE, "~")]);
        assert_eq!(kinds("min~"), [(IDENT, "min"), (TILDE, "~")]);
    }

    #[test]
    fn hyphenated_operator_names_are_three_tokens() {
        assert_eq!(
            kinds("project-away"),
            [(IDENT, "project"), (MINUS, "-"), (IDENT, "away")]
        );
        assert_eq!(
            kinds("| mv-expand"),
            [(PIPE, "|"), (IDENT, "mv"), (MINUS, "-"), (IDENT, "expand")]
        );
    }

    #[test]
    fn unexpected_characters() {
        let text = "x ## y";
        let lexed = lex(text, FileId(0));
        assert_eq!(lexed.diagnostics.len(), 1);
        let d = &lexed.diagnostics[0];
        assert_eq!(d.code, codes::DS0001);
        assert_eq!(d.message, "unexpected characters `#`");
        assert_eq!(&text[d.primary.as_ref().unwrap().span.range()], "##");
        assert_eq!(kinds(text)[1], (ERROR, "##"));

        assert_eq!(errors("$foo")[0].message, "unexpected character `$`");
        assert_eq!(kinds("$foo"), [(ERROR, "$"), (IDENT, "foo")]);
        assert_eq!(kinds("a & b")[1], (ERROR, "&"));
        assert_eq!(kinds("@x")[0], (ERROR, "@"));
        assert_eq!(
            errors("a\u{200B}b")[0].message,
            "unexpected character U+200B"
        );
        assert_eq!(kinds("`order id` x"), [(ERROR, "`order id`"), (IDENT, "x")]);
        assert_eq!(errors("`")[0].message, "unexpected character `` ` ``");
    }

    #[test]
    fn non_ascii_names() {
        let text = "naïve + 1";
        let lexed = lex(text, FileId(0));
        assert_eq!(lexed.tokens[0].kind, ERROR);
        assert_eq!(lexed.tokens[0].text(text), "naïve");
        let d = &lexed.diagnostics[0];
        assert_eq!(d.code, codes::DS0001);
        assert!(d.help[0].message.contains("['naïve']"));
    }

    #[test]
    fn tokens_cover_input() {
        let text = "let x = 'a\n| where y !has_cs \"z\" // c\n/* open";
        let lexed = lex(text, FileId(0));
        let joined: String = lexed.tokens.iter().map(|t| t.text(text)).collect();
        assert_eq!(joined, text);
    }
}
