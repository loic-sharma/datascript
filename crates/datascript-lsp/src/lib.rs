//! The DataScript language server (`ds lsp`, ARCHITECTURE.md § Frontends).
//!
//! It speaks the LSP over stdio with incremental text sync and publishes diagnostics as the user
//! types: each change is checked after a short debounce ([`Config`]), syntax errors show at once
//! when schemas still have to be read, and semantic ones follow when they resolve. Schemas are
//! cached by the [`Session`](datascript_engine::Session); when the client reports a changed data
//! file, the cache is cleared and open documents are checked again.
//!
//! With the `initializationOptions` `{ "syntaxOnly": true }` it reports only lexical and syntax
//! errors and reads no files, for workspaces the user does not trust.
//!
//! The server does not pick an engine: the frontend that starts it passes a session over one.

pub mod convert;
mod document;
mod server;

pub use server::{Backend, Config, serve, serve_stdio, service};
