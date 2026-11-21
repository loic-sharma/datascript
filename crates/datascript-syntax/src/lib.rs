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
