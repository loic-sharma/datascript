//! Engine abstraction, result model and session facade for DataScript (ARCHITECTURE.md
//! § Engine abstraction).
//!
//! - [`Engine`]: what a backend implements. Its futures are `Send` on native targets and
//!   `?Send` on wasm32 (D-011).
//! - [`Session`]: what frontends call. It holds the engine, a schema cache and a [`Clock`], and
//!   offers `check`, `explain` and `run` over a source file, running the source-resolution loop
//!   (D-022).
//! - [`ExecContext`]: per-run settings: the clock value, the base directory for relative paths,
//!   an optional sandbox and an optional row limit.
//! - [`table`]: the ASCII result table shared by the CLI and the golden tests.

pub mod table;

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use arrow_array::RecordBatch;
use arrow_schema::SchemaRef;
use datascript_compiler::{
    Analysis, Capabilities, Program, SchemaLookup, SchemaProvider, SourceError, SourceRef,
};
use datascript_diagnostics::codes::DS9001;
use datascript_diagnostics::{Diagnostic, FileId, SourceFile};
use futures::{Stream, StreamExt};

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

/// The batches of one query, as they are produced.
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
    /// Reads every batch.
    pub async fn collect(mut self) -> Result<QueryResult, Diagnostic> {
        let mut batches = Vec::new();
        while let Some(batch) = self.batches.next().await {
            batches.push(batch?);
        }
        Ok(QueryResult {
            schema: self.schema,
            batches,
        })
    }
}

/// The complete result of one query. (`render` joins it in M1-10.)
#[derive(Debug, Clone)]
pub struct QueryResult {
    pub schema: SchemaRef,
    pub batches: Vec<RecordBatch>,
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
    /// If set, results stop after this many rows (Q-007).
    pub row_limit: Option<usize>,
}

/// The diagnostics and results of [`Session::run`].
#[derive(Debug, Clone, Default)]
pub struct RunOutput {
    /// Every diagnostic: errors stop the run before anything executes; warnings do not.
    pub diagnostics: Vec<Diagnostic>,
    /// One result per query statement, in source order. Empty if there were errors.
    pub results: Vec<QueryResult>,
}

/// The facade frontends call. It does not hold a document, so one session can serve many
/// documents and concurrent requests (the LSP, `ds ui`). On native targets it is `Send + Sync`.
pub struct Session {
    engine: Arc<dyn Engine>,
    clock: Arc<dyn Clock>,
    /// Schemas fetched so far, by base directory and source. Only successful lookups are cached;
    /// invalidation when files change is M1-07's.
    schemas: Mutex<HashMap<(PathBuf, SourceRef), SchemaRef>>,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("engine", &self.engine.name())
            .finish_non_exhaustive()
    }
}

/// Lookups for one call: the session's cache, plus this call's failures.
struct CallSchemas<'a> {
    cache: HashMap<SourceRef, SchemaRef>,
    failures: &'a HashMap<SourceRef, SourceError>,
}

impl SchemaProvider for CallSchemas<'_> {
    fn schema_of(&self, source: &SourceRef) -> SchemaLookup {
        if let Some(schema) = self.cache.get(source) {
            return SchemaLookup::Ready(schema.clone());
        }
        match self.failures.get(source) {
            Some(error) => SchemaLookup::Failed(error.clone()),
            None => SchemaLookup::Pending,
        }
    }
}

impl Session {
    /// A session over `engine`, reading `now()` from `clock`.
    pub fn new(engine: Arc<dyn Engine>, clock: Arc<dyn Clock>) -> Self {
        Session {
            engine,
            clock,
            schemas: Mutex::new(HashMap::new()),
        }
    }

    /// The engine.
    pub fn engine(&self) -> &dyn Engine {
        self.engine.as_ref()
    }

    /// A context for one run: reads the clock once (D-015).
    pub fn exec_context(&self, base_dir: &Path) -> ExecContext {
        ExecContext {
            now_micros: self.clock.now_micros(),
            base_dir: base_dir.to_path_buf(),
            sandbox: None,
            row_limit: None,
        }
    }

    /// Checks `src` (file `file`), fetching the schemas of the sources it uses.
    pub async fn check(&self, src: &SourceFile, file: FileId, ctx: &ExecContext) -> Analysis {
        let mut failures = HashMap::new();
        // Pass 1 finds every source, pass 2 binds with them all; sources depend only on
        // constants, so a third pass with pending sources is a bug (D-022).
        for _pass in 0..2 {
            let analysis = self.check_once(src, file, ctx, &failures);
            if analysis.pending_sources.is_empty() {
                return analysis;
            }
            for source in &analysis.pending_sources {
                match self.engine.schema_of(source, ctx).await {
                    Ok(schema) => {
                        self.lock_schemas()
                            .insert((ctx.base_dir.clone(), source.clone()), schema);
                    }
                    Err(error) => {
                        failures.insert(source.clone(), error);
                    }
                }
            }
        }
        let mut analysis = self.check_once(src, file, ctx, &failures);
        if let Some(source) = analysis.pending_sources.first() {
            analysis.diagnostics.push(
                Diagnostic::error(
                    DS9001,
                    format!(
                        "internal error: the schema of {source} is still unknown after two passes"
                    ),
                )
                .in_file(file)
                .note("this is a bug in DataScript; please report it"),
            );
        }
        analysis
    }

    /// Checks and plans `src`. Returns the program, or every diagnostic if there are errors.
    pub async fn compile(
        &self,
        src: &SourceFile,
        file: FileId,
        ctx: &ExecContext,
    ) -> Result<Program, Vec<Diagnostic>> {
        let analysis = self.check(src, file, ctx).await;
        datascript_compiler::program_of(analysis, file)
    }

    /// The `explain` printout of `src`, or its diagnostics.
    pub async fn explain(
        &self,
        src: &SourceFile,
        file: FileId,
        ctx: &ExecContext,
    ) -> Result<String, Vec<Diagnostic>> {
        Ok(self.compile(src, file, ctx).await?.explain())
    }

    /// Checks, plans and runs every query in `src`.
    pub async fn run(&self, src: &SourceFile, file: FileId, ctx: &ExecContext) -> RunOutput {
        let program = match self.compile(src, file, ctx).await {
            Ok(program) => program,
            Err(diagnostics) => {
                return RunOutput {
                    diagnostics,
                    results: Vec::new(),
                };
            }
        };
        let mut output = RunOutput::default();
        for query in &program.queries {
            let result = match self.engine.execute(query, ctx).await {
                Ok(stream) => stream.collect().await,
                Err(diagnostic) => Err(diagnostic),
            };
            match result {
                Ok(result) => output.results.push(result),
                Err(diagnostic) => {
                    output.diagnostics.push(diagnostic);
                    output.results.clear();
                    break;
                }
            }
        }
        output
    }

    fn check_once(
        &self,
        src: &SourceFile,
        file: FileId,
        ctx: &ExecContext,
        failures: &HashMap<SourceRef, SourceError>,
    ) -> Analysis {
        let cache = self
            .lock_schemas()
            .iter()
            .filter(|((dir, _), _)| *dir == ctx.base_dir)
            .map(|((_, source), schema)| (source.clone(), schema.clone()))
            .collect();
        let schemas = CallSchemas { cache, failures };
        datascript_compiler::check(src, file, &schemas, self.engine.capabilities())
    }

    fn lock_schemas(&self) -> std::sync::MutexGuard<'_, HashMap<(PathBuf, SourceRef), SchemaRef>> {
        // A panic while holding the lock cannot leave the map half-updated (single inserts).
        self.schemas.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests;
