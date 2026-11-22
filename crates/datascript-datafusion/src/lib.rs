//! Apache DataFusion backend for DataScript.
//!
//! [`DataFusionEngine`] implements [`Engine`]: it reads source schemas ([`sources`]), lowers the
//! IR to a DataFusion `LogicalPlan` ([`lower`]), and runs it. Scalar functions and checked
//! arithmetic are UDFs ([`udf`], [`udaf`]) that carry the span of their expression, so runtime
//! errors point at the expression that failed ([`errors`]).
//!
//! Features: `compression` (default) reads `.gz`, `.bz2`, `.xz` and `.zst` CSV, TSV and JSON
//! files. `wasm` is the set that builds for `wasm32-unknown-unknown`; use it with
//! `--no-default-features` (docs/spikes/datafusion-wasm.md).

mod calendar;
mod checked;
mod errors;
mod lower;
mod sources;
mod udaf;
mod udf;

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use arrow_schema::SchemaRef;
use datafusion::arrow::array::{ArrayRef, RecordBatch};
use datafusion::arrow::compute::cast;
use datafusion::prelude::{SessionConfig, SessionContext};
use datascript_compiler::capabilities::FunctionSupport;
use datascript_compiler::ir::{PlanNode, QueryPlan};
use datascript_compiler::{Capabilities, SourceError, SourceRef};
use datascript_diagnostics::Diagnostic;
use datascript_engine::{Engine, ExecContext, ResultStream};
use futures::StreamExt;

use crate::lower::{Lowering, internal};
use crate::sources::Resolved;

/// Settings that make results deterministic in tests (D-017).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataFusionConfig {
    pub target_partitions: usize,
    pub batch_size: usize,
}

impl Default for DataFusionConfig {
    fn default() -> Self {
        DataFusionConfig {
            target_partitions: 1,
            batch_size: 8192,
        }
    }
}

/// The key of a resolved source: relative paths depend on the base directory, and what may be
/// read on the sandbox.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SourceKey {
    base_dir: PathBuf,
    sandbox: Option<PathBuf>,
    source: SourceRef,
}

impl SourceKey {
    fn new(source: &SourceRef, ctx: &ExecContext) -> Self {
        SourceKey {
            base_dir: ctx.base_dir.clone(),
            sandbox: ctx.sandbox.clone(),
            source: source.clone(),
        }
    }
}

/// The DataFusion engine.
///
/// It remembers the files and per-file schemas it found for each source in `schema_of`, so the
/// scan reads exactly the files the program was checked against without inferring their schemas
/// again. Each `schema_of` call reads the source afresh and replaces what it remembered, so a
/// session that invalidates its schema cache also refreshes this one.
#[derive(Debug)]
pub struct DataFusionEngine {
    config: DataFusionConfig,
    capabilities: Capabilities,
    /// Host tables (`--table name=path`, web UI drag-drop): name to source.
    tables: BTreeMap<String, SourceRef>,
    resolved: Mutex<HashMap<SourceKey, Arc<Resolved>>>,
}

/// What this backend supports: every join kind and format, the functions it has a mapping for
/// ([`lower::SUPPORTED`]), and no deviations (integer arithmetic is checked, D-014).
fn capabilities() -> Capabilities {
    Capabilities {
        functions: FunctionSupport::Only(lower::SUPPORTED.iter().map(|f| f.to_string()).collect()),
        ..Capabilities::full()
    }
}

impl DataFusionEngine {
    pub fn new(config: DataFusionConfig) -> Self {
        DataFusionEngine {
            config,
            capabilities: capabilities(),
            tables: BTreeMap::new(),
            resolved: Mutex::new(HashMap::new()),
        }
    }

    /// Registers a host table: `from name` reads `source`, a path, glob or reader call. Its
    /// relative paths resolve against the run's base directory, like paths in the program.
    /// Registering a name again replaces it.
    ///
    /// The host checks the path's format first (`datascript_compiler::source::format_of_path`);
    /// a host table whose format cannot be told fails when it is read.
    pub fn with_table(mut self, name: impl Into<String>, source: SourceRef) -> Self {
        self.tables.insert(name.into(), source);
        self
    }

    /// The registered host tables, sorted by name.
    pub fn tables(&self) -> impl Iterator<Item = (&str, &SourceRef)> {
        self.tables
            .iter()
            .map(|(name, source)| (name.as_str(), source))
    }

    fn context(&self) -> SessionContext {
        let config = SessionConfig::new()
            .with_target_partitions(self.config.target_partitions)
            .with_batch_size(self.config.batch_size);
        SessionContext::new_with_config(config)
    }

