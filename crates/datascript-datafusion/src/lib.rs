//! Apache DataFusion backend for DataScript.
//!
//! [`DataFusionEngine`] implements [`Engine`]: it reads source schemas ([`sources`]), lowers the
//! IR to a DataFusion `LogicalPlan`, and runs it. Lowering handles the `Scan`, `Filter`,
//! `Project` and `Limit` nodes so far; M1-09 builds it out.
//!
//! Features: `compression` (default) reads `.gz`, `.bz2`, `.xz` and `.zst` CSV, TSV and JSON
//! files. `wasm` is the set that builds for `wasm32-unknown-unknown`; use it with
//! `--no-default-features` (docs/spikes/datafusion-wasm.md).

mod checked;
mod sources;

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use arrow_schema::{DataType, SchemaRef};
use datafusion::common::{Column, ScalarValue};
use datafusion::datasource::listing::{ListingTable, ListingTableConfig};
use datafusion::datasource::provider_as_source;
use datafusion::error::DataFusionError;
use datafusion::functions::expr_fn::nullif;
use datafusion::logical_expr::{LogicalPlan, LogicalPlanBuilder, Operator, binary_expr, cast};
use datafusion::prelude::{Expr as DfExpr, SessionConfig, SessionContext, lit};
use datascript_compiler::arrow::to_arrow_type;
use datascript_compiler::ir::{
    BinaryOp, Expr, ExprKind, Plan, PlanNode, PrefixOp, QueryPlan, Value,
};
use datascript_compiler::{Capabilities, SourceError, SourceRef};
use datascript_diagnostics::codes::{DS4002, DS4003, DS9001};
use datascript_diagnostics::{Diagnostic, Span};
use datascript_engine::{Engine, ExecContext, ResultStream};
use futures::StreamExt;

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

impl DataFusionEngine {
    pub fn new(config: DataFusionConfig) -> Self {
        DataFusionEngine {
            config,
            capabilities: Capabilities::full(),
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
        let mut lowering = Lowering {
            engine: self,
            session: &session,
            ctx,
            tables: 0,
        };
        let logical = lowering.plan(&plan.root).await?;
        let span = plan.root.span;
        let frame = session
            .execute_logical_plan(logical)
            .await
            .map_err(|e| engine_error(span, &e))?;
        let stream = frame
            .execute_stream()
            .await
            .map_err(|e| engine_error(span, &e))?;
        let batches = stream.map(move |batch| batch.map_err(|e| engine_error(span, &e)));
        Ok(ResultStream {
            schema: plan.output_schema.clone(),
            batches: Box::pin(batches),
        })
    }
}

/// Lowers IR plans to DataFusion logical plans.
struct Lowering<'a> {
    engine: &'a DataFusionEngine,
    session: &'a SessionContext,
    ctx: &'a ExecContext,
    /// Scans so far; each gets its own table name.
    tables: usize,
}

