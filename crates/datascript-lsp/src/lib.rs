//! The DataScript language server (`ds lsp`, ARCHITECTURE.md § Frontends).
//!
//! It speaks the LSP over stdio with incremental text sync and publishes diagnostics as the user
//! types: each change is checked after a short debounce ([`Config`]), syntax errors show at once
//! when schemas still have to be read, and semantic ones follow when they resolve. Schemas are
//! cached by the [`Session`](datascript_engine::Session); when the client reports a changed data
//! file, the cache is cleared and open documents are checked again.
//!
//! Completion, hover, signature help, quick-fix code actions, `let` navigation and rename,
//! document symbols and semantic tokens (M3-02, M3-03, D-057, D-060) come from
//! `datascript_compiler::ide`; the server adds file names inside path strings.
//!
//! With the `initializationOptions` `{ "syntaxOnly": true }` it reports only lexical and syntax
//! errors and reads no files (completion offers no columns and no files; navigation, rename,
//! document symbols and semantic tokens work from the parse alone; code actions come only from
//! syntax diagnostics), for workspaces the user does not trust.
//!
//! Run (M3-05, D-054): a "▶ Run" CodeLens over each query statement, and the custom request
//! `datascript/run` ([`RunParams`] → [`RunResult`]) that runs the server's text of an open
//! document and returns what `ds run` prints for it. Its runtime error is published with the
//! document's diagnostics until the next edit. A syntax-only server runs nothing.
//!
//! The server does not pick an engine: the frontend that starts it passes a session over one.

pub mod convert;
mod document;
mod ide;
mod run;
mod server;

pub use run::{RUN_FILE_COMMAND, RUN_METHOD, RUN_STATEMENT_COMMAND, RunParams, RunResult};
pub use server::{Backend, Config, serve, serve_stdio, service};
