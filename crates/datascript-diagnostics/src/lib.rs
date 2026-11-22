//! Diagnostic model and renderers shared by every DataScript frontend.
//!
//! - [`Diagnostic`] is the one model every stage produces; see DIAGNOSTICS.md for style rules.
//! - [`Diagnostics`] collects them, then sorts, de-duplicates and caps them.
//! - [`Renderer`] prints them for terminals ([`Renderer::terminal`]) and golden tests
//!   ([`Renderer::plain`]); [`JsonReport`] is the JSON form for JavaScript consumers.
//! - [`SourceMap`] holds the source text and converts between bytes, characters and UTF-16.
//! - [`did_you_mean`] finds close names for "unknown name" errors.
//! - [`codes`] is the code registry.
//!
//! This crate has no dependency on DataFusion, tokio or the filesystem and builds for
//! `wasm32-unknown-unknown`.

pub mod codes;
mod collection;
mod diagnostic;
pub mod json;
mod render;
mod source;
mod span;
mod suggest;

pub use collection::{DiagnosticSink, Diagnostics, MAX_ERRORS_PER_FILE};
pub use diagnostic::{
    Applicability, Code, Diagnostic, Help, InvalidCode, Label, OverlappingEdits, Severity,
    Suggestion, TextEdit,
};
pub use json::{JsonReport, render_json};
pub use render::{Renderer, text_width};
pub use source::{LineCol, SourceFile, SourceMap};
pub use span::{FileId, Span};
pub use suggest::{Suggestions, did_you_mean};
