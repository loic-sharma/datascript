//! Session logic, tested against an in-memory engine.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use arrow_array::{ArrayRef, Float64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use datascript_compiler::capabilities::FunctionSupport;
use datascript_compiler::ir::QueryPlan;
use datascript_diagnostics::codes::{DS0004, DS1001, DS1007, DS3001, DS4001, DS4002, DS9001};
use datascript_diagnostics::{FileId, Severity, SourceFile};
use futures::executor::block_on;

use super::*;

/// An in-memory engine. Each known path has the columns `amount: real` and `name: string`.
/// Every query returns `batches` batches of `rows_per_batch` rows, built from the plan's output
/// schema.
struct FakeEngine {
    caps: Capabilities,
    /// Paths that exist. A `Mutex` so a test can create a file between checks.
    tables: Mutex<BTreeSet<String>>,
    /// How often each source was looked up.
    lookups: Mutex<HashMap<SourceRef, usize>>,
    executions: AtomicUsize,
    /// The `now_micros` of the last `execute` call.
    last_now: AtomicI64,
    batches: usize,
    rows_per_batch: usize,
    /// Batches handed out by the current result stream.
    polled: Arc<AtomicUsize>,
    /// Set when the current result stream is dropped.
    stream_dropped: Arc<AtomicBool>,
    /// Panic where asked to, to test the panic boundary.
    panic_in: Option<PanicIn>,
    /// Fail `execute` with DS4002.
    fail_execute: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PanicIn {
    SchemaOf,
    Execute,
    Stream,
}

impl FakeEngine {
    fn new() -> Self {
        FakeEngine {
            caps: Capabilities::full(),
            tables: Mutex::new(["orders.csv".to_string(), "customers.csv".to_string()].into()),
            lookups: Mutex::new(HashMap::new()),
            executions: AtomicUsize::new(0),
            last_now: AtomicI64::new(0),
            batches: 1,
            rows_per_batch: 1,
            polled: Arc::new(AtomicUsize::new(0)),
            stream_dropped: Arc::new(AtomicBool::new(false)),
            panic_in: None,
            fail_execute: false,
        }
    }

    fn schema() -> SchemaRef {
        Arc::new(Schema::new(vec![
            Field::new("amount", DataType::Float64, true),
            Field::new("name", DataType::Utf8, true),
        ]))
    }

    fn lookups_of(&self, path: &str) -> usize {
        let lookups = self.lookups.lock().unwrap();
        lookups
            .get(&SourceRef::Path(path.into()))
            .copied()
            .unwrap_or(0)
    }

    fn total_lookups(&self) -> usize {
        self.lookups.lock().unwrap().values().sum()
    }

    /// One batch of `rows` rows in `schema`, whose columns are all `real` or `string`.
    fn batch(schema: &SchemaRef, rows: usize) -> RecordBatch {
        let columns = schema
            .fields()
            .iter()
            .map(|field| -> ArrayRef {
                match field.data_type() {
                    DataType::Float64 => Arc::new(Float64Array::from(vec![150.0; rows])),
                    _ => Arc::new(StringArray::from(vec!["x"; rows])),
                }
            })
            .collect();
        RecordBatch::try_new(schema.clone(), columns).unwrap()
    }
}

#[async_trait::async_trait]
impl Engine for FakeEngine {
    fn name(&self) -> &str {
        "fake"
    }

    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }

    async fn schema_of(
        &self,
        source: &SourceRef,
        _ctx: &ExecContext,
    ) -> Result<SchemaRef, SourceError> {
        *self
            .lookups
            .lock()
            .unwrap()
            .entry(source.clone())
            .or_default() += 1;
        if self.panic_in == Some(PanicIn::SchemaOf) {
            panic!("schema_of exploded");
        }
        let tables = self.tables.lock().unwrap();
        match source {
            SourceRef::Path(p) if tables.contains(p.as_str()) => Ok(Self::schema()),
            _ => Err(SourceError::NotFound {
                nearby: tables.iter().cloned().collect(),
            }),
        }
    }

