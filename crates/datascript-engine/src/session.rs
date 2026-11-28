//! The [`Session`] facade.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use arrow_schema::SchemaRef;
use datascript_compiler::hir::Hir;
use datascript_compiler::ir::QueryPlan;
use datascript_compiler::{
    Analysis, Program, SchemaLookup, SchemaProvider, SourceError, SourceRef,
};
use datascript_diagnostics::codes::{DS0004, DS9001};
use datascript_diagnostics::{Diagnostic, FileId, SourceFile};

use crate::boundary::catch_panic_async;
use crate::{Clock, Engine, ExecContext, QueryResult, QueryStream, guard_panics};

/// The diagnostics and results of [`Session::run`].
#[derive(Debug, Clone, Default)]
pub struct RunOutput {
    /// Every diagnostic: errors stop the run before anything executes (or at the failing query);
    /// warnings do not.
    pub diagnostics: Vec<Diagnostic>,
    /// One result per query statement, in source order. Empty if there were errors.
    pub results: Vec<QueryResult>,
}

/// A program checked and planned, run one query at a time ([`Session::start`]).
///
/// It owns what it needs, so it can move to another task or thread (on native targets it is
/// `Send`). Dropping it, or a [`QueryStream`] it handed out, cancels the running query.
pub struct RunStream {
    engine: Arc<dyn Engine>,
    ctx: ExecContext,
    file: FileId,
    /// Diagnostics from checking: warnings and `DS0004`, or the errors that stop the run (then
    /// there are no queries).
    pub diagnostics: Vec<Diagnostic>,
    queries: Vec<QueryPlan>,
    next: usize,
    failed: bool,
}

impl fmt::Debug for RunStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RunStream")
            .field("engine", &self.engine.name())
            .field("diagnostics", &self.diagnostics)
            .field("queries", &self.queries.len())
            .field("next", &self.next)
            .finish_non_exhaustive()
    }
}

impl RunStream {
    /// Whether checking found errors. Then nothing runs.
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(Diagnostic::is_error)
    }

    /// The plans of the queries that will run, in source order. Frontends read their spans and
    /// `render` specs before anything runs.
    pub fn queries(&self) -> &[QueryPlan] {
        &self.queries
    }

    /// Keeps only the query statement at `index` (0-based, in source order, counting query
    /// statements only). Past the last query, nothing runs. Call it before
    /// [`next_query`](Self::next_query).
    pub fn select(&mut self, index: usize) {
        let kept = self.queries.get(index).cloned();
        self.queries = kept.into_iter().collect();
    }

    /// Starts the next query: `None` when every query has run, or after one failed to start.
    /// Read each [`QueryStream`] to its end (or drop it) before starting the next; a batch error
    /// means the program failed, and the caller stops there.
    pub async fn next_query(&mut self) -> Option<Result<QueryStream, Diagnostic>> {
        if self.failed {
            return None;
        }
        let index = self.next;
        self.next += 1;
        let query = self.queries.get(index)?;
        let started = catch_panic_async(
            Some(self.file),
            "running the program",
            self.engine.execute(query, &self.ctx),
        )
        .await
        .and_then(|started| started);
        match started {
            Ok(stream) => Some(Ok(QueryStream {
                schema: stream.schema,
                batches: guard_panics(self.file, stream.batches),
                render: query.render.clone(),
                render_span: query.render_span,
                statement: query.span,
            })),
            Err(diagnostic) => {
                self.failed = true;
                Some(Err(diagnostic))
            }
        }
    }
}

/// A schema lookup as the engine answered it.
type Lookup = Result<SchemaRef, SourceError>;

/// The schema cache key. Relative paths depend on the base directory, and whether a path is
/// allowed depends on the sandbox, so both are part of the key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    base_dir: PathBuf,
    sandbox: Option<PathBuf>,
    source: SourceRef,
}

impl CacheKey {
    fn new(ctx: &ExecContext, source: &SourceRef) -> Self {
        CacheKey {
            base_dir: ctx.base_dir.clone(),
            sandbox: ctx.sandbox.clone(),
            source: source.clone(),
        }
    }

    fn matches(&self, ctx: &ExecContext) -> bool {
        self.base_dir == ctx.base_dir && self.sandbox == ctx.sandbox
    }
}

/// The facade frontends call. It does not hold a document, so one session can serve many
/// documents and concurrent requests (the LSP, `ds ui`). On native targets it is `Send + Sync`.
///
/// Schema lookups are cached, failures included, until [`invalidate_schemas`] or
/// [`invalidate_source`] is called. Frontends that outlive one run (the REPL, the LSP, `ds ui`)
/// invalidate when files may have changed.
///
/// Every method runs behind the panic boundary: a panic becomes `DS9001`.
///
/// [`invalidate_schemas`]: Session::invalidate_schemas
/// [`invalidate_source`]: Session::invalidate_source
pub struct Session {
    engine: Arc<dyn Engine>,
    clock: Arc<dyn Clock>,
    schemas: Mutex<HashMap<CacheKey, Lookup>>,
    /// Binding passes so far, for tests of the resolution loop.
    #[cfg(test)]
    passes: std::sync::atomic::AtomicUsize,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("engine", &self.engine.name())
            .finish_non_exhaustive()
    }
}

