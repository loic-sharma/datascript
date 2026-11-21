//! Sources and schema discovery (ARCHITECTURE.md § Semantics, D-022, D-023).
//!
//! The compiler never reads files. It asks a [`SchemaProvider`] for each source's schema. The
//! provider is a synchronous cache: on a miss it answers [`SchemaLookup::Pending`], the binder
//! lists the source in [`Analysis::pending_sources`](crate::Analysis::pending_sources), and the
//! session fetches it and binds again.

use std::fmt;

use arrow_schema::SchemaRef;

/// A source as the program names it. Engines resolve it (relative paths against the base
/// directory of the run).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SourceRef {
    /// A file path, exactly as the string literal's value spells it.
    Path(String),
}

impl fmt::Display for SourceRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SourceRef::Path(path) => write!(f, "'{path}'"),
        }
    }
}

/// Why a source's schema could not be found. It carries no span: the binder builds the DS3xxx
/// diagnostic at every place the source appears (D-023).
///
/// M1-08 adds the other cases (`UnknownFormat`, `NoMatches`, `Conflict`, …).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SourceError {
    /// No file at the path. `nearby` lists file names in the same directory, sorted, for a
    /// did-you-mean.
    NotFound { nearby: Vec<String> },
    /// The file exists but its schema could not be read. `reason` is the engine's message.
    /// M1-08 replaces this with specific cases and DS3008.
    Unreadable { reason: String },
}

/// The answer of a [`SchemaProvider`].
#[derive(Debug, Clone, PartialEq)]
pub enum SchemaLookup {
    /// The schema, already widened to DataScript types (LANGUAGE.md §5.1).
    Ready(SchemaRef),
    /// The source cannot be read.
    Failed(SourceError),
    /// Not known yet. The binder gives the source the error type silently and lists it in
    /// `pending_sources`.
    Pending,
}

/// Answers schema lookups synchronously, from a cache (D-022).
pub trait SchemaProvider {
    /// The schema of `source`, or [`SchemaLookup::Pending`] if it has not been fetched yet.
    fn schema_of(&self, source: &SourceRef) -> SchemaLookup;
}

/// A provider that knows nothing: every source is pending. A first binding pass with it finds
/// every source the program uses.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoSchemas;

impl SchemaProvider for NoSchemas {
    fn schema_of(&self, _source: &SourceRef) -> SchemaLookup {
        SchemaLookup::Pending
    }
}
