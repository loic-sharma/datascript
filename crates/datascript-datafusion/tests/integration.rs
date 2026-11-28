//! `Session` over the real `DataFusionEngine` (M2-29): schema-cache invalidation, streaming and
//! row limits, cancellation and rollback, multi-partition parity, and reader edge cases on files
//! generated in a temporary directory.
//!
//! The in-crate tests of `datascript-engine` cover the same session logic against an in-memory
//! engine; these check that the real engine keeps the contract.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use datascript_compiler::SourceRef;
use datascript_datafusion::{DataFusionConfig, DataFusionEngine};
use datascript_diagnostics::codes::{DS3001, DS3008, DS4003};
use datascript_diagnostics::{Code, Diagnostic, FileId, SourceFile};
use datascript_engine::table::format_table;
use datascript_engine::{Engine, ExecContext, FixedClock, QueryResult, RunOutput, Session};
use futures::StreamExt;
use tempfile::TempDir;

// ---- helpers ----

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(f)
}

fn session_with(config: DataFusionConfig) -> Session {
    Session::new(
        Arc::new(DataFusionEngine::new(config)),
        Arc::new(FixedClock(0)),
    )
}

fn session() -> Session {
    session_with(DataFusionConfig::default())
}

/// A config whose batches hold two rows, so small files come back in several batches.
fn small_batches() -> DataFusionConfig {
    DataFusionConfig {
        batch_size: 2,
        ..DataFusionConfig::default()
    }
}

fn ctx(base_dir: &Path) -> ExecContext {
    ExecContext {
        now_micros: 0,
        base_dir: base_dir.to_path_buf(),
        sandbox: None,
        row_limit: None,
    }
}

fn limited(base_dir: &Path, row_limit: usize) -> ExecContext {
    ExecContext {
        row_limit: Some(row_limit),
        ..ctx(base_dir)
    }
}

/// The repository's shared fixtures.
fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/data")
}

fn write(dir: &TempDir, name: &str, contents: &str) {
    std::fs::write(dir.path().join(name), contents).unwrap();
}

fn src(text: &str) -> SourceFile {
    SourceFile::new("query", text)
}

fn run(session: &Session, text: &str, ctx: &ExecContext) -> RunOutput {
    block_on(session.run(&src(text), FileId(0), ctx))
}

fn codes(diagnostics: &[Diagnostic]) -> Vec<Code> {
    diagnostics.iter().map(|d| d.code).collect()
}

/// The table of each result, as the golden tests print it.
fn tables(results: &[QueryResult]) -> Vec<String> {
    results
        .iter()
        .map(|r| format_table(&r.schema, &r.batches))
        .collect()
}

/// Runs `text` and returns its tables, failing on any diagnostic.
fn run_ok(session: &Session, text: &str, ctx: &ExecContext) -> Vec<String> {
    let output = run(session, text, ctx);
    assert!(output.diagnostics.is_empty(), "{:#?}", output.diagnostics);
    tables(&output.results)
}

fn rows(result: &QueryResult) -> usize {
    result.batches.iter().map(|b| b.num_rows()).sum()
}

/// A run that failed with exactly one error, `code`, and kept no results.
fn assert_fails_with(output: &RunOutput, code: Code) {
    assert_eq!(
        codes(&output.diagnostics),
        [code],
        "{:#?}",
        output.diagnostics
    );
    assert!(output.results.is_empty());
}

/// Everything a diagnostic says, for `contains` checks.
fn said(d: &Diagnostic) -> String {
    format!("{d:?}")
}

/// `n.csv`: one `long` column `n` holding 1..=7.
fn numbers(dir: &TempDir) {
    let body: String = (1..=7).map(|n| format!("{n}\n")).collect();
    write(dir, "n.csv", &format!("n\n{body}"));
}

// ---- schema cache invalidation ----

