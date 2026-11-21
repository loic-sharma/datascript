use std::sync::atomic::{AtomicUsize, Ordering};

use arrow_array::{ArrayRef, Float64Array};
use arrow_schema::{DataType, Field, Schema};
use datascript_diagnostics::codes::{DS3001, DS9001};
use futures::executor::block_on;

use super::*;

/// An in-memory engine: one known file, `orders.csv`, with one `amount` column.
struct FakeEngine {
    caps: Capabilities,
    lookups: AtomicUsize,
}

impl FakeEngine {
    fn new() -> Self {
        FakeEngine {
            caps: Capabilities::full(),
            lookups: AtomicUsize::new(0),
        }
    }

    fn schema() -> SchemaRef {
        Arc::new(Schema::new(vec![Field::new(
            "amount",
            DataType::Float64,
            true,
        )]))
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
        self.lookups.fetch_add(1, Ordering::SeqCst);
        match source {
            SourceRef::Path(p) if p == "orders.csv" => Ok(Self::schema()),
            _ => Err(SourceError::NotFound {
                nearby: vec!["orders.csv".into()],
            }),
        }
    }

    async fn execute(
        &self,
        plan: &datascript_compiler::ir::QueryPlan,
        _ctx: &ExecContext,
    ) -> Result<ResultStream, Diagnostic> {
        let column: ArrayRef = Arc::new(Float64Array::from(vec![150.0]));
        let batch = RecordBatch::try_new(plan.output_schema.clone(), vec![column]).unwrap();
        Ok(ResultStream {
            schema: plan.output_schema.clone(),
            batches: Box::pin(futures::stream::iter([Ok(batch)])),
        })
    }
}

fn session() -> (Session, Arc<FakeEngine>) {
    let engine = Arc::new(FakeEngine::new());
    (
        Session::new(engine.clone(), Arc::new(FixedClock(42))),
        engine,
    )
}

fn source(text: &str) -> SourceFile {
    SourceFile::new("query", text)
}

#[test]
fn native_session_is_send_and_sync() {
    fn assert_send<T: Send + Sync>() {}
    fn assert_send_future<F: std::future::Future + Send>(_: F) {}
    assert_send::<Session>();
    assert_send::<ExecContext>();
    let (session, _) = session();
    let ctx = session.exec_context(Path::new("."));
    let src = source("'orders.csv'");
    assert_send_future(session.check(&src, FileId(0), &ctx));
    assert_send_future(session.run(&src, FileId(0), &ctx));
}

#[test]
fn exec_context_reads_the_injected_clock() {
    let (session, _) = session();
    assert_eq!(session.exec_context(Path::new("x")).now_micros, 42);
}

#[test]
fn sources_are_fetched_then_bound_again() {
    let (session, engine) = session();
    let ctx = session.exec_context(Path::new("."));
    let src = source("'orders.csv' | where amount > 100 | take 1");
    let analysis = block_on(session.check(&src, FileId(0), &ctx));
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    assert!(analysis.pending_sources.is_empty());
    assert!(analysis.hir[0].plan.is_some());
    assert_eq!(engine.lookups.load(Ordering::SeqCst), 1);

    // The schema is cached: a second check does not ask the engine again.
    block_on(session.check(&src, FileId(0), &ctx));
    assert_eq!(engine.lookups.load(Ordering::SeqCst), 1);
}

#[test]
fn source_errors_become_diagnostics() {
    let (session, _) = session();
    let ctx = session.exec_context(Path::new("."));
    let src = source("'order.csv' | where amount > 100");
    let analysis = block_on(session.check(&src, FileId(0), &ctx));
    let codes: Vec<_> = analysis.diagnostics.iter().map(|d| d.code).collect();
    assert_eq!(codes, [DS3001]);
}

#[test]
fn run_executes_each_query() {
    let (session, _) = session();
    let ctx = session.exec_context(Path::new("."));
    let src = source("'orders.csv' | project amount; 'orders.csv' | take 1");
    let output = block_on(session.run(&src, FileId(0), &ctx));
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    assert_eq!(output.results.len(), 2);
}

#[test]
fn not_implemented_is_a_diagnostic() {
    let (session, _) = session();
    let ctx = session.exec_context(Path::new("."));
    let output = block_on(session.run(&source("'orders.csv' | count"), FileId(0), &ctx));
    let codes: Vec<_> = output.diagnostics.iter().map(|d| d.code).collect();
    assert_eq!(codes, [DS9001]);
}
