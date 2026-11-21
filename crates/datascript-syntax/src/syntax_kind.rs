//! The kinds of tokens (and, from M1-02, nodes) in the syntax tree.

/// The kind of a token or node. The lexer produces the token kinds; the parser (M1-02) adds node
/// kinds and uses this enum as the rowan `SyntaxKind`.
///
/// Keywords are contextual (LANGUAGE.md §1.1): only the seven reserved words have their own kind.
/// Every other word, including operator names like `where` and `project`, is an [`IDENT`].
///
/// [`IDENT`]: SyntaxKind::IDENT
#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u16)]
pub enum SyntaxKind {
    // Trivia
    /// Spaces, tabs, newlines and other Unicode white space.
    WHITESPACE = 0,
    /// `// ...` to the end of the line (the newline is not included).
    LINE_COMMENT,
    /// `/* ... */`, which does not nest. Unterminated ones run to the end of the input.
    BLOCK_COMMENT,

    // Punctuation and operators
    /// `|`
    PIPE,
    /// `;`
    SEMICOLON,
    /// `,`
    COMMA,
    /// `(`
    L_PAREN,
    /// `)`
    R_PAREN,
    /// `[`
    L_BRACK,
    /// `]`
    R_BRACK,
    /// `{`
    L_BRACE,
    /// `}`
    R_BRACE,
    /// `.`
    DOT,
    /// `..`
    DOT2,
    /// `:`
    COLON,
    /// `=`
    EQ,
    /// `==`
    EQ2,
    /// `!=`
    NEQ,
    /// `<`
    LT,
    /// `<=`
    LTEQ,
    /// `>`
    GT,
    /// `>=`
    GTEQ,
    /// `<>`: not an operator. Lexed so the parser can suggest `!=` (DS0107).
    LTGT,
    /// `=~`
    EQ_TILDE,
    /// `!~`
    BANG_TILDE,
    /// `+`
    PLUS,
    /// `-`. There are no negative number literals: `-7` is `-` then `7`.
    MINUS,
    /// `*`
    STAR,
    /// `/`
    SLASH,
    /// `%`
    PERCENT,
    /// `!` that does not start a negated operator. The parser reports it (DS0111).
    BANG,
    /// `~`
    TILDE,
    /// `&&`: not an operator. Lexed so the parser can suggest `and` (DS0111).
    AMP2,
    /// `||`: not an operator. Lexed so the parser can suggest `or` (DS0111).
    PIPE2,
    /// `$left`
    DOLLAR_LEFT,
    /// `$right`
    DOLLAR_RIGHT,
    /// A negated string or set operator: `!` directly followed by `in`, `between`, `has`,
    /// `contains`, `startswith`, `endswith`, a `_cs` variant of the last four, or `in~`. The
    /// token text says which (`!has`, `!in~`, …).
    NEGATED_OP,
    /// `in~`, case-insensitive `in`.
    IN_TILDE,

    // Reserved words
    /// `let`
    LET_KW,
    /// `true`
    TRUE_KW,
    /// `false`
    FALSE_KW,
    /// `null`
    NULL_KW,
    /// `and`
    AND_KW,
    /// `or`
    OR_KW,
    /// `not`
    NOT_KW,

    // Names and literals
    /// `[A-Za-z_][A-Za-z0-9_]*` that is not a reserved word.
    IDENT,
    /// `['name']` or `["name"]`, with no trivia inside the brackets. See
    /// [`literal::quoted_ident`](crate::literal::quoted_ident).
    QUOTED_IDENT,
    /// `42`, `1_000_000`. See [`literal::long`](crate::literal::long).
    LONG,
    /// `3.14`, `1e-3`, `.5`. See [`literal::real`](crate::literal::real).
    REAL,
    /// `"text"`, `'text'`, `@"verbatim"`. See [`literal::string`](crate::literal::string).
    STRING,
    /// `1d`, `1.5h`, `250ms`, or `timespan(1.02:03:04)` as one token. See
    /// [`literal::timespan`](crate::literal::timespan).
    TIMESPAN,
    /// `datetime(2024-01-31)` as one token. See [`literal::datetime`](crate::literal::datetime).
    DATETIME,

    /// Input the lexer could not make sense of. It always comes with a diagnostic.
    ERROR,
}

impl SyntaxKind {
    /// Whether the token is white space or a comment.
    pub fn is_trivia(self) -> bool {
        matches!(
            self,
            SyntaxKind::WHITESPACE | SyntaxKind::LINE_COMMENT | SyntaxKind::BLOCK_COMMENT
        )
    }

    /// Whether the token is one of the seven reserved words.
    pub fn is_reserved_word(self) -> bool {
        matches!(
            self,
            SyntaxKind::LET_KW
                | SyntaxKind::TRUE_KW
                | SyntaxKind::FALSE_KW
                | SyntaxKind::NULL_KW
                | SyntaxKind::AND_KW
                | SyntaxKind::OR_KW
                | SyntaxKind::NOT_KW
        )
    }

    /// Whether the token is a literal value (not counting `true`, `false` and `null`).
    pub fn is_literal(self) -> bool {
        matches!(
            self,
            SyntaxKind::LONG
                | SyntaxKind::REAL
                | SyntaxKind::STRING
                | SyntaxKind::TIMESPAN
                | SyntaxKind::DATETIME
        )
    }

    /// The kind of a reserved word, or `None` for any other text. Matching is case-sensitive.
    pub fn from_reserved_word(text: &str) -> Option<SyntaxKind> {
        Some(match text {
            "let" => SyntaxKind::LET_KW,
            "true" => SyntaxKind::TRUE_KW,
            "false" => SyntaxKind::FALSE_KW,
            "null" => SyntaxKind::NULL_KW,
            "and" => SyntaxKind::AND_KW,
            "or" => SyntaxKind::OR_KW,
            "not" => SyntaxKind::NOT_KW,
            _ => return None,
        })
    }
}

impl From<SyntaxKind> for u16 {
    fn from(kind: SyntaxKind) -> u16 {
        kind as u16
    }
}