#[test]
fn invalidating_a_source_picks_up_its_new_schema_and_data() {
    let dir = TempDir::new().unwrap();
    let session = session();
    let ctx = ctx(dir.path());
    let query = "from 'data.csv'";
    write(&dir, "data.csv", "a\n1\n2\n");
    assert_eq!(
        run_ok(&session, query, &ctx),
        ["a\nlong\n----\n   1\n   2\n"]
    );

    write(&dir, "data.csv", "a,b\n3,x\n");
    session.invalidate_source(&SourceRef::Path("data.csv".into()));
    assert_eq!(
        run_ok(&session, query, &ctx),
        ["a    | b\nlong | string\n-----+-------\n   3 | x\n"]
    );

    // `invalidate_schemas` forgets everything, so a change of type is seen too.
    write(&dir, "data.csv", "a,b\n4.5,y\n5.25,z\n");
    session.invalidate_schemas();
    assert_eq!(
        run_ok(&session, query, &ctx),
        ["a    | b\nreal | string\n-----+-------\n4.5  | y\n5.25 | z\n"]
    );
}

#[test]
fn a_cached_missing_file_clears_once_it_exists_and_is_invalidated() {
    let dir = TempDir::new().unwrap();
    let session = session();
    let ctx = ctx(dir.path());
    let query = "from 'later.csv'";
    assert_fails_with(&run(&session, query, &ctx), DS3001);

    // The failure is cached: creating the file is not enough.
    write(&dir, "later.csv", "a\n1\n");
    assert_fails_with(&run(&session, query, &ctx), DS3001);

    session.invalidate_source(&SourceRef::Path("later.csv".into()));
    assert_eq!(run_ok(&session, query, &ctx), ["a\nlong\n----\n   1\n"]);
}

#[test]
fn a_file_deleted_between_check_and_run_is_a_diagnostic() {
    let dir = TempDir::new().unwrap();
    let session = session();
    let ctx = ctx(dir.path());
    write(&dir, "data.csv", "a\n1\n");
    let analysis = block_on(session.check(&src("from 'data.csv'"), FileId(0), &ctx));
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );

    std::fs::remove_file(dir.path().join("data.csv")).unwrap();
    // The run uses the checked schema, but finds the file gone rather than printing no rows.
    let output = run(&session, "from 'data.csv'", &ctx);
    assert_fails_with(&output, DS3008);
    let d = &output.diagnostics[0];
    assert_eq!(d.message, "cannot read `data.csv`");
    assert!(said(d).contains("moved or deleted"), "{d:#?}");
    assert!(d.primary.is_some(), "{d:#?}");
}

#[test]
fn a_file_deleted_before_another_engine_runs_the_plan_is_not_an_internal_error() {
    // An engine that never read the source reads it again to run the plan, and finds it gone.
    let dir = TempDir::new().unwrap();
    let session = session();
    let ctx = ctx(dir.path());
    write(&dir, "data.csv", "a\n1\n");
    let program = block_on(session.compile(&src("from 'data.csv'"), FileId(0), &ctx)).unwrap();
    std::fs::remove_file(dir.path().join("data.csv")).unwrap();
    let Err(d) = block_on(DataFusionEngine::default().execute(&program.queries[0], &ctx)) else {
        panic!("expected an error");
    };
    assert_eq!(d.code, DS3008, "{d:#?}");
    assert_eq!(d.message, "cannot read `data.csv`");
    assert!(said(&d).contains("moved or deleted"), "{d:#?}");
}

/// Restores a directory's permissions when dropped, so the `TempDir` can be removed.
#[cfg(unix)]
struct Unlock(PathBuf);

#[cfg(unix)]
impl Drop for Unlock {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
    }
}