impl Lowering<'_> {
    async fn plan(&mut self, plan: &Plan) -> Result<LogicalPlan, Diagnostic> {
        // Only single-input nodes lower for now, so walk the first inputs to the bottom and build
        // upwards; the first node that cannot be lowered yet is reported (M1-09).
        let mut chain = vec![plan];
        while let Some(input) = chain.last().and_then(|p| p.first_input()) {
            chain.push(input);
        }
        let mut builder: Option<LogicalPlanBuilder> = None;
        for node in chain.into_iter().rev() {
            let span = node.span;
            let fail = |e: DataFusionError| engine_error(span, &e);
            builder = Some(match (&node.node, builder) {
                (PlanNode::Scan { source }, _) => self.scan(source, node).await?,
                (PlanNode::Filter { predicate, .. }, Some(b)) => {
                    b.filter(lower_expr(predicate)?).map_err(fail)?
                }
                (PlanNode::Project { columns, .. }, Some(b)) => {
                    let exprs = columns
                        .iter()
                        .map(|item| Ok(lower_expr(&item.expr)?.alias(&item.name)))
                        .collect::<Result<Vec<_>, Diagnostic>>()?;
                    b.project(exprs).map_err(fail)?
                }
                (PlanNode::Limit { count, .. }, Some(b)) => {
                    let count = usize::try_from(*count).unwrap_or(usize::MAX);
                    b.limit(0, Some(count)).map_err(fail)?
                }
                (
                    PlanNode::Values { .. }
                    | PlanNode::Aggregate { .. }
                    | PlanNode::Sort { .. }
                    | PlanNode::TopN { .. }
                    | PlanNode::Distinct { .. }
                    | PlanNode::Join { .. }
                    | PlanNode::Union { .. },
                    _,
                ) => return Err(cannot_run_yet(node)),
                (_, None) => return Err(internal(span, "a plan node has no input")),
            });
        }
        let builder = builder.ok_or_else(|| internal(plan.span, "the plan is empty"))?;
        builder.build().map_err(|e| engine_error(plan.span, &e))
    }

    /// Scans a source and casts each column to the type of `plan.schema`.
    ///
    /// Files with the same schema are read by one table. Otherwise each file is read with its
    /// own schema, projected to the merged columns (null where a file lacks one), and the files
    /// are combined with `UNION ALL`, in path order. That keeps CSV columns aligned by name when
    /// headers differ: DataFusion applies a table schema to CSV files by position.
    async fn scan(
        &mut self,
        source: &SourceRef,
        plan: &Plan,
    ) -> Result<LogicalPlanBuilder, Diagnostic> {
        let span = plan.span;
        let resolved = self
            .engine
            .resolved(self.session, source, self.ctx)
            .await
            .map_err(|e| {
                // The schema was read during checking, so the files changed since.
                internal(
                    span,
                    &format!("the source changed while the query ran: {e:?}"),
                )
            })?;
        let fail = |e: DataFusionError| engine_error(span, &e);
        let groups: Vec<Vec<&sources::SourceFile>> = if resolved.uniform() {
            vec![resolved.files.iter().collect()]
        } else {
            resolved.files.iter().map(|f| vec![f]).collect()
        };
        let mut combined: Option<LogicalPlanBuilder> = None;
        let null_value = resolved.reader.null_value();
        for files in groups {
            let raw = files[0].raw.clone();
            // With `null_value`, read text and turn the value into null here (see `CsvReader`).
            let read_as = match null_value {
                Some(_) => as_text(&raw),
                None => raw.clone(),
            };
            let urls = files.iter().map(|f| f.url.clone()).collect();
            let config = ListingTableConfig::new_with_multi_paths(urls)
                .with_listing_options(resolved.reader.listing_options())
                .with_schema(read_as.clone());
            let provider = ListingTable::try_new(config).map_err(fail)?;
            self.tables += 1;
            let name = format!("source{}", self.tables);
            let builder =
                LogicalPlanBuilder::scan(name, provider_as_source(Arc::new(provider)), None)
                    .map_err(fail)?;
            let columns = plan
                .schema
                .fields()
                .iter()
                .map(|field| scan_column(&read_as, field.name(), field.data_type(), null_value))
                .collect::<Vec<_>>();
            let builder = builder.project(columns).map_err(fail)?;
            combined = Some(match combined {
                None => builder,
                Some(before) => before.union(builder.build().map_err(fail)?).map_err(fail)?,
            });
        }
        combined.ok_or_else(|| internal(span, "a source with no files"))
    }
}

/// Column `name` of a file with schema `raw`, as type `target`: cast if needed, null if the
/// file does not have it, null where it holds `null_value`. `UInt64` is converted with a check,
/// since values above `i64::MAX` must be an error naming the column (LANGUAGE.md §5.1).
fn scan_column(
    raw: &arrow_schema::Schema,
    name: &str,
    target: &DataType,
    null_value: Option<&str>,
) -> DfExpr {
    let Ok(field) = raw.field_with_name(name) else {
        return cast(lit(ScalarValue::Null), target.clone()).alias(name);
    };
    let mut column = DfExpr::Column(Column::new_unqualified(name));
    if let Some(text) = null_value {
        column = nullif(column, lit(text));
    }
    if field.data_type() == target {
        column.alias(name)
    } else if field.data_type() == &DataType::UInt64 && target == &DataType::Int64 {
        checked::uint64_to_long(name).call(vec![column]).alias(name)
    } else {
        cast(column, target.clone()).alias(name)
    }
}

/// `schema` with every column read as text.
fn as_text(schema: &arrow_schema::Schema) -> SchemaRef {
    let fields: Vec<arrow_schema::Field> = schema
        .fields()
        .iter()
        .map(|f| arrow_schema::Field::new(f.name(), DataType::Utf8, true))
        .collect();
    Arc::new(arrow_schema::Schema::new(fields))
}

