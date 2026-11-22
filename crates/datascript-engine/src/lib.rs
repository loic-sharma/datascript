//! Engine abstraction, result model and session facade for DataScript (ARCHITECTURE.md
//! § Engine abstraction).
//!
//! - [`Engine`]: what a backend implements. Its futures are `Send` on native targets and
//!   `?Send` on wasm32 (D-011).
//! - [`Session`]: what frontends call. It holds the engine, a schema cache and a [`Clock`], and
//!   offers `check`, `compile`, `explain` and `run` over a source file, running the
//!   source-resolution loop (D-022) behind a panic boundary ([`catch_panic`]).
//!   [`Session::start`] is the streaming form of `run`: a [`RunStream`] hands out one
//!   [`QueryStream`] per query, whose batches arrive as the engine produces them.
//! - [`ExecContext`]: per-run settings: the clock value, the base directory for relative paths,
//!   an optional sandbox and an optional row limit.
//! - [`table`] and [`writers`]: the ASCII result table, CSV and JSON, shared by the frontends and
//!   the golden tests. All three print values the same way.
//! - [`printer`]: streams a run's results as `ds run` prints them, for the CLI, the REPL and the
//!   language server's Run (D-054).

mod boundary;
pub mod printer;
mod session;
pub mod table;
pub mod writers;

use std::fmt;
use std::path::PathBuf;
use std::pin::Pin;

use arrow_array::RecordBatch;
use arrow_schema::SchemaRef;
use datascript_compiler::ir::RenderSpec;
use datascript_compiler::{Capabilities, SourceError, SourceRef};
use datascript_diagnostics::{Diagnostic, FileId, Span};
use futures::{Stream, StreamExt};

pub use boundary::{catch_panic, catch_panic_async};
pub use session::{RunOutput, RunStream, Session};

/// `Send + Sync` on native targets, nothing on wasm32 (D-011).
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSendSync: Send + Sync {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Send + Sync + ?Sized> MaybeSendSync for T {}

/// `Send + Sync` on native targets, nothing on wasm32 (D-011).
#[cfg(target_arch = "wasm32")]
pub trait MaybeSendSync {}
#[cfg(target_arch = "wasm32")]
impl<T: ?Sized> MaybeSendSync for T {}

/// A stream of result batches. `Send` on native targets.
#[cfg(not(target_arch = "wasm32"))]
pub type BatchStream = Pin<Box<dyn Stream<Item = Result<RecordBatch, Diagnostic>> + Send>>;
/// A stream of result batches. `Send` on native targets.
#[cfg(target_arch = "wasm32")]
pub type BatchStream = Pin<Box<dyn Stream<Item = Result<RecordBatch, Diagnostic>>>>;

/// A query backend.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
pub trait Engine: MaybeSendSync {
    /// The name used in `engine:` headers and messages, such as `datafusion`.
    fn name(&self) -> &str;

    /// What this backend supports. The compiler checks programs against it.
    fn capabilities(&self) -> &Capabilities;

    /// Reads the schema of `source`, widened to DataScript types (LANGUAGE.md §5.1). Relative
    /// paths resolve against `ctx.base_dir`. Errors carry no span; the compiler builds the
    /// diagnostic (D-023).
    async fn schema_of(
        &self,
        source: &SourceRef,
        ctx: &ExecContext,
    ) -> Result<SchemaRef, SourceError>;

    /// Starts running one query. Dropping the stream cancels it.
    async fn execute(
        &self,
        plan: &datascript_compiler::ir::QueryPlan,
        ctx: &ExecContext,
    ) -> Result<ResultStream, Diagnostic>;
}

/// The batches of one query, as they are produced. Dropping it (or its `batches`) cancels the
/// query: that is how Ctrl-C, LSP cancellation and the web UI's Stop button work.
pub struct ResultStream {
    /// The result schema, in canonical Arrow types.
    pub schema: SchemaRef,
    pub batches: BatchStream,
}

impl fmt::Debug for ResultStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResultStream")
            .field("schema", &self.schema)
            .finish_non_exhaustive()
    }
}

impl ResultStream {
    /// Reads every batch, or the first `row_limit` rows. Once the limit is reached and another
    /// row arrives, the stream is dropped, which cancels the rest of the query, and the result
    /// is marked [`truncated`](QueryResult::truncated).
    ///
    /// The result has no `render` spec or spans; [`Session::run`] adds the plan's.
    pub async fn collect(mut self, row_limit: Option<usize>) -> Result<QueryResult, Diagnostic> {
        let mut batches = Vec::new();
        let mut rows = 0;
        let mut truncated = false;
        while let Some(batch) = self.batches.next().await {
            let batch = batch?;
            if let Some(limit) = row_limit {
                let room = limit - rows;
                if batch.num_rows() > room {
                    if room > 0 {
                        batches.push(batch.slice(0, room));
                    }
                    truncated = true;
                    break;
                }
            }
            rows += batch.num_rows();
            batches.push(batch);
        }
        Ok(QueryResult {
            schema: self.schema,
            batches,
            render: None,
            render_span: None,
            statement: None,
            diagnostics: Vec::new(),
            truncated,
        })
    }
}