#[cfg(unix)]
#[test]
fn a_file_locked_away_between_check_and_run_is_not_reported_as_deleted() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new().unwrap();
    let sub = dir.path().join("sub");
    std::fs::create_dir(&sub).unwrap();
    std::fs::write(sub.join("data.csv"), "a\n1\n").unwrap();
    let session = session();
    let ctx = ctx(dir.path());
    let query = "from 'sub/data.csv'";
    let analysis = block_on(session.check(&src(query), FileId(0), &ctx));
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );

    std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o000)).unwrap();
    let _unlock = Unlock(sub.clone());
    // Root can read anything; the check means nothing then.
    if std::fs::metadata(sub.join("data.csv")).is_ok() {
        return;
    }
    let output = run(&session, query, &ctx);
    assert_fails_with(&output, DS3008);
    let d = &output.diagnostics[0];
    assert_eq!(d.message, "cannot read `sub/data.csv`");
    assert!(said(d).contains("permission denied"), "{d:#?}");
    assert!(!said(d).contains("moved or deleted"), "{d:#?}");
}

#[test]
fn a_file_changed_between_check_and_run_is_a_diagnostic() {
    for changed in [
        // A value that no longer parses as the checked type.
        "a\nnot a number\n",
        // A column more than the checked schema has.
        "a,b\n1,2\n",
    ] {
        let dir = TempDir::new().unwrap();
        let session = session();
        let ctx = ctx(dir.path());
        write(&dir, "data.csv", "a\n1\n");
        let analysis = block_on(session.check(&src("from 'data.csv'"), FileId(0), &ctx));
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );

        write(&dir, "data.csv", changed);
        let output = run(&session, "from 'data.csv'", &ctx);
        assert_fails_with(&output, DS3008);
        assert_eq!(output.diagnostics[0].message, "cannot read `data.csv`");
    }

    // A file emptied since the check reads as no rows: that is what it holds now.
    let dir = TempDir::new().unwrap();
    let session = session();
    let ctx = ctx(dir.path());
    write(&dir, "data.csv", "a\n1\n");
    block_on(session.check(&src("from 'data.csv'"), FileId(0), &ctx));
    write(&dir, "data.csv", "");
    assert_eq!(
        run_ok(&session, "from 'data.csv'", &ctx),
        ["a\nlong\n----\n"]
    );
}

// ---- streaming and limits ----

const TWO_QUERIES: &str =
    "from 'n.csv' | sort by n asc; from 'n.csv' | where n % 2 == 0 | sort by n desc";

#[test]
fn start_streams_the_same_results_as_run_in_several_batches() {
    let dir = TempDir::new().unwrap();
    numbers(&dir);
    let session = session_with(small_batches());
    let ctx = ctx(dir.path());
    let ran = run_ok(&session, TWO_QUERIES, &ctx);
    assert_eq!(ran.len(), 2);

    let mut stream = block_on(session.start(&src(TWO_QUERIES), FileId(0), &ctx));
    assert!(stream.diagnostics.is_empty(), "{:?}", stream.diagnostics);
    let mut streamed = Vec::new();
    let mut batch_counts = Vec::new();
    while let Some(query) = block_on(stream.next_query()) {
        let mut query = query.unwrap();
        let mut batches = Vec::new();
        while let Some(batch) = block_on(query.batches.next()) {
            batches.push(batch.unwrap());
        }
        batch_counts.push(batches.len());
        streamed.push(format_table(&query.schema, &batches));
    }
    assert_eq!(streamed, ran);
    // 7 rows and 3 rows in batches of 2.
    assert!(
        batch_counts[0] >= 4 && batch_counts[1] >= 2,
        "{batch_counts:?}"
    );
}

#[test]
fn row_limit_zero_keeps_no_rows() {
    let dir = TempDir::new().unwrap();
    numbers(&dir);
    let session = session_with(small_batches());
    let text = "from 'n.csv'; from 'n.csv' | where n > 100";
    let output = run(&session, text, &limited(dir.path(), 0));
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    let found: Vec<(usize, bool)> = output
        .results
        .iter()
        .map(|r| (rows(r), r.truncated))
        .collect();
    // Rows were left out of the first result only: the second has none to leave out.
    assert_eq!(found, [(0, true), (0, false)]);
    // The schema is still there.
    assert_eq!(output.results[0].schema.field(0).name(), "n");
}