fn lower_expr(expr: &Expr) -> Result<DfExpr, Diagnostic> {
    Ok(match &expr.kind {
        // `Column::new_unqualified`, not `col()`, which lowercases unquoted names.
        ExprKind::Column(name) => DfExpr::Column(Column::new_unqualified(name)),
        ExprKind::Literal(value) => lit(match value {
            Value::Bool(b) => ScalarValue::Boolean(Some(*b)),
            Value::Long(n) => ScalarValue::Int64(Some(*n)),
            Value::Real(x) => ScalarValue::Float64(Some(*x)),
            Value::String(s) => ScalarValue::Utf8(Some(s.clone())),
            Value::DateTime(us) => ScalarValue::TimestampMicrosecond(Some(*us), Some("UTC".into())),
            Value::TimeSpan(us) => ScalarValue::DurationMicrosecond(Some(*us)),
            Value::Null => ScalarValue::try_from(&to_arrow_type(expr.ty))
                .map_err(|e| engine_error(expr.span, &e))?,
        }),
        ExprKind::Binary { op, left, right } => binary_expr(
            lower_expr(left)?,
            lower_op(*op, expr.span)?,
            lower_expr(right)?,
        ),
        ExprKind::Unary { op, operand } => match op {
            PrefixOp::Neg => DfExpr::Negative(Box::new(lower_expr(operand)?)),
            PrefixOp::Not => DfExpr::Not(Box::new(lower_expr(operand)?)),
        },
        ExprKind::In {
            expr: lhs,
            list,
            negated,
            case_insensitive: false,
        } => {
            let list = list.iter().map(lower_expr).collect::<Result<Vec<_>, _>>()?;
            lower_expr(lhs)?.in_list(list, *negated)
        }
        ExprKind::Between {
            expr: lhs,
            low,
            high,
            negated,
        } => {
            let (lhs, low, high) = (lower_expr(lhs)?, lower_expr(low)?, lower_expr(high)?);
            if *negated {
                lhs.not_between(low, high)
            } else {
                lhs.between(low, high)
            }
        }
        ExprKind::Cast(inner) => cast(lower_expr(inner)?, to_arrow_type(expr.ty)),
        ExprKind::In {
            case_insensitive: true,
            ..
        } => return Err(not_implemented(expr.span, "`in~`")),
        ExprKind::Call { function, .. } => {
            return Err(not_implemented(
                expr.span,
                &format!("the function `{function}`"),
            ));
        }
    })
}

/// The arithmetic, comparison and logical operators; M1-09 maps the string operators.
fn lower_op(op: BinaryOp, span: Span) -> Result<Operator, Diagnostic> {
    Ok(match op {
        BinaryOp::Eq => Operator::Eq,
        BinaryOp::NotEq => Operator::NotEq,
        BinaryOp::Lt => Operator::Lt,
        BinaryOp::LtEq => Operator::LtEq,
        BinaryOp::Gt => Operator::Gt,
        BinaryOp::GtEq => Operator::GtEq,
        BinaryOp::And => Operator::And,
        BinaryOp::Or => Operator::Or,
        BinaryOp::Add => Operator::Plus,
        BinaryOp::Sub => Operator::Minus,
        BinaryOp::Mul => Operator::Multiply,
        BinaryOp::Div => Operator::Divide,
        BinaryOp::Mod => Operator::Modulo,
        BinaryOp::EqCi | BinaryOp::NotEqCi | BinaryOp::String { .. } | BinaryOp::MatchesRegex => {
            let symbol = datascript_compiler::operators::op_symbol(op);
            return Err(not_implemented(span, &format!("the `{symbol}` operator")));
        }
    })
}

/// A plan this engine cannot run yet (M1-09).
fn not_implemented(span: Span, what: &str) -> Diagnostic {
    Diagnostic::error(
        DS9001,
        format!("{what} is not implemented in the DataFusion engine yet"),
    )
    .primary(span, "not supported by this version of DataScript")
}

/// DS4002: DataFusion failed. Its message goes in a note (DIAGNOSTICS.md rule 10). A value
/// out of range at the scan is DS4003 instead.
fn engine_error(span: Span, error: &DataFusionError) -> Diagnostic {
    if let Some(out_of_range) = checked::find_out_of_range(error) {
        return Diagnostic::error(
            DS4003,
            format!("a value in column `{}` is too large", out_of_range.column),
        )
        .primary(span, "while reading this source")
        .note(format!(
            "the file has {}, but the largest `long` is {}",
            out_of_range.value,
            i64::MAX
        ));
    }
    Diagnostic::error(DS4002, "the query failed while running")
        .primary(span, "this step failed")
        .note(format!("DataFusion said: {}", error.strip_backtrace()))
}

/// `DS9001` for a plan node this backend cannot lower yet (M1-09).
fn cannot_run_yet(plan: &Plan) -> Diagnostic {
    let what = match &plan.node {
        PlanNode::Aggregate { .. } => "`summarize` and `count` cannot run yet",
        PlanNode::Sort { .. } => "`sort` cannot run yet",
        PlanNode::TopN { .. } => "`top` cannot run yet",
        PlanNode::Distinct { .. } => "`distinct` cannot run yet",
        PlanNode::Join { .. } => "`join` cannot run yet",
        PlanNode::Union { .. } => "`union` cannot run yet",
        PlanNode::Values { .. } => "`getschema` cannot run yet",
        _ => "this operator cannot run yet",
    };
    Diagnostic::error(DS9001, what)
        .primary(plan.span, "this version can check it, but not run it")
        .note(
            "this version can run `where`, `project`, `project-away`, `project-rename`, \
             `extend` and `take`",
        )
}

fn internal(span: Span, message: &str) -> Diagnostic {
    Diagnostic::error(DS9001, format!("internal error: {message}"))
        .primary(span, "while running this")
        .note("this is a bug in DataScript; please report it")
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
}
