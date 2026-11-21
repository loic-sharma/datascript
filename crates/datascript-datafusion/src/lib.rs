//! Apache DataFusion backend for DataScript.
//!
//! [`DataFusionEngine`] implements [`Engine`]: it reads source schemas, lowers the IR to a
//! DataFusion `LogicalPlan`, and runs it. The walking skeleton (M1-15) handles CSV files and the
//! `Scan`, `Filter`, `Project` and `Limit` nodes; M1-08 and M1-09 build it out.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow_schema::{Field, Schema, SchemaRef};
use datafusion::common::{Column, ScalarValue};
use datafusion::datasource::file_format::csv::CsvFormat;
use datafusion::datasource::listing::{
    ListingOptions, ListingTable, ListingTableConfig, ListingTableUrl,
};
use datafusion::datasource::provider_as_source;
use datafusion::error::DataFusionError;
use datafusion::logical_expr::{LogicalPlan, LogicalPlanBuilder, Operator, binary_expr, cast};
use datafusion::prelude::{Expr as DfExpr, SessionConfig, SessionContext, lit};
use datascript_compiler::arrow::{from_arrow_source, to_arrow, to_arrow_type};
use datascript_compiler::ir::{BinaryOp, Expr, ExprKind, Plan, PlanNode, QueryPlan, Value};
use datascript_compiler::{Capabilities, SourceError, SourceRef};
use datascript_diagnostics::codes::{DS4002, DS9001};
use datascript_diagnostics::{Diagnostic, Span};
use datascript_engine::{Engine, ExecContext, ResultStream};
use futures::StreamExt;

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

/// The DataFusion engine.
#[derive(Debug)]
pub struct DataFusionEngine {
    config: DataFusionConfig,
    capabilities: Capabilities,
}

impl DataFusionEngine {
    pub fn new(config: DataFusionConfig) -> Self {
        DataFusionEngine {
            config,
            capabilities: Capabilities::full(),
        }
    }

    fn context(&self) -> SessionContext {
        let config = SessionConfig::new()
            .with_target_partitions(self.config.target_partitions)
            .with_batch_size(self.config.batch_size);
        SessionContext::new_with_config(config)
    }
}

impl Default for DataFusionEngine {
    fn default() -> Self {
        Self::new(DataFusionConfig::default())
    }
}

#[async_trait::async_trait]
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
        let table = CsvTable::open(&self.context(), source, ctx).await?;
        Ok(table.widened)
    }

    async fn execute(
        &self,
        plan: &QueryPlan,
        ctx: &ExecContext,
    ) -> Result<ResultStream, Diagnostic> {
        let session = self.context();
        let mut lowering = Lowering {
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

/// A CSV file registered as a `ListingTable`, with its schema as read and as widened.
struct CsvTable {
    url: ListingTableUrl,
    options: ListingOptions,
    raw: SchemaRef,
    widened: SchemaRef,
}

impl CsvTable {
    async fn open(
        session: &SessionContext,
        source: &SourceRef,
        ctx: &ExecContext,
    ) -> Result<CsvTable, SourceError> {
        let SourceRef::Path(path) = source;
        let path = resolve(path, &ctx.base_dir);
        if !path.is_file() {
            return Err(SourceError::NotFound {
                nearby: nearby_files(&path),
            });
        }
        let unreadable = |e: &dyn std::fmt::Display| SourceError::Unreadable {
            reason: e.to_string(),
        };
        let url = ListingTableUrl::parse(path.to_string_lossy()).map_err(|e| unreadable(&e))?;
        let options = ListingOptions::new(Arc::new(CsvFormat::default().with_has_header(true)))
            .with_file_extension(file_extension(&path));
        let raw = options
            .infer_schema(&session.state(), &url)
            .await
            .map_err(|e| unreadable(&e))?;
        let widened = widen(&raw);
        Ok(CsvTable {
            url,
            options,
            raw,
            widened,
        })
    }
}

/// The schema with each column widened to its DataScript type's canonical Arrow type
/// (LANGUAGE.md §5.1). Columns with no DataScript type keep their Arrow type.
fn widen(raw: &Schema) -> SchemaRef {
    let fields: Vec<Field> = raw
        .fields()
        .iter()
        .map(|field| match from_arrow_source(field) {
            Ok(ty) => to_arrow(field.name(), ty),
            Err(_) => field.as_ref().clone(),
        })
        .collect();
    Arc::new(Schema::new(fields))
}

fn resolve(path: &str, base_dir: &Path) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_dir.join(path)
    }
}

/// The extension as `ListingOptions` wants it (with the dot); empty if there is none.
fn file_extension(path: &Path) -> String {
    path.extension()
        .map(|ext| format!(".{}", ext.to_string_lossy()))
        .unwrap_or_default()
}

/// File names in the directory of `path`, sorted, for a did-you-mean.
fn nearby_files(path: &Path) -> Vec<String> {
    let Some(dir) = path.parent() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    names.sort();
    names
}