#[test]
fn row_limit_exact_match_and_mid_batch_cut() {
    let dir = TempDir::new().unwrap();
    numbers(&dir);
    let session = session_with(small_batches());
    let text = "from 'n.csv' | sort by n asc";

    let exact = run(&session, text, &limited(dir.path(), 7));
    assert_eq!(
        (rows(&exact.results[0]), exact.results[0].truncated),
        (7, false)
    );

    // Batches of 2: the third row is the first of the second batch.
    let cut = run(&session, text, &limited(dir.path(), 3));
    assert_eq!((rows(&cut.results[0]), cut.results[0].truncated), (3, true));
    assert_eq!(tables(&cut.results), ["n\nlong\n----\n   1\n   2\n   3\n"]);
}

#[test]
fn row_limit_applies_to_each_query_on_its_own() {
    let dir = TempDir::new().unwrap();
    numbers(&dir);
    let session = session_with(small_batches());
    // 7 rows, then 3 rows, then 2 rows, with a limit of 3.
    let text = "from 'n.csv'; from 'n.csv' | where n % 2 == 0; from 'n.csv' | take 2";
    let ctx = limited(dir.path(), 3);
    let output = run(&session, text, &ctx);
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    let found: Vec<(usize, bool)> = output
        .results
        .iter()
        .map(|r| (rows(r), r.truncated))
        .collect();
    assert_eq!(found, [(3, true), (3, false), (2, false)]);

    // Streaming with the same limit gives the same results.
    let mut stream = block_on(session.start(&src(text), FileId(0), &ctx));
    let mut streamed = Vec::new();
    while let Some(query) = block_on(stream.next_query()) {
        let result = block_on(query.unwrap().collect(ctx.row_limit)).unwrap();
        streamed.push((rows(&result), result.truncated));
    }
    assert_eq!(streamed, found);
}

// ---- cancellation and rollback ----

#[test]
fn dropping_a_query_stream_mid_stream_stops_it_cleanly() {
    let dir = TempDir::new().unwrap();
    numbers(&dir);
    let session = session_with(small_batches());
    let ctx = ctx(dir.path());
    let mut stream = block_on(session.start(&src(TWO_QUERIES), FileId(0), &ctx));
    let mut first = block_on(stream.next_query()).unwrap().unwrap();
    let batch = block_on(first.batches.next()).unwrap().unwrap();
    assert_eq!(batch.num_rows(), 2);
    drop(first);

    // The next query still runs, from the start.
    let second = block_on(stream.next_query()).unwrap().unwrap();
    let result = block_on(second.collect(None)).unwrap();
    assert_eq!(tables(&[result]), ["n\nlong\n----\n   6\n   4\n   2\n"]);
    assert!(block_on(stream.next_query()).is_none());
}

#[test]
fn dropping_a_run_stream_between_queries_stops_cleanly() {
    let dir = TempDir::new().unwrap();
    numbers(&dir);
    let session = session_with(small_batches());
    let ctx = ctx(dir.path());
    let mut stream = block_on(session.start(&src(TWO_QUERIES), FileId(0), &ctx));
    let first = block_on(stream.next_query()).unwrap().unwrap();
    assert_eq!(rows(&block_on(first.collect(None)).unwrap()), 7);
    drop(stream);

    // The session is unaffected and runs the whole program again.
    assert_eq!(run_ok(&session, TWO_QUERIES, &ctx).len(), 2);
}

#[test]
fn a_failing_second_query_clears_the_first_result() {
    let dir = TempDir::new().unwrap();
    numbers(&dir);
    let session = session_with(small_batches());
    let ctx = ctx(dir.path());
    // `n - n` is 0 in every row, so the division fails while the query runs, not while it is
    // planned.
    let text = "from 'n.csv'; from 'n.csv' | extend x = n / (n - n)";
    assert_fails_with(&run(&session, text, &ctx), DS4003);

    // Streaming, the first query's result did arrive before the second failed.
    let mut stream = block_on(session.start(&src(text), FileId(0), &ctx));
    let first = block_on(stream.next_query()).unwrap().unwrap();
    assert_eq!(rows(&block_on(first.collect(None)).unwrap()), 7);
    let second = block_on(stream.next_query()).unwrap().unwrap();
    assert_eq!(block_on(second.collect(None)).unwrap_err().code, DS4003);
}