    async fn execute(
        &self,
        plan: &QueryPlan,
        ctx: &ExecContext,
    ) -> Result<ResultStream, Diagnostic> {
        self.executions.fetch_add(1, Ordering::SeqCst);
        self.last_now.store(ctx.now_micros, Ordering::SeqCst);
        if self.panic_in == Some(PanicIn::Execute) {
            panic!("execute exploded");
        }
        if self.fail_execute {
            return Err(Diagnostic::error(DS4002, "the fake engine failed"));
        }
        let batch = Self::batch(&plan.output_schema, self.rows_per_batch);
        let polled = self.polled.clone();
        polled.store(0, Ordering::SeqCst);
        self.stream_dropped.store(false, Ordering::SeqCst);
        let guard = DropFlag(self.stream_dropped.clone());
        let panic_in_stream = self.panic_in == Some(PanicIn::Stream);
        let batches = futures::stream::iter(0..self.batches).map(move |i| {
            let _keep_alive = &guard;
            polled.fetch_add(1, Ordering::SeqCst);
            if panic_in_stream && i == 1 {
                panic!("stream exploded");
            }
            Ok(batch.clone())
        });
        Ok(ResultStream {
            schema: plan.output_schema.clone(),
            batches: Box::pin(batches),
        })
    }
}

/// Sets its flag when dropped.
struct DropFlag(Arc<AtomicBool>);

impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// A clock that counts how often it is read.
struct CountingClock {
    now: i64,
    reads: AtomicUsize,
}

impl Clock for CountingClock {
    fn now_micros(&self) -> i64 {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.now
    }
}

fn session_with(engine: FakeEngine) -> (Session, Arc<FakeEngine>) {
    let engine = Arc::new(engine);
    (
        Session::new(engine.clone(), Arc::new(FixedClock(42))),
        engine,
    )
}

fn session() -> (Session, Arc<FakeEngine>) {
    session_with(FakeEngine::new())
}

fn source(text: &str) -> SourceFile {
    SourceFile::new("query", text)
}

fn codes(diagnostics: &[Diagnostic]) -> Vec<datascript_diagnostics::Code> {
    diagnostics.iter().map(|d| d.code).collect()
}

fn ctx(session: &Session) -> ExecContext {
    session.exec_context(Path::new("."))
}

#[test]
fn native_session_and_its_futures_are_send() {
    fn assert_send<T: Send + Sync>() {}
    fn assert_send_future<F: std::future::Future + Send>(_: F) {}
    assert_send::<Session>();
    assert_send::<ExecContext>();
    assert_send::<QueryResult>();
    fn assert_send_only<T: Send>() {}
    assert_send_only::<ResultStream>();
    let (session, _) = session();
    let ctx = ctx(&session);
    let src = source("from 'orders.csv'");
    assert_send_future(session.check(&src, FileId(0), &ctx));
    assert_send_future(session.compile(&src, FileId(0), &ctx));
    assert_send_future(session.explain(&src, FileId(0), &ctx));
    assert_send_future(session.run(&src, FileId(0), &ctx));

    // One session serves several threads.
    let session = Arc::new(session);
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let session = session.clone();
            std::thread::spawn(move || {
                let ctx = session.exec_context(Path::new("."));
                block_on(session.run(&source("from 'orders.csv'"), FileId(0), &ctx))
            })
        })
        .collect();
    for handle in handles {
        let output = handle.join().unwrap();
        assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    }
}

#[test]
fn the_clock_is_read_once_per_context_and_reaches_the_engine() {
    let engine = Arc::new(FakeEngine::new());
    let clock = Arc::new(CountingClock {
        now: 1_700_000_000_000_000,
        reads: AtomicUsize::new(0),
    });
    let session = Session::new(engine.clone(), clock.clone());
    let ctx = session.exec_context(Path::new("x"));
    assert_eq!(ctx.now_micros, 1_700_000_000_000_000);
    assert_eq!(clock.reads.load(Ordering::SeqCst), 1);

    // Every statement of the run sees the context's time; the session does not read the clock
    // again (D-015).
    let src = source("from 'orders.csv' | take 1; from 'orders.csv' | take 2");
    let output = block_on(session.run(&src, FileId(0), &ctx));
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    assert_eq!(engine.executions.load(Ordering::SeqCst), 2);
    assert_eq!(
        engine.last_now.load(Ordering::SeqCst),
        1_700_000_000_000_000
    );
    assert_eq!(clock.reads.load(Ordering::SeqCst), 1);
}

#[test]
fn sources_are_fetched_once_each_then_bound_again() {
    let (session, engine) = session();
    let ctx = ctx(&session);
    // `orders.csv` appears twice, `customers.csv` once.
    let src = source(
        "from 'orders.csv' | join kind=inner 'customers.csv' on name;\n\
         from 'orders.csv' | where amount > 100 | take 1",
    );
    let analysis = block_on(session.check(&src, FileId(0), &ctx));
    // The join renames the right side's `amount` (info DS1007); nothing is an error.
    assert_eq!(codes(&analysis.diagnostics), [DS1007]);
    assert!(analysis.pending_sources.is_empty());
    assert!(analysis.hir.queries().all(|q| q.pipeline.columns.is_some()));
    assert_eq!(engine.lookups_of("orders.csv"), 1);
    assert_eq!(engine.lookups_of("customers.csv"), 1);
    assert_eq!(
        session.passes(),
        2,
        "pass 1 finds the sources, pass 2 binds them"
    );
}