/// Lowers IR plans to DataFusion logical plans.
struct Lowering<'a> {
    session: &'a SessionContext,
    ctx: &'a ExecContext,
    /// Scans so far; each gets its own table name.
    tables: usize,
}

impl Lowering<'_> {
    async fn plan(&mut self, plan: &Plan) -> Result<LogicalPlan, Diagnostic> {
        // The IR is a chain for now, so walk to the scan and build upwards.
        let mut chain = vec![plan];
        while let Some(input) = chain.last().and_then(|p| p.input()) {
            chain.push(input);
        }
        let mut builder: Option<LogicalPlanBuilder> = None;
        for node in chain.into_iter().rev() {
            let span = node.span;
            let fail = |e: DataFusionError| engine_error(span, &e);
            builder = Some(match (&node.node, builder) {
                (PlanNode::Scan { source }, _) => self.scan(source, node).await?,
                (PlanNode::Filter { predicate, .. }, Some(b)) => {
                    b.filter(lower_expr(predicate)).map_err(fail)?
                }
                (PlanNode::Project { columns, .. }, Some(b)) => b
                    .project(
                        columns
                            .iter()
                            .map(|item| lower_expr(&item.expr).alias(&item.name)),
                    )
                    .map_err(fail)?,
                (PlanNode::Limit { count, .. }, Some(b)) => {
                    let count = usize::try_from(*count).unwrap_or(usize::MAX);
                    b.limit(0, Some(count)).map_err(fail)?
                }
                (_, None) => return Err(internal(span, "a plan node has no input")),
            });
        }
        let builder = builder.ok_or_else(|| internal(plan.span, "the plan is empty"))?;
        builder.build().map_err(|e| engine_error(plan.span, &e))
    }

    /// Scans a CSV file and casts each column to the canonical type of `plan.schema`.
    async fn scan(
        &mut self,
        source: &SourceRef,
        plan: &Plan,
    ) -> Result<LogicalPlanBuilder, Diagnostic> {
        let span = plan.span;
        let table = CsvTable::open(self.session, source, self.ctx)
            .await
            .map_err(|e| {
                // The schema was read during checking, so the file changed since.
                internal(
                    span,
                    &format!("the source changed while the query ran: {e:?}"),
                )
            })?;
        let config = ListingTableConfig::new(table.url)
            .with_listing_options(table.options)
            .with_schema(table.raw.clone());
        let fail = |e: DataFusionError| engine_error(span, &e);
        let provider = ListingTable::try_new(config).map_err(fail)?;
        self.tables += 1;
        let name = format!("source{}", self.tables);
        let builder = LogicalPlanBuilder::scan(name, provider_as_source(Arc::new(provider)), None)
            .map_err(fail)?;
        let columns = plan.schema.fields().iter().map(|field| {
            let column = DfExpr::Column(Column::new_unqualified(field.name()));
            let read_as = table
                .raw
                .field_with_name(field.name())
                .map(|f| f.data_type());
            if read_as.is_ok_and(|t| t == field.data_type()) {
                column
            } else {
                cast(column, field.data_type().clone()).alias(field.name())
            }
        });
        builder.project(columns).map_err(fail)
    }
}

fn lower_expr(expr: &Expr) -> DfExpr {
    match &expr.kind {
        // `Column::new_unqualified`, not `col()`, which lowercases unquoted names.
        ExprKind::Column(name) => DfExpr::Column(Column::new_unqualified(name)),
        ExprKind::Literal(value) => lit(match value {
            Value::Bool(b) => ScalarValue::Boolean(Some(*b)),
            Value::Long(n) => ScalarValue::Int64(Some(*n)),
            Value::Real(x) => ScalarValue::Float64(Some(*x)),
            Value::String(s) => ScalarValue::Utf8(Some(s.clone())),
        }),
        ExprKind::Binary { op, left, right } => {
            binary_expr(lower_expr(left), lower_op(*op), lower_expr(right))
        }
        ExprKind::Cast(inner) => cast(lower_expr(inner), to_arrow_type(expr.ty)),
    }
}

/// The binder only lets comparisons through so far (M1-15); M1-09 maps the rest.
fn lower_op(op: BinaryOp) -> Operator {
    match op {
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
        // Never reached: the binder reports these as not implemented.
        BinaryOp::EqCi | BinaryOp::NotEqCi | BinaryOp::String { .. } | BinaryOp::MatchesRegex => {
            Operator::Eq
        }
    }
}

/// DS4002: DataFusion failed. Its message goes in a note (DIAGNOSTICS.md rule 10).
fn engine_error(span: Span, error: &DataFusionError) -> Diagnostic {
    Diagnostic::error(DS4002, "the query failed while running")
        .primary(span, "this step failed")
        .note(format!("DataFusion said: {}", error.strip_backtrace()))
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

    fn fixtures() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/data")
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
}