// ---- multi-partition parity ----

#[test]
fn several_partitions_give_the_same_results_as_one() {
    let text = "\
        from 'orders.csv' | join kind = leftouter 'customers.csv' on customerId \
            | project orderId, customerId, name, amount | sort by orderId asc;\n\
        from 'orders.csv' | summarize revenue = sum(amount), orders = count(), first = min(timestamp) by customerId \
            | sort by customerId asc;\n\
        from 'orders.csv' | sort by amount desc, orderId asc | project orderId, amount;\n\
        from 'orders.csv' | join kind = leftanti 'customers.csv' on customerId | sort by orderId asc";
    let ctx = ctx(&fixtures());
    let results = |target_partitions: usize| {
        let session = session_with(DataFusionConfig {
            target_partitions,
            batch_size: 4,
        });
        run_ok(&session, text, &ctx)
    };
    let one = results(1);
    assert_eq!(one.len(), 4);
    assert_eq!(results(4), one);
}

// ---- reader edge cases ----

/// A file whose first `DEFAULT_INFER_ROWS` (1000) values are numbers, then text.
fn late_text(rows: usize, line: impl Fn(usize) -> String, last: &str) -> String {
    let mut text: String = (0..rows).map(line).collect();
    text.push_str(last);
    text
}

#[test]
fn a_csv_value_past_the_inferred_rows_is_ds3008_at_runtime() {
    let dir = TempDir::new().unwrap();
    let body = late_text(1200, |i| format!("{i}\n"), "oops\n");
    write(&dir, "late.csv", &format!("n\n{body}"));
    let session = session();
    let output = run(
        &session,
        "from 'late.csv' | summarize total = sum(n)",
        &ctx(dir.path()),
    );
    assert_fails_with(&output, DS3008);
    let d = &output.diagnostics[0];
    assert_eq!(d.message, "cannot read `late.csv`");
    assert!(d.primary.is_some(), "{d:#?}");
    assert!(said(d).contains("infer_rows"), "{d:#?}");
}

#[test]
fn an_ndjson_value_past_the_inferred_rows_is_ds3008_at_runtime() {
    let dir = TempDir::new().unwrap();
    let body = late_text(1200, |i| format!("{{\"n\": {i}}}\n"), "{\"n\": \"oops\"}\n");
    write(&dir, "late.ndjson", &body);
    let session = session();
    let output = run(
        &session,
        "from 'late.ndjson' | summarize total = sum(n)",
        &ctx(dir.path()),
    );
    assert_fails_with(&output, DS3008);
    let d = &output.diagnostics[0];
    assert_eq!(d.message, "cannot read `late.ndjson`");
    assert!(d.primary.is_some(), "{d:#?}");
}

#[cfg(feature = "compression")]
#[test]
fn a_gzipped_ndjson_file_is_read() {
    let session = session();
    assert_eq!(
        run_ok(
            &session,
            "from 'compressed/events.ndjson.gz' | sort by id asc",
            &ctx(&fixtures())
        ),
        ["id   | name\nlong | string\n-----+-------\n   1 | ant\n   2 | bee\n"]
    );
}

#[cfg(not(feature = "compression"))]
#[test]
fn a_gzipped_ndjson_file_needs_the_compression_feature() {
    let session = session();
    let output = run(
        &session,
        "from 'compressed/events.ndjson.gz'",
        &ctx(&fixtures()),
    );
    assert_fails_with(&output, DS3008);
    let d = &output.diagnostics[0];
    assert!(
        said(d).contains("this build cannot read compressed files"),
        "{d:#?}"
    );
}