/// The lookups one binding pass sees.
struct CallSchemas {
    lookups: HashMap<SourceRef, Lookup>,
    sandboxed: bool,
}

impl SchemaProvider for CallSchemas {
    fn schema_of(&self, source: &SourceRef) -> SchemaLookup {
        match self.lookups.get(source) {
            Some(Ok(schema)) => SchemaLookup::Ready(schema.clone()),
            Some(Err(error)) => SchemaLookup::Failed(error.clone()),
            None => SchemaLookup::Pending,
        }
    }

    fn is_sandboxed(&self) -> bool {
        self.sandboxed
    }
}

impl Session {
    /// A session over `engine`, reading `now()` from `clock`.
    pub fn new(engine: Arc<dyn Engine>, clock: Arc<dyn Clock>) -> Self {
        Session {
            engine,
            clock,
            schemas: Mutex::new(HashMap::new()),
            #[cfg(test)]
            passes: std::sync::atomic::AtomicUsize::new(0),
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

    /// Forgets every cached schema and failed lookup.
    pub fn invalidate_schemas(&self) {
        self.lock_schemas().clear();
    }

    /// Forgets the cached schema or failed lookup of `source`, for every base directory.
    pub fn invalidate_source(&self, source: &SourceRef) {
        self.lock_schemas().retain(|key, _| key.source != *source);
    }

    /// Checks `src` (file `file`), fetching the schemas of the sources it uses.
    pub async fn check(&self, src: &SourceFile, file: FileId, ctx: &ExecContext) -> Analysis {
        match catch_panic_async(
            Some(file),
            "checking the program",
            self.check_inner(src, file, ctx),
        )
        .await
        {
            Ok(analysis) => analysis,
            Err(diagnostic) => Analysis {
                hir: Hir::default(),
                diagnostics: vec![diagnostic],
                stage_schemas: Vec::new(),
                pending_sources: Vec::new(),
                deviations: self
                    .engine
                    .capabilities()
                    .deviations
                    .iter()
                    .cloned()
                    .collect(),
            },
        }
    }

    /// Checks and plans `src`. Returns the program, or every diagnostic if there are errors.
    pub async fn compile(
        &self,
        src: &SourceFile,
        file: FileId,
        ctx: &ExecContext,
    ) -> Result<Program, Vec<Diagnostic>> {
        catch_panic_async(
            Some(file),
            "compiling the program",
            self.compile_inner(src, file, ctx),
        )
        .await
        .unwrap_or_else(|diagnostic| Err(vec![diagnostic]))
    }

    /// The `explain` printout of `src`, or its diagnostics.
    pub async fn explain(
        &self,
        src: &SourceFile,
        file: FileId,
        ctx: &ExecContext,
    ) -> Result<String, Vec<Diagnostic>> {
        let explain = async { Ok(self.compile_inner(src, file, ctx).await?.explain()) };
        catch_panic_async(Some(file), "explaining the program", explain)
            .await
            .unwrap_or_else(|diagnostic| Err(vec![diagnostic]))
    }

    /// Checks and plans `src`, ready to run its queries one at a time as streams: the
    /// streaming form of [`run`](Session::run). The diagnostics are the ones `run` would give
    /// before running anything, `DS0004` included. A panic while checking becomes `DS9001`.
    pub async fn start(&self, src: &SourceFile, file: FileId, ctx: &ExecContext) -> RunStream {
        catch_panic_async(
            Some(file),
            "running the program",
            self.start_inner(src, file, ctx),
        )
        .await
        .unwrap_or_else(|diagnostic| self.run_stream(file, ctx, vec![diagnostic], Vec::new()))
    }

    /// Checks, plans and runs every query in `src`. Warnings from checking are kept. Warns
    /// (DS0004) if the program has no query.
    pub async fn run(&self, src: &SourceFile, file: FileId, ctx: &ExecContext) -> RunOutput {
        self.run_selected(src, file, ctx, None).await
    }

    /// Like [`run`](Session::run), but runs only the query statement at `index` (0-based, in
    /// source order, counting query statements only). The whole program is still checked, so an
    /// error anywhere stops the run. With `index` past the last query, nothing runs and
    /// `results` is empty; frontends check the index against [`Program::queries`] first.
    pub async fn run_statement(
        &self,
        src: &SourceFile,
        file: FileId,
        ctx: &ExecContext,
        index: usize,
    ) -> RunOutput {
        self.run_selected(src, file, ctx, Some(index)).await
    }

    async fn run_selected(
        &self,
        src: &SourceFile,
        file: FileId,
        ctx: &ExecContext,
        index: Option<usize>,
    ) -> RunOutput {
        catch_panic_async(
            Some(file),
            "running the program",
            self.run_inner(src, file, ctx, index),
        )
        .await
        .unwrap_or_else(|diagnostic| RunOutput {
            diagnostics: vec![diagnostic],
            results: Vec::new(),
        })
    }

    async fn check_inner(&self, src: &SourceFile, file: FileId, ctx: &ExecContext) -> Analysis {
        // This call's lookups win over the shared cache, so a concurrent invalidation cannot
        // make a source pending again between passes.
        let mut fetched: HashMap<SourceRef, Lookup> = HashMap::new();
        // Pass 1 finds every source, pass 2 binds with them all; sources depend only on
        // constants, so a third pass with pending sources is a bug (D-022).
        for _pass in 0..2 {
            let analysis = self.check_once(src, file, ctx, &fetched);
            if analysis.pending_sources.is_empty() {
                return analysis;
            }
            for source in analysis.pending_sources {
                let lookup = self.engine.schema_of(&source, ctx).await;
                self.lock_schemas()
                    .insert(CacheKey::new(ctx, &source), lookup.clone());
                fetched.insert(source, lookup);
            }
        }
        let mut analysis = self.check_once(src, file, ctx, &fetched);
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

    async fn compile_inner(
        &self,
        src: &SourceFile,
        file: FileId,
        ctx: &ExecContext,
    ) -> Result<Program, Vec<Diagnostic>> {
        let analysis = self.check_inner(src, file, ctx).await;
        datascript_compiler::program_of(analysis, file)
    }

    fn run_stream(
        &self,
        file: FileId,
        ctx: &ExecContext,
        diagnostics: Vec<Diagnostic>,
        queries: Vec<QueryPlan>,
    ) -> RunStream {
        RunStream {
            engine: self.engine.clone(),
            ctx: ctx.clone(),
            file,
            diagnostics,
            queries,
            next: 0,
            failed: false,
        }
    }

    async fn start_inner(&self, src: &SourceFile, file: FileId, ctx: &ExecContext) -> RunStream {
        let analysis = self.check_inner(src, file, ctx).await;
        if analysis.has_errors() {
            return self.run_stream(file, ctx, analysis.diagnostics, Vec::new());
        }
        let mut diagnostics = analysis.diagnostics.clone();
        let program = match datascript_compiler::program_of(analysis, file) {
            Ok(program) => program,
            // These diagnostics already include the warnings.
            Err(diagnostics) => return self.run_stream(file, ctx, diagnostics, Vec::new()),
        };
        if program.queries.is_empty() {
            // Valid for `check`, but `run` has nothing to show (LANGUAGE.md §2).
            diagnostics.push(
                Diagnostic::warning(DS0004, "program has no query to run")
                    .in_file(file)
                    .help("add a query, such as `from 'data.csv' | take 10`"),
            );
        }
        self.run_stream(file, ctx, diagnostics, program.queries)
    }

    async fn run_inner(
        &self,
        src: &SourceFile,
        file: FileId,
        ctx: &ExecContext,
        index: Option<usize>,
    ) -> RunOutput {
        let mut run = self.start_inner(src, file, ctx).await;
        if let Some(index) = index {
            run.select(index);
        }
        let mut output = RunOutput {
            diagnostics: std::mem::take(&mut run.diagnostics),
            results: Vec::new(),
        };
        while let Some(started) = run.next_query().await {
            let result = match started {
                Ok(stream) => stream.collect(ctx.row_limit).await,
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
        fetched: &HashMap<SourceRef, Lookup>,
    ) -> Analysis {
        #[cfg(test)]
        self.passes
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut lookups: HashMap<SourceRef, Lookup> = self
            .lock_schemas()
            .iter()
            .filter(|(key, _)| key.matches(ctx))
            .map(|(key, lookup)| (key.source.clone(), lookup.clone()))
            .collect();
        lookups.extend(fetched.iter().map(|(s, l)| (s.clone(), l.clone())));
        datascript_compiler::check(
            src,
            file,
            &CallSchemas {
                lookups,
                sandboxed: ctx.sandbox.is_some(),
            },
            self.engine.capabilities(),
        )
    }

    #[cfg(test)]
    pub(crate) fn passes(&self) -> usize {
        self.passes.load(std::sync::atomic::Ordering::SeqCst)
    }

    #[cfg(test)]
    pub(crate) fn reset_passes(&self) {
        self.passes.store(0, std::sync::atomic::Ordering::SeqCst);
    }

    fn lock_schemas(&self) -> MutexGuard<'_, HashMap<CacheKey, Lookup>> {
        // A panic while holding the lock cannot leave the map half-updated: every update is a
        // single insert, retain or clear.
        self.schemas.lock().unwrap_or_else(|e| e.into_inner())
    }
}