#[test]
fn a_program_without_sources_needs_one_pass() {
    let (session, engine) = session();
    let analysis = block_on(session.check(&source("let x = 1"), FileId(0), &ctx(&session)));
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    assert_eq!(session.passes(), 1);
    assert_eq!(engine.total_lookups(), 0);
}

#[test]
fn cached_schemas_are_not_fetched_again() {
    let (session, engine) = session();
    let ctx = ctx(&session);
    let src = source("from 'orders.csv' | take 1");
    block_on(session.check(&src, FileId(0), &ctx));
    session.reset_passes();
    let output = block_on(session.run(&src, FileId(0), &ctx));
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    assert_eq!(engine.lookups_of("orders.csv"), 1);
    assert_eq!(session.passes(), 1, "a cache hit needs no second pass");

    // A relative path means something else in another directory, so it is looked up again.
    let elsewhere = session.exec_context(Path::new("elsewhere"));
    block_on(session.check(&src, FileId(0), &elsewhere));
    assert_eq!(engine.lookups_of("orders.csv"), 2);
    // So is the same directory under a sandbox, which may refuse the path.
    let sandboxed = ExecContext {
        sandbox: Some(".".into()),
        ..ctx.clone()
    };
    block_on(session.check(&src, FileId(0), &sandboxed));
    assert_eq!(engine.lookups_of("orders.csv"), 3);
}

#[test]
fn failed_lookups_are_cached_until_invalidated() {
    let (session, engine) = session();
    let ctx = ctx(&session);
    let src = source("from 'later.csv' | where amount > 1");
    for _ in 0..2 {
        let analysis = block_on(session.check(&src, FileId(0), &ctx));
        assert_eq!(codes(&analysis.diagnostics), [DS3001]);
    }
    assert_eq!(engine.lookups_of("later.csv"), 1);

    // The file appears, but the session keeps the cached failure until told otherwise.
    engine.tables.lock().unwrap().insert("later.csv".into());
    let analysis = block_on(session.check(&src, FileId(0), &ctx));
    assert_eq!(codes(&analysis.diagnostics), [DS3001]);

    session.invalidate_source(&SourceRef::Path("later.csv".into()));
    let analysis = block_on(session.check(&src, FileId(0), &ctx));
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    assert_eq!(engine.lookups_of("later.csv"), 2);
}

#[test]
fn invalidating_everything_fetches_every_source_again() {
    let (session, engine) = session();
    let ctx = ctx(&session);
    let src = source("from 'orders.csv' | take 1; from 'missing.csv' | take 1");
    block_on(session.check(&src, FileId(0), &ctx));
    session.invalidate_schemas();
    block_on(session.check(&src, FileId(0), &ctx));
    assert_eq!(engine.lookups_of("orders.csv"), 2);
    assert_eq!(engine.lookups_of("missing.csv"), 2);
}

#[test]
fn source_errors_become_diagnostics() {
    let (session, _) = session();
    let src = source("from 'order.csv' | where amount > 100");
    let analysis = block_on(session.check(&src, FileId(0), &ctx(&session)));
    assert_eq!(codes(&analysis.diagnostics), [DS3001]);
}