/// One query of a [`RunStream`], running. Its batches come from the engine as it produces them,
/// behind the panic boundary: a panic while producing one becomes a `DS9001` item and ends the
/// stream. Dropping it (or its `batches`) cancels the query.
///
/// An `Err` item means the query, and so the program, failed: stop reading, and do not start
/// the next query.
pub struct QueryStream {
    /// The result schema, in canonical Arrow types.
    pub schema: SchemaRef,
    pub batches: BatchStream,
    /// What a final `render` asks frontends to draw; `None` without one.
    pub render: Option<RenderSpec>,
    /// The `render` operator of [`QueryStream::render`], for `DS5001`.
    pub render_span: Option<Span>,
    /// The query statement this is the result of.
    pub statement: Span,
}

impl fmt::Debug for QueryStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("QueryStream")
            .field("schema", &self.schema)
            .field("statement", &self.statement)
            .finish_non_exhaustive()
    }
}

impl QueryStream {
    /// Reads every batch, or the first `row_limit` rows, as [`ResultStream::collect`] does, and
    /// keeps the query's `render` spec and spans.
    pub async fn collect(self, row_limit: Option<usize>) -> Result<QueryResult, Diagnostic> {
        let stream = ResultStream {
            schema: self.schema,
            batches: self.batches,
        };
        let mut result = stream.collect(row_limit).await?;
        result.render = self.render;
        result.render_span = self.render_span;
        result.statement = Some(self.statement);
        Ok(result)
    }
}

/// `batches`, with a panic while producing one turned into a `DS9001` item that ends the stream.
pub(crate) fn guard_panics(file: FileId, batches: BatchStream) -> BatchStream {
    Box::pin(
        std::panic::AssertUnwindSafe(batches)
            .catch_unwind()
            .map(move |item| match item {
                Ok(item) => item,
                Err(payload) => Err(boundary::panic_diagnostic(
                    Some(file),
                    "running the program",
                    &*payload,
                )),
            }),
    )
}

/// The complete result of one query.
#[derive(Debug, Clone)]
pub struct QueryResult {
    /// The result schema, in canonical Arrow types.
    pub schema: SchemaRef,
    pub batches: Vec<RecordBatch>,
    /// What a final `render` asks frontends to draw; `None` without one.
    pub render: Option<RenderSpec>,
    /// The `render` operator of [`QueryResult::render`] ([`QueryPlan::render_span`]), for
    /// `DS5001`. [`Session::run`] sets it.
    ///
    /// [`QueryPlan::render_span`]: datascript_compiler::ir::QueryPlan::render_span
    pub render_span: Option<Span>,
    /// The query statement this is the result of ([`QueryPlan::span`]). [`Session::run`] sets
    /// it.
    ///
    /// [`QueryPlan::span`]: datascript_compiler::ir::QueryPlan::span
    pub statement: Option<Span>,
    /// Warnings and notes about running this query. Errors stop the run instead and are in
    /// [`RunOutput::diagnostics`].
    pub diagnostics: Vec<Diagnostic>,
    /// Whether rows were left out because of [`ExecContext::row_limit`]. Frontends say so
    /// (Q-007, decided in M4-01).
    pub truncated: bool,
}

/// The source of `now()` (D-015). Read once per run.
pub trait Clock: MaybeSendSync {
    /// The current time in microseconds since the Unix epoch, UTC.
    fn now_micros(&self) -> i64;
}

/// The system clock. Not on wasm32, where `SystemTime::now()` panics; the WASM runtime injects
/// a clock that reads JavaScript's `Date`.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

#[cfg(not(target_arch = "wasm32"))]
impl Clock for SystemClock {
    fn now_micros(&self) -> i64 {
        let since_epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        i64::try_from(since_epoch.as_micros()).unwrap_or(i64::MAX)
    }
}

/// A clock that always says the same time, for tests.
#[derive(Debug, Clone, Copy)]
pub struct FixedClock(pub i64);

impl Clock for FixedClock {
    fn now_micros(&self) -> i64 {
        self.0
    }
}

/// Settings for one run (ARCHITECTURE.md § Engine abstraction).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecContext {
    /// The value of `now()` for this run (D-015), in microseconds since the Unix epoch.
    pub now_micros: i64,
    /// Relative paths resolve against this directory: the script's directory.
    pub base_dir: PathBuf,
    /// If set, every path must resolve inside this directory (D-016, M4-01).
    pub sandbox: Option<PathBuf>,
    /// If set, each result stops after this many rows and is marked truncated (Q-007).
    pub row_limit: Option<usize>,
}

#[cfg(test)]
mod tests;