    /// Reads `source` and remembers what was found.
    async fn resolve(
        &self,
        session: &SessionContext,
        source: &SourceRef,
        ctx: &ExecContext,
    ) -> Result<Arc<Resolved>, SourceError> {
        let resolved = Arc::new(sources::resolve(session, source, &self.tables, ctx).await?);
        self.remembered()
            .insert(SourceKey::new(source, ctx), resolved.clone());
        Ok(resolved)
    }

    /// What `schema_of` found for `source`, or a fresh read if it was never asked.
    async fn resolved(
        &self,
        session: &SessionContext,
        source: &SourceRef,
        ctx: &ExecContext,
    ) -> Result<Arc<Resolved>, SourceError> {
        let known = self.remembered().get(&SourceKey::new(source, ctx)).cloned();
        match known {
            Some(resolved) => Ok(resolved),
            None => self.resolve(session, source, ctx).await,
        }
    }

    fn remembered(&self) -> std::sync::MutexGuard<'_, HashMap<SourceKey, Arc<Resolved>>> {
        // A panic while holding the lock leaves the map consistent (inserts are atomic).
        self.resolved.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Default for DataFusionEngine {
    fn default() -> Self {
        Self::new(DataFusionConfig::default())
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl Engine for DataFusionEngine {
    fn name(&self) -> &str {
        "datafusion"
    }

    fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    async fn schema_of(
        &self,
        source: &SourceRef,
        ctx: &ExecContext,
    ) -> Result<SchemaRef, SourceError> {
        let resolved = self.resolve(&self.context(), source, ctx).await?;
        Ok(resolved.widened.clone())
    }

    async fn execute(
        &self,
        plan: &QueryPlan,
        ctx: &ExecContext,
    ) -> Result<ResultStream, Diagnostic> {
        let session = self.context();
        // Resolve every scan first: that reads files, lowering does not.
        let mut resolved = Vec::new();
        for scan in lower::scans(&plan.root) {
            let PlanNode::Scan { source } = &scan.node else {
                continue;
            };
            let r = self.resolved(&session, source, ctx).await.map_err(|e| {
                // The schema was read during checking, so the files changed since.
                internal(
                    scan.span,
                    &format!("the source changed while the query ran: {e:?}"),
                )
            })?;
            resolved.push(r);
        }
        let mut lowering = Lowering::new(resolved, ctx.now_micros);
        let logical = lowering.plan(&plan.root)?;
        let errors = lowering.errors;
        let span = plan.root.span;
        let frame = session
            .execute_logical_plan(logical)
            .await
            .map_err(|e| errors.diagnostic(span, &e))?;
        let stream = frame
            .execute_stream()
            .await
            .map_err(|e| errors.diagnostic(span, &e))?;
        let schema = plan.output_schema.clone();
        let out = schema.clone();
        let batches = stream.map(move |batch| {
            let batch = batch.map_err(|e| errors.diagnostic(span, &e))?;
            conform(&out, &batch).map_err(|message| internal(span, &message))
        });
        Ok(ResultStream {
            schema,
            batches: Box::pin(batches),
        })
    }
}

/// `batch` with the plan's output schema: the canonical types and the field metadata that
/// carries extension types (D-024). A column whose type differs is cast; a cast that fails is
/// a lowering bug.
fn conform(schema: &SchemaRef, batch: &RecordBatch) -> Result<RecordBatch, String> {
    if batch.num_columns() != schema.fields().len() {
        return Err(format!(
            "the engine returned {} columns for {} in the plan",
            batch.num_columns(),
            schema.fields().len()
        ));
    }
    let columns = batch
        .columns()
        .iter()
        .zip(schema.fields())
        .map(|(column, field)| {
            if column.data_type() == field.data_type() {
                Ok(column.clone())
            } else {
                cast(column, field.data_type()).map_err(|e| {
                    format!(
                        "column `{}` came back as {}, not {}: {e}",
                        field.name(),
                        column.data_type(),
                        field.data_type()
                    )
                })
            }
        })
        .collect::<Result<Vec<ArrayRef>, String>>()?;
    RecordBatch::try_new(schema.clone(), columns).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_schema::{DataType, TimeUnit};
    use std::path::Path;

    fn fixtures() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/data")
    }

    #[test]
    fn strip_verbatim_removes_windows_prefixes() {
        let strip = |s: &str| {
            sources::strip_verbatim(Path::new(s))
                .to_string_lossy()
                .into_owned()
        };
        assert_eq!(strip(r"\\?\C:\data\orders.csv"), r"C:\data\orders.csv");
        assert_eq!(
            strip(r"\\?\UNC\server\share\orders.csv"),
            r"\\server\share\orders.csv"
        );
        assert_eq!(strip(r"C:\data\orders.csv"), r"C:\data\orders.csv");
        assert_eq!(strip("/data/orders.csv"), "/data/orders.csv");
        // Verbatim paths that are not disk or UNC paths have no plain form; leave them alone.
        assert_eq!(
            strip(r"\\?\Volume{abc}\orders.csv"),
            r"\\?\Volume{abc}\orders.csv"
        );
    }

    fn ctx() -> ExecContext {
        ExecContext {
            now_micros: 0,
            base_dir: fixtures(),
            sandbox: None,
            row_limit: None,
        }
    }

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(f)
    }

    #[test]
    fn orders_csv_is_widened_to_datascript_types() {
        let engine = DataFusionEngine::default();
        let source = SourceRef::Path("orders.csv".into());
        let schema = block_on(engine.schema_of(&source, &ctx())).unwrap();
        let types: Vec<(&str, &DataType)> = schema
            .fields()
            .iter()
            .map(|f| (f.name().as_str(), f.data_type()))
            .collect();
        assert_eq!(
            types,
            [
                ("orderId", &DataType::Utf8),
                ("customerId", &DataType::Utf8),
                ("amount", &DataType::Float64),
                (
                    "timestamp",
                    &DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into()))
                ),
            ]
        );
    }

    #[test]
    fn missing_file_lists_nearby_files() {
        let engine = DataFusionEngine::default();
        let source = SourceRef::Path("order.csv".into());
        let Err(SourceError::NotFound { nearby }) = block_on(engine.schema_of(&source, &ctx()))
        else {
            panic!("expected NotFound");
        };
        assert!(nearby.contains(&"orders.csv".to_string()), "{nearby:?}");
    }

    /// A fresh directory under the system temp directory, removed when dropped.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> TempDir {
            let dir =
                std::env::temp_dir().join(format!("datascript-df-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        fn write(&self, name: &str, contents: &str) -> PathBuf {
            let path = self.0.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, contents).unwrap();
            path
        }

        fn ctx(&self) -> ExecContext {
            ExecContext {
                base_dir: self.0.clone(),
                ..ctx()
            }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn schema_of(
        engine: &DataFusionEngine,
        source: SourceRef,
        ctx: &ExecContext,
    ) -> Result<SchemaRef, SourceError> {
        block_on(engine.schema_of(&source, ctx))
    }

    fn columns(schema: &SchemaRef) -> Vec<(String, DataType)> {
        schema
            .fields()
            .iter()
            .map(|f| (f.name().clone(), f.data_type().clone()))
            .collect()
    }

    #[test]
    fn a_directory_is_unreadable() {
        let dir = TempDir::new("isdir");
        std::fs::create_dir_all(dir.0.join("logs.csv")).unwrap();
        let err = schema_of(
            &DataFusionEngine::default(),
            SourceRef::Path("logs.csv".into()),
            &dir.ctx(),
        );
        assert_eq!(
            err,
            Err(SourceError::Unreadable {
                file: "logs.csv".into(),
                problem: datascript_compiler::ReadProblem::IsDirectory,
            })
        );
    }

    #[cfg(unix)]
    #[test]
    fn permission_denied_is_reported() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new("perm");
        let path = dir.write("secret.csv", "a\n1\n");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        // Root can read anything; the check means nothing then.
        if std::fs::File::open(&path).is_ok() {
            return;
        }
        let err = schema_of(
            &DataFusionEngine::default(),
            SourceRef::Path("secret.csv".into()),
            &dir.ctx(),
        );
        assert_eq!(
            err,
            Err(SourceError::Unreadable {
                file: "secret.csv".into(),
                problem: datascript_compiler::ReadProblem::PermissionDenied,
            })
        );
    }

    #[test]
    fn sandbox_rejects_paths_outside_it() {
        let dir = TempDir::new("sandbox");
        dir.write("inside/a.csv", "a\n1\n");
        dir.write("outside.csv", "a\n1\n");
        let ctx = ExecContext {
            base_dir: dir.0.join("inside"),
            sandbox: Some(dir.0.join("inside")),
            ..ctx()
        };
        let engine = DataFusionEngine::default();
        assert!(schema_of(&engine, SourceRef::Path("a.csv".into()), &ctx).is_ok());
        for source in [
            SourceRef::Path("../outside.csv".into()),
            SourceRef::Path("../missing.csv".into()),
            SourceRef::Glob("../*.csv".into()),
        ] {
            assert!(
                matches!(
                    schema_of(&engine, source.clone(), &ctx),
                    Err(SourceError::OutsideSandbox { .. })
                ),
                "{source}"
            );
        }
    }

    #[test]
    fn null_columns_take_the_other_files_type() {
        let dir = TempDir::new("nullmerge");
        dir.write("a.csv", "id,x\n1,\n2,\n");
        dir.write("b.csv", "id,x\n3,4\n");
        dir.write("c.csv", "id,y\n5,\n");
        let engine = DataFusionEngine::default();
        let schema = schema_of(&engine, SourceRef::Glob("*.csv".into()), &dir.ctx()).unwrap();
        assert_eq!(
            columns(&schema),
            [
                ("id".into(), DataType::Int64),
                ("x".into(), DataType::Int64),
                ("y".into(), DataType::Utf8),
            ]
        );
    }

    #[test]
    fn conflicts_name_the_first_typed_file() {
        let dir = TempDir::new("conflict");
        dir.write("a.csv", "x\n\n");
        dir.write("b.csv", "x\n1\n");
        dir.write("c.csv", "x\ntrue\n");
        let err = schema_of(
            &DataFusionEngine::default(),
            SourceRef::Glob("*.csv".into()),
            &dir.ctx(),
        );
        assert_eq!(
            err,
            Err(SourceError::Conflict {
                column: "x".into(),
                first: datascript_compiler::FileType {
                    file: "b.csv".into(),
                    ty: "long".into()
                },
                second: datascript_compiler::FileType {
                    file: "c.csv".into(),
                    ty: "bool".into()
                },
            })
        );
    }

    #[test]
    fn host_tables_resolve_and_report_their_files() {
        let dir = TempDir::new("hosttables");
        dir.write("data/orders.csv", "id\n1\n");
        let engine = DataFusionEngine::default()
            .with_table("orders", SourceRef::Path("data/orders.csv".into()))
            .with_table("gone", SourceRef::Path("data/gone.csv".into()));
        let ctx = dir.ctx();
        assert!(schema_of(&engine, SourceRef::Table("orders".into()), &ctx).is_ok());
        assert_eq!(
            schema_of(&engine, SourceRef::Table("ordrs".into()), &ctx),
            Err(SourceError::NotFound {
                nearby: vec!["gone".into(), "orders".into()]
            })
        );
        assert!(matches!(
            schema_of(&engine, SourceRef::Table("gone".into()), &ctx),
            Err(SourceError::Unreadable { file, .. }) if file == "data/gone.csv"
        ));
    }

    #[test]
    fn globs_are_sorted_and_cross_directories_with_double_star() {
        let dir = TempDir::new("globs");
        dir.write("b.csv", "n\n2\n");
        dir.write("a.csv", "n\n1\n");
        dir.write("sub/deep/c.csv", "n,m\n3,x\n");
        let engine = DataFusionEngine::default();
        let ctx = dir.ctx();
        let flat = schema_of(&engine, SourceRef::Glob("*.csv".into()), &ctx).unwrap();
        assert_eq!(flat.fields().len(), 1);
        let deep = schema_of(&engine, SourceRef::Glob("**/*.csv".into()), &ctx).unwrap();
        assert_eq!(deep.fields().len(), 2);
        let key = SourceKey::new(&SourceRef::Glob("**/*.csv".into()), &ctx);
        let files: Vec<String> = engine.remembered()[&key]
            .files
            .iter()
            .map(|f| f.url.as_str().rsplit('/').next().unwrap().to_string())
            .collect();
        assert_eq!(files, ["a.csv", "b.csv", "c.csv"]);
    }

    #[test]
    fn globs_resolve_against_a_canonicalized_base_dir() {
        // On Windows `canonicalize` adds a verbatim `\\?\` prefix, which must not stop matches.
        let dir = TempDir::new("canonical");
        dir.write("data/a.csv", "n\n1\n");
        dir.write("cases/.keep", "");
        let ctx = ExecContext {
            base_dir: dir.0.join("cases").canonicalize().unwrap(),
            ..dir.ctx()
        };
        let engine = DataFusionEngine::default();
        let schema = schema_of(&engine, SourceRef::Glob("../data/*.csv".into()), &ctx).unwrap();
        assert_eq!(schema.fields().len(), 1);
    }
}