#[test]
fn capability_errors_come_from_the_engine_capabilities() {
    let mut engine = FakeEngine::new();
    engine.caps.functions = FunctionSupport::Only(BTreeSet::new());
    let (session, engine) = session_with(engine);
    let ctx = ctx(&session);
    let src = source("from 'orders.csv' | where strlen(name) > 1");

    let analysis = block_on(session.check(&src, FileId(0), &ctx));
    assert_eq!(codes(&analysis.diagnostics), [DS4001]);

    // `run` stops before anything executes.
    let output = block_on(session.run(&src, FileId(0), &ctx));
    assert_eq!(codes(&output.diagnostics), [DS4001]);
    assert!(output.results.is_empty());
    assert_eq!(engine.executions.load(Ordering::SeqCst), 0);

    // With full capabilities the same program is fine.
    let (full, _) = session_with(FakeEngine::new());
    let analysis = block_on(full.check(&src, FileId(0), &ctx));
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn run_executes_each_query() {
    let (session, _) = session();
    let src = source("from 'orders.csv' | project amount; from 'orders.csv' | take 1");
    let output = block_on(session.run(&src, FileId(0), &ctx(&session)));
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    assert_eq!(output.results.len(), 2);
    assert_eq!(output.results[0].schema.fields().len(), 1);
    assert!(
        output
            .results
            .iter()
            .all(|r| r.render.is_none() && !r.truncated)
    );
    let statements: Vec<_> = output
        .results
        .iter()
        .map(|r| r.statement.map(|s| (s.start, s.end)))
        .collect();
    assert_eq!(statements, [Some((0, 34)), Some((36, 62))]);
}

#[test]
fn run_statement_executes_only_the_chosen_query() {
    let (session, engine) = session();
    let src =
        source("let o = 'orders.csv';\nfrom o | project amount;\nfrom o | take 1 | project name");
    let output = block_on(session.run_statement(&src, FileId(0), &ctx(&session), 1));
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    assert_eq!(output.results.len(), 1);
    assert_eq!(output.results[0].schema.field(0).name(), "name");
    assert_eq!(engine.executions.load(Ordering::SeqCst), 1);

    let past_the_end = block_on(session.run_statement(&src, FileId(0), &ctx(&session), 2));
    assert!(past_the_end.diagnostics.is_empty());
    assert!(past_the_end.results.is_empty());
    assert_eq!(engine.executions.load(Ordering::SeqCst), 1);
}

#[test]
fn run_statement_still_reports_errors_in_other_statements() {
    let (session, engine) = session();
    let src = source("from 'orders.csv' | project nope; from 'orders.csv'");
    let output = block_on(session.run_statement(&src, FileId(0), &ctx(&session), 1));
    assert_eq!(codes(&output.diagnostics), [DS1001]);
    assert!(output.results.is_empty());
    assert_eq!(engine.executions.load(Ordering::SeqCst), 0);
}

#[test]
fn run_keeps_diagnostics_that_are_not_errors() {
    let (session, _) = session();
    let src = source("from 'orders.csv' | join kind=inner 'customers.csv' on name");
    let output = block_on(session.run(&src, FileId(0), &ctx(&session)));
    let found: Vec<_> = output
        .diagnostics
        .iter()
        .map(|d| (d.code, d.severity))
        .collect();
    assert_eq!(found, [(DS1007, Severity::Info)]);
    assert_eq!(output.results.len(), 1);

    // They are kept when execution fails too.
    let mut engine = FakeEngine::new();
    engine.fail_execute = true;
    let (session, _) = session_with(engine);
    let output = block_on(session.run(&src, FileId(0), &ctx(&session)));
    assert_eq!(codes(&output.diagnostics), [DS1007, DS4002]);
    assert!(output.results.is_empty());
}

#[test]
fn an_engine_error_stops_the_run() {
    let mut engine = FakeEngine::new();
    engine.fail_execute = true;
    let (session, engine) = session_with(engine);
    let src = source("from 'orders.csv' | take 1; from 'orders.csv' | take 2");
    let output = block_on(session.run(&src, FileId(0), &ctx(&session)));
    assert_eq!(codes(&output.diagnostics), [DS4002]);
    assert!(output.results.is_empty());
    assert_eq!(engine.executions.load(Ordering::SeqCst), 1);
}

#[test]
fn not_implemented_is_a_diagnostic() {
    let (session, _) = session();
    // `render` is checked but not implemented yet (M1-10).
    let src = source("from 'orders.csv' | render table");
    let output = block_on(session.run(&src, FileId(0), &ctx(&session)));
    assert_eq!(codes(&output.diagnostics), [DS9001]);
}

#[test]
fn run_warns_when_there_is_no_query() {
    let (session, _) = session();
    let ctx = ctx(&session);
    for text in ["", "   \n", "// only a comment\n", ";"] {
        let output = block_on(session.run(&source(text), FileId(0), &ctx));
        let found: Vec<_> = output
            .diagnostics
            .iter()
            .map(|d| (d.code, d.severity, d.primary.is_none()))
            .collect();
        assert_eq!(found, [(DS0004, Severity::Warning, true)], "{text:?}");
        assert!(output.results.is_empty());
    }
    // `check` accepts a program with no query.
    let analysis = block_on(session.check(&source(""), FileId(0), &ctx));
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn row_limit_truncates_and_cancels_the_stream() {
    let mut engine = FakeEngine::new();
    engine.batches = 100;
    engine.rows_per_batch = 2;
    let (session, engine) = session_with(engine);
    let ctx = ExecContext {
        row_limit: Some(3),
        ..ctx(&session)
    };
    let output = block_on(session.run(&source("from 'orders.csv'"), FileId(0), &ctx));
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    let result = &output.results[0];
    assert!(result.truncated);
    let rows: usize = result.batches.iter().map(RecordBatch::num_rows).sum();
    assert_eq!(rows, 3);
    // The session stopped pulling once it had enough, and dropped (cancelled) the stream.
    assert_eq!(engine.polled.load(Ordering::SeqCst), 2);
    assert!(engine.stream_dropped.load(Ordering::SeqCst));
}

#[test]
fn a_result_that_exactly_fills_the_row_limit_is_not_truncated() {
    let mut engine = FakeEngine::new();
    engine.batches = 2;
    engine.rows_per_batch = 2;
    let (session, _) = session_with(engine);
    let ctx = ExecContext {
        row_limit: Some(4),
        ..ctx(&session)
    };
    let output = block_on(session.run(&source("from 'orders.csv'"), FileId(0), &ctx));
    let result = &output.results[0];
    assert!(!result.truncated);
    assert_eq!(
        result
            .batches
            .iter()
            .map(RecordBatch::num_rows)
            .sum::<usize>(),
        4
    );
}

#[test]
fn dropping_a_result_stream_cancels_it() {
    let mut engine = FakeEngine::new();
    engine.batches = 100;
    let (session, engine) = session_with(engine);
    let ctx = ctx(&session);
    let program = block_on(session.compile(&source("from 'orders.csv'"), FileId(0), &ctx)).unwrap();
    let mut stream = block_on(engine.execute(&program.queries[0], &ctx)).unwrap();
    block_on(stream.batches.next()).unwrap().unwrap();
    assert!(!engine.stream_dropped.load(Ordering::SeqCst));
    drop(stream);
    assert!(engine.stream_dropped.load(Ordering::SeqCst));
    assert_eq!(engine.polled.load(Ordering::SeqCst), 1);
}

fn assert_panic_diagnostic(diagnostics: &[Diagnostic], said: &str) {
    assert_eq!(codes(diagnostics), [DS9001], "{diagnostics:?}");
    let d = &diagnostics[0];
    assert!(
        d.message
            .starts_with("internal error: DataScript crashed while "),
        "{}",
        d.message
    );
    assert!(d.notes.iter().any(|n| n.contains(said)), "{:?}", d.notes);
    assert!(
        d.notes.iter().any(|n| n.contains("please report it")),
        "{:?}",
        d.notes
    );
    assert_eq!(d.file, Some(FileId(0)));
}

#[test]
fn panics_become_internal_errors() {
    let src = source("from 'orders.csv' | take 1");
    for (panic_in, said) in [
        (PanicIn::SchemaOf, "schema_of exploded"),
        (PanicIn::Execute, "execute exploded"),
        (PanicIn::Stream, "stream exploded"),
    ] {
        let mut engine = FakeEngine::new();
        engine.panic_in = Some(panic_in);
        engine.batches = 3;
        let (session, _) = session_with(engine);
        let ctx = ctx(&session);
        let output = block_on(session.run(&src, FileId(0), &ctx));
        assert_panic_diagnostic(&output.diagnostics, said);
        assert!(output.results.is_empty());
        if panic_in == PanicIn::SchemaOf {
            let analysis = block_on(session.check(&src, FileId(0), &ctx));
            assert_panic_diagnostic(&analysis.diagnostics, said);
            let compiled = block_on(session.compile(&src, FileId(0), &ctx));
            assert_panic_diagnostic(&compiled.unwrap_err(), said);
            let explained = block_on(session.explain(&src, FileId(0), &ctx));
            assert_panic_diagnostic(&explained.unwrap_err(), said);
        }
    }
}

#[test]
fn the_session_still_works_after_a_panic() {
    let mut engine = FakeEngine::new();
    engine.panic_in = Some(PanicIn::Execute);
    let (session, engine) = session_with(engine);
    let ctx = ctx(&session);
    let src = source("from 'orders.csv' | take 1");
    block_on(session.run(&src, FileId(0), &ctx));
    // The schema fetched before the panic is still cached, and checking still works.
    let analysis = block_on(session.check(&src, FileId(0), &ctx));
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    assert_eq!(engine.lookups_of("orders.csv"), 1);
}

#[test]
fn catch_panic_passes_values_through_and_names_the_file() {
    assert_eq!(catch_panic(None, "testing", || 7).unwrap(), 7);
    let d = catch_panic(Some(FileId(3)), "testing", || -> () {
        panic!("{}", "boom\nmore")
    })
    .unwrap_err();
    assert_eq!(
        d.message,
        "internal error: DataScript crashed while testing"
    );
    assert_eq!(d.file, Some(FileId(3)));
    assert_eq!(d.notes[0], "the crash said: boom");
}
