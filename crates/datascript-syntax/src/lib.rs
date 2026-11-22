//! Lexer, parser, lossless syntax tree and typed AST for DataScript.
//!
//! - [`lex`] turns source text into [`Token`]s, trivia included, plus lexical diagnostics.
//! - [`parse`] builds a lossless rowan tree ([`SyntaxNode`]) with error recovery, plus syntax
//!   diagnostics. [`Parse::program`] gives the typed view in [`ast`].
//! - [`SyntaxKind`] names every kind of token and node.
//! - [`literal`] reads the values of literal tokens.
//!
//! This crate has no dependency on DataFusion, tokio or the filesystem and builds for
//! `wasm32-unknown-unknown`.

pub mod ast;
mod lexer;
pub mod literal;
mod parse;
mod parser;
mod syntax_kind;
mod syntax_tree;

pub use lexer::{Lexed, Token, lex};
pub use parse::{Parse, parse};
pub use syntax_kind::SyntaxKind;
pub use syntax_tree::{DataScriptLanguage, SyntaxElement, SyntaxNode, SyntaxToken, debug_tree};

/// The names of the operators the parser knows, in alphabetical order (for completion and
/// did-you-mean).
pub const OPERATOR_NAMES: &[&str] = parser::OPERATORS;

/// The reader functions (LANGUAGE.md §3.1). They return a table, so a reader call at the head
/// of a pipeline or as a `let` value needs `from` (DS0150). The function catalog lives above
/// this crate; a test in `datascript-compiler` checks that the two agree.
pub const READERS: &[&str] = &["read_csv", "read_json", "read_parquet"];

/// `name` as it would be written in a query: bare when it is a plain identifier that is not a
/// reserved word (`month`), quoted otherwise (`['order id']`, `['let']`).
pub fn display_name(name: &str) -> String {
    let mut chars = name.chars();
    let plain = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && SyntaxKind::from_reserved_word(name).is_none();
    if plain {
        name.to_string()
    } else {
        lexer::quote_name(name)
    }
}

#[cfg(test)]
mod display_name_tests {
    use super::display_name;

    #[test]
    fn quotes_only_when_needed() {
        assert_eq!(display_name("month"), "month");
        assert_eq!(display_name("_a1"), "_a1");
        assert_eq!(display_name("order id"), "['order id']");
        assert_eq!(display_name("let"), "['let']");
        assert_eq!(display_name("1a"), "['1a']");
        assert_eq!(display_name(""), "['']");
    }
}
