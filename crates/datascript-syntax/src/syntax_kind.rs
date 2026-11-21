//! The kinds of tokens and nodes in the syntax tree.

macro_rules! syntax_kinds {
    ($($(#[$meta:meta])* $name:ident,)*) => {
        /// The kind of a token or node, used as the rowan `SyntaxKind`.
        ///
        /// The lexer produces the token kinds (everything up to [`ERROR`]); the parser adds the
        /// node kinds after it. `ERROR` is both: a token the lexer rejected, or a node wrapping
        /// tokens the parser skipped.
        ///
        /// Keywords are contextual (LANGUAGE.md §1.1): only the seven reserved words have their
        /// own kind. Every other word, including operator names like `where` and `project`, is an
        /// [`IDENT`], also inside the tree.
        ///
        /// [`ERROR`]: SyntaxKind::ERROR
        /// [`IDENT`]: SyntaxKind::IDENT
        #[allow(non_camel_case_types)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[repr(u16)]
        pub enum SyntaxKind {
            $($(#[$meta])* $name,)*
        }

        impl SyntaxKind {
            /// Every kind, in declaration order, so `ALL[k as usize] == k`.
            pub const ALL: &'static [SyntaxKind] = &[$(SyntaxKind::$name,)*];
        }
    };
}

syntax_kinds! {
    // Trivia
    /// Spaces, tabs, newlines and other Unicode white space.
    WHITESPACE,
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

    /// The end of the input. Never in a tree: the parser uses it when it looks past the last
    /// token.
    EOF,
    /// As a token: input the lexer could not make sense of; it always comes with a diagnostic.
    /// As a node: tokens the parser could not fit into the grammar, after reporting an error.
    ERROR,

    // Nodes
    /// The whole file: statements separated by `;`.
    PROGRAM,
    /// `let name = value`. Tabular if the value is a `PIPELINE` (or one in parentheses), scalar
    /// otherwise: the kind is syntactic (D-036).
    LET_STMT,
    /// `from source | op | op`, the `from` token first (D-036). A query statement is always a
    /// pipeline, even with no operators, and so is `from source` anywhere.
    PIPELINE,

    /// `where predicate`
    WHERE_OP,
    /// `project a, b = expr`
    PROJECT_OP,
    /// `project-away a, b*`
    PROJECT_AWAY_OP,
    /// `project-rename new = old`
    PROJECT_RENAME_OP,
    /// `extend c = expr`
    EXTEND_OP,
    /// `summarize aggs by keys`
    SUMMARIZE_OP,
    /// `sort by keys` or `order by keys`
    SORT_OP,
    /// `top n by key`
    TOP_OP,
    /// `take n` or `limit n`
    TAKE_OP,
    /// `count`
    COUNT_OP,
    /// `distinct a, b` or `distinct *`
    DISTINCT_OP,
    /// `join kind = k right on conditions`
    JOIN_OP,
    /// `union t1, t2`
    UNION_OP,
    /// `render kind with (props)`
    RENDER_OP,
    /// `getschema`
    GETSCHEMA_OP,

    /// `[name =] expr`: a projection, aggregate, `by` key, or named argument.
    NAMED_EXPR,
    /// A column name with optional `*` wildcards, written without spaces: `a`, `col*`, `*_id`.
    COLUMN_PATTERN,
    /// `new = old` in `project-rename`.
    RENAME_ITEM,
    /// `by keys` in `summarize`.
    BY_CLAUSE,
    /// `expr [asc | desc] [nulls first | nulls last]` in `sort` and `top`.
    SORT_KEY,
    /// `kind = inner` in `join`.
    JOIN_KIND,
    /// `with (prop = value, ...)` in `render`.
    RENDER_PROPS,
    /// `prop = value [, value ...]` inside `with (...)`.
    RENDER_PROP,
    /// `(arg, name = arg, ...)`: call arguments, or the list of `in`.
    ARG_LIST,
    /// A name being defined: a `let` name, an output column name, a property name.
    NAME,

    /// A literal token: number, string, timespan, datetime, `true`, `false`, `null`.
    LITERAL,
    /// `long(null)`, `string(null)`, …: a typed null literal, not a call.
    TYPED_NULL,
    /// A reference to a name: a column, `let`, table, `$left` or `$right`.
    NAME_REF,
    /// `( inner )`. The inner part may be a pipeline.
    PAREN_EXPR,
    /// `f(args)`
    CALL_EXPR,
    /// `base.field`
    FIELD_EXPR,
    /// `base[index]`
    INDEX_EXPR,
    /// `-x` or `not x`
    PREFIX_EXPR,
    /// `lhs op rhs`, including string operators such as `has` and `matches regex`.
    BIN_EXPR,
    /// `x in (a, b)`, `x !in (...)`, `x in~ (...)`, `x !in~ (...)`
    IN_EXPR,
    /// `x between (lo .. hi)` or `x !between (lo .. hi)`
    BETWEEN_EXPR,
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

impl SyntaxKind {
    /// The kind with this raw value, or `None` if there is none.
    pub fn from_raw(raw: u16) -> Option<SyntaxKind> {
        SyntaxKind::ALL.get(usize::from(raw)).copied()
    }

    /// Whether this is a node kind (or `ERROR`, which is both a token and a node kind).
    pub fn is_node(self) -> bool {
        self >= SyntaxKind::ERROR
    }
}

impl From<SyntaxKind> for u16 {
    fn from(kind: SyntaxKind) -> u16 {
        kind as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_is_indexed_by_raw_value() {
        for (i, kind) in SyntaxKind::ALL.iter().enumerate() {
            assert_eq!(*kind as usize, i);
            assert_eq!(SyntaxKind::from_raw(i as u16), Some(*kind));
        }
        assert_eq!(SyntaxKind::from_raw(SyntaxKind::ALL.len() as u16), None);
    }
}
