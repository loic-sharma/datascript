//! Lexer, parser, lossless syntax tree and typed AST for DataScript.
//!
//! - [`lex`] turns source text into [`Token`]s, trivia included, plus lexical diagnostics.
//! - [`SyntaxKind`] names every kind of token (and, from M1-02, node).
//! - [`literal`] reads the values of literal tokens.
//!
//! This crate has no dependency on DataFusion, tokio or the filesystem and builds for
//! `wasm32-unknown-unknown`.

mod lexer;
pub mod literal;
mod syntax_kind;

pub use lexer::{Lexed, Token, lex};
pub use syntax_kind::SyntaxKind;
