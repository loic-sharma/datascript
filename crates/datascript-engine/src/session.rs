//! The [`Session`] facade.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use arrow_schema::SchemaRef;
use datascript_compiler::hir::Hir;
use datascript_compiler::{
    Analysis, Program, SchemaLookup, SchemaProvider, SourceError, SourceRef,
};
use datascript_diagnostics::codes::{DS0004, DS9001};
use datascript_diagnostics::{Diagnostic, FileId, SourceFile};

use crate::boundary::catch_panic_async;
use crate::{Clock, Engine, ExecContext, QueryResult};

/// The diagnostics and results of [`Session::run`].
#[derive(Debug, Clone, Default)]
pub struct RunOutput {
    /// Every diagnostic: errors stop the run before anything executes (or at the failing query);
    /// warnings do not.
    pub diagnostics: Vec<Diagnostic>,
    /// One result per query statement, in source order. Empty if there were errors.
    pub results: Vec<QueryResult>,
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
struct CallSchemas(HashMap<SourceRef, Lookup>);

impl SchemaProvider for CallSchemas {
    fn schema_of(&self, source: &SourceRef) -> SchemaLookup {
        match self.0.get(source) {
            Some(Ok(schema)) => SchemaLookup::Ready(schema.clone()),
            Some(Err(error)) => SchemaLookup::Failed(error.clone()),
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

    /// Checks, plans and runs every query in `src`. Warnings from checking are kept. Warns
    /// (DS0004) if the program has no query.
    pub async fn run(&self, src: &SourceFile, file: FileId, ctx: &ExecContext) -> RunOutput {
        catch_panic_async(
            Some(file),
            "running the program",
            self.run_inner(src, file, ctx),
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

    async fn run_inner(&self, src: &SourceFile, file: FileId, ctx: &ExecContext) -> RunOutput {
        let analysis = self.check_inner(src, file, ctx).await;
        if analysis.has_errors() {
            return RunOutput {
                diagnostics: analysis.diagnostics,
                results: Vec::new(),
            };
        }
        let warnings = analysis.diagnostics.clone();
        let program = match datascript_compiler::program_of(analysis, file) {
            Ok(program) => program,
            // These diagnostics already include the warnings.
            Err(diagnostics) => {
                return RunOutput {
                    diagnostics,
                    results: Vec::new(),
                };
            }
        };
        let mut output = RunOutput {
            diagnostics: warnings,
            results: Vec::new(),
        };
        if program.queries.is_empty() {
            // Valid for `check`, but `run` has nothing to show (LANGUAGE.md §2).
            output.diagnostics.push(
                Diagnostic::warning(DS0004, "program has no query to run")
                    .in_file(file)
                    .help("add a query, such as `from 'data.csv' | take 10`"),
            );
        }
        for query in &program.queries {
            let result = match self.engine.execute(query, ctx).await {
                Ok(stream) => stream.collect(ctx.row_limit).await,
                Err(diagnostic) => Err(diagnostic),
            };
            match result {
                Ok(mut result) => {
                    result.render = query.render.clone();
                    output.results.push(result);
                }
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
        datascript_compiler::check(src, file, &CallSchemas(lookups), self.engine.capabilities())
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
