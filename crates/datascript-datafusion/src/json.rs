//! Custom JSON and NDJSON [`FileFormat`] and [`FileSource`] for DataScript (M2-33).
//!
//! Unlike DataFusion's built-in `JsonFormat` (which delegates to `arrow-json`'s strict recursive
//! schema inference), this reader infers **top-level columns only**:
//!
//! - Consistent scalar columns infer `Boolean`, `Int64`, `Float64`, or `Utf8`.
//! - `Int64` + `Float64` widens to `Float64`.
//! - Nested values (objects and arrays), mixed-type columns (including mixed scalars such as
//!   `Int64` + `Utf8`), integers outside `i64`, numbers whose magnitude overflows `f64`, and
//!   all-`null` columns become `Utf8` with the `arrow.json` extension type, storing raw JSON text
//!   (with insignificant whitespace removed from objects and arrays).
//! - Both NDJSON and top-level JSON arrays (`[{...}, ...]`) stream without buffering the whole
//!   file, skip a leading UTF-8 BOM, and auto-detect array vs. NDJSON per file when requested.
//! - Files are always read whole from byte 0 (`supports_repartitioning() == false`) so arrays and
//!   1-based line numbers work accurately even on large files.
//! - Projection pushdown builds Arrow arrays only for projected columns while still type-checking
//!   every column in the target schema, so whether a file reads never depends on the query.

use std::borrow::Cow;
use std::collections::HashMap;
use std::error::Error;
use std::fmt;
#[cfg(any(not(target_arch = "wasm32"), test))]
use std::io::Read;
use std::sync::Arc;

use arrow_schema::extension::EXTENSION_TYPE_NAME_KEY;
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use datafusion::arrow::array::{
    ArrayRef, BooleanBuilder, Float64Builder, Int64Builder, RecordBatch, RecordBatchOptions,
    StringBuilder,
};
use datafusion::catalog::Session;
use datafusion::common::tree_node::TreeNodeRecursion;
use datafusion::common::{GetExt, Result as DfResult, Statistics};
use datafusion::datasource::file_format::FileFormat;
use datafusion::datasource::file_format::file_compression_type::FileCompressionType;
use datafusion::datasource::listing::PartitionedFile;
use datafusion::datasource::physical_plan::{
    FileOpenFuture, FileOpener, FileScanConfig, FileScanConfigBuilder, FileSource,
};
use datafusion::datasource::projection::{ProjectionOpener, SplitProjection};
use datafusion::datasource::source::DataSourceExec;
use datafusion::datasource::table_schema::TableSchema;
use datafusion::error::DataFusionError;
use datafusion::object_store::path::Path as ObjectPath;
use datafusion::object_store::{
    GetOptions, GetResultPayload, ObjectMeta, ObjectStore, ObjectStoreExt,
};
use datafusion::physical_expr::projection::ProjectionExprs;
use datafusion::physical_plan::metrics::ExecutionPlanMetricsSet;
use datafusion::physical_plan::{ExecutionPlan, PhysicalExpr, apply_expression_roots};
use datascript_compiler::arrow::JSON_EXTENSION;
use futures::{StreamExt, TryStreamExt};

/// Chunk size for synchronous `Read` polling (64 KiB).
const READ_CHUNK_SIZE: usize = 64 * 1024;

/// How a JSON file's top-level layout is chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JsonLayout {
    /// Detect per file: `[` as the first non-whitespace byte (after an optional UTF-8 BOM) selects
    /// a JSON array, otherwise NDJSON.
    Auto,
    /// One JSON object per line (`.ndjson`, `.jsonl`, `format = "ndjson"`).
    Ndjson,
    /// A single top-level JSON array of objects (`format = "array"`).
    Array,
}

/// The JSON value kind observed during schema inference or decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JsonValueKind {
    Null,
    Boolean,
    Integer,
    /// An integer outside the `i64` range (for example a `u64` ID).
    OutOfRangeInteger,
    Float,
    /// A number whose magnitude overflows `f64` (for example `1e400`).
    OverflowFloat,
    String,
    Array,
    Object,
}

impl fmt::Display for JsonValueKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            JsonValueKind::Null => "null",
            JsonValueKind::Boolean => "boolean",
            JsonValueKind::Integer => "integer",
            JsonValueKind::OutOfRangeInteger => "out-of-range integer",
            JsonValueKind::Float => "number",
            JsonValueKind::OverflowFloat => "out-of-range number",
            JsonValueKind::String => "string",
            JsonValueKind::Array => "array",
            JsonValueKind::Object => "object",
        };
        f.write_str(name)
    }
}

/// Why reading a JSON/NDJSON file failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum JsonReadErrorKind {
    /// Malformed JSON syntax.
    Syntax { detail: String },
    /// A top-level record is valid JSON, but not a JSON object.
    NonObject { found: JsonValueKind },
    /// A value does not match the target column's Arrow type.
    TypeMismatch {
        column: String,
        expected: DataType,
        found: JsonValueKind,
    },
}

/// Structured error carried through `DataFusionError::External` so diagnostics (M2-33, M2-34,
/// M2-12) can inspect the failing file, location, column, and failure kind without parsing strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JsonReadError {
    /// File path (from `PartitionedFile` / `ObjectMeta` or the caller-supplied display path).
    pub file: String,
    /// 1-based line number where the failing record or syntax error starts.
    pub line: usize,
    /// 1-based record index for JSON arrays; `None` for NDJSON.
    pub record: Option<usize>,
    /// True when reading as NDJSON and the file's first non-whitespace byte (after an optional
    /// UTF-8 BOM) is `[`.
    pub looks_like_json_array: bool,
    pub kind: JsonReadErrorKind,
}

impl JsonReadError {
    pub(crate) fn into_datafusion(self) -> DataFusionError {
        DataFusionError::External(Box::new(self))
    }
}

impl fmt::Display for JsonReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            JsonReadErrorKind::Syntax { detail } => match self.record {
                Some(r) => write!(
                    f,
                    "record {r} (line {}) is not valid JSON: {detail}",
                    self.line
                ),
                None => write!(f, "line {} is not valid JSON: {detail}", self.line),
            },
            JsonReadErrorKind::NonObject { found } => match self.record {
                Some(r) => write!(
                    f,
                    "record {r} (line {}) is not a JSON object (found {found})",
                    self.line
                ),
                None => write!(f, "line {} is not a JSON object (found {found})", self.line),
            },
            JsonReadErrorKind::TypeMismatch {
                column,
                expected,
                found,
            } => match self.record {
                Some(r) => write!(
                    f,
                    "column `{column}` at record {r} (line {}) expected {expected}, found {found}",
                    self.line
                ),
                None => write!(
                    f,
                    "column `{column}` at line {} expected {expected}, found {found}",
                    self.line
                ),
            },
        }
    }
}

impl Error for JsonReadError {}

// ---------------------------------------------------------------------------------------------
// Schema inference lattice
// ---------------------------------------------------------------------------------------------

/// The joined column type observed while sampling a JSON/NDJSON source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InferredType {
    /// Only JSON `null` seen so far (bottom of the join lattice).
    Null,
    Boolean,
    Int64,
    Float64,
    Utf8,
    /// Nested value, mixed types, out-of-`i64` integer, or `f64`-overflowing number.
    Json,
}

impl InferredType {
    fn from_kind(kind: JsonValueKind) -> Self {
        match kind {
            JsonValueKind::Null => InferredType::Null,
            JsonValueKind::Boolean => InferredType::Boolean,
            JsonValueKind::Integer => InferredType::Int64,
            JsonValueKind::Float => InferredType::Float64,
            JsonValueKind::String => InferredType::Utf8,
            JsonValueKind::OutOfRangeInteger
            | JsonValueKind::OverflowFloat
            | JsonValueKind::Array
            | JsonValueKind::Object => InferredType::Json,
        }
    }

    pub(crate) fn join(self, other: Self) -> Self {
        match (self, other) {
            (InferredType::Null, t) | (t, InferredType::Null) => t,
            (a, b) if a == b => a,
            (InferredType::Int64, InferredType::Float64)
            | (InferredType::Float64, InferredType::Int64) => InferredType::Float64,
            _ => InferredType::Json,
        }
    }

    pub(crate) fn into_field(self, name: &str) -> Field {
        match self {
            InferredType::Boolean => Field::new(name, DataType::Boolean, true),
            InferredType::Int64 => Field::new(name, DataType::Int64, true),
            InferredType::Float64 => Field::new(name, DataType::Float64, true),
            InferredType::Utf8 => Field::new(name, DataType::Utf8, true),
            // An all-null JSON column becomes `Utf8` + `arrow.json` so any later value fits
            // losslessly rather than failing a scalar type check.
            InferredType::Json | InferredType::Null => arrow_json_field(name),
        }
    }
}

/// A nullable `Utf8` Arrow field carrying the `arrow.json` canonical extension type.
pub(crate) fn arrow_json_field(name: &str) -> Field {
    Field::new(name, DataType::Utf8, true).with_metadata(
        [(
            EXTENSION_TYPE_NAME_KEY.to_string(),
            JSON_EXTENSION.to_string(),
        )]
        .into_iter()
        .collect(),
    )
}

/// Top-level columns sampled from one or more JSON/NDJSON files, in order of first appearance.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct SampledSchema {
    columns: Vec<(String, InferredType)>,
    index: HashMap<String, usize>,
}

impl SampledSchema {
    fn observe(&mut self, name: &str, ty: InferredType) {
        if let Some(&idx) = self.index.get(name) {
            self.columns[idx].1 = self.columns[idx].1.join(ty);
        } else {
            let idx = self.columns.len();
            self.index.insert(name.to_string(), idx);
            self.columns.push((name.to_string(), ty));
        }
    }

    /// Joins `other`'s sampled columns into `self`, preserving order of first appearance.
    pub(crate) fn merge(&mut self, other: SampledSchema) {
        for (name, ty) in other.columns {
            self.observe(&name, ty);
        }
    }

    /// Converts the joined sample into an Arrow schema.
    pub(crate) fn into_schema(self) -> SchemaRef {
        let fields: Vec<Field> = self
            .columns
            .into_iter()
            .map(|(name, ty)| ty.into_field(&name))
            .collect();
        Arc::new(Schema::new(fields))
    }

    /// Samples one top-level JSON object record into `self` (duplicate keys: last wins).
    fn observe_record(&mut self, bytes: &[u8]) -> Result<(), JsonReadErrorKind> {
        let text = std::str::from_utf8(bytes).map_err(|_| JsonReadErrorKind::Syntax {
            detail: "invalid UTF-8".into(),
        })?;
        // Resolve duplicate keys within one top-level object as last-wins while preserving the
        // order of first appearance.
        let mut row: Vec<(String, InferredType)> = Vec::new();
        let mut row_idx: HashMap<String, usize> = HashMap::new();
        parse_top_level_object(text, |key, value| {
            let ty = InferredType::from_kind(value.kind());
            if let Some(&idx) = row_idx.get(key.as_ref()) {
                row[idx].1 = ty;
            } else {
                let name = key.into_owned();
                let idx = row.len();
                row_idx.insert(name.clone(), idx);
                row.push((name, ty));
            }
        })?;
        for (name, ty) in row {
            self.observe(&name, ty);
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// DataFusion FileFormat, FileSource, FileOpener
// ---------------------------------------------------------------------------------------------

/// Custom DataFusion [`FileFormat`] for DataScript JSON/NDJSON sources.
#[derive(Debug, Clone)]
pub(crate) struct DataScriptJsonFormat {
    layout: JsonLayout,
    infer_rows: usize,
    compression: FileCompressionType,
}

impl DataScriptJsonFormat {
    pub(crate) fn new(
        layout: JsonLayout,
        infer_rows: usize,
        compression: FileCompressionType,
    ) -> Self {
        Self {
            layout,
            infer_rows,
            compression,
        }
    }
}

#[async_trait::async_trait]
impl FileFormat for DataScriptJsonFormat {
    fn get_ext(&self) -> String {
        "json".to_string()
    }

    fn get_ext_with_compression(
        &self,
        file_compression_type: &FileCompressionType,
    ) -> DfResult<String> {
        Ok(format!(
            "{}{}",
            self.get_ext(),
            file_compression_type.get_ext()
        ))
    }

    fn compression_type(&self) -> Option<FileCompressionType> {
        Some(self.compression)
    }

    async fn infer_schema(
        &self,
        _state: &dyn Session,
        store: &Arc<dyn ObjectStore>,
        objects: &[ObjectMeta],
    ) -> DfResult<SchemaRef> {
        let mut combined = SampledSchema::default();
        for object in objects {
            let sample = sample_object(
                store,
                &object.location,
                object.location.as_ref(),
                self.layout,
                self.compression,
                self.infer_rows,
            )
            .await?;
            combined.merge(sample);
        }
        Ok(combined.into_schema())
    }

    async fn infer_stats(
        &self,
        _state: &dyn Session,
        _store: &Arc<dyn ObjectStore>,
        table_schema: SchemaRef,
        _object: &ObjectMeta,
    ) -> DfResult<Statistics> {
        Ok(Statistics::new_unknown(&table_schema))
    }

    async fn create_physical_plan(
        &self,
        _state: &dyn Session,
        conf: FileScanConfig,
    ) -> DfResult<Arc<dyn ExecutionPlan>> {
        let conf = FileScanConfigBuilder::from(conf)
            .with_file_compression_type(self.compression)
            .build();
        Ok(DataSourceExec::from_data_source(conf))
    }

    fn file_source(&self, table_schema: TableSchema) -> Arc<dyn FileSource> {
        Arc::new(DataScriptJsonSource::new(table_schema, self.layout))
    }
}

/// Custom DataFusion [`FileSource`] for DataScript JSON/NDJSON sources.
#[derive(Debug, Clone)]
pub(crate) struct DataScriptJsonSource {
    table_schema: TableSchema,
    batch_size: Option<usize>,
    metrics: ExecutionPlanMetricsSet,
    projection: SplitProjection,
    layout: JsonLayout,
}

impl DataScriptJsonSource {
    pub(crate) fn new(table_schema: impl Into<TableSchema>, layout: JsonLayout) -> Self {
        let table_schema = table_schema.into();
        Self {
            projection: SplitProjection::unprojected(&table_schema),
            table_schema,
            batch_size: None,
            metrics: ExecutionPlanMetricsSet::new(),
            layout,
        }
    }
}

impl FileSource for DataScriptJsonSource {
    fn create_file_opener(
        &self,
        object_store: Arc<dyn ObjectStore>,
        base_config: &FileScanConfig,
        _partition: usize,
    ) -> DfResult<Arc<dyn FileOpener>> {
        let file_schema = Arc::clone(self.table_schema.file_schema());
        let opener: Arc<dyn FileOpener> = Arc::new(DataScriptJsonOpener {
            batch_size: self
                .batch_size
                .expect("Batch size must be set before creating opener"),
            full_schema: Arc::clone(&file_schema),
            projected_indices: self.projection.file_indices.clone(),
            compression: base_config.file_compression_type,
            object_store,
            layout: self.layout,
        });
        ProjectionOpener::try_new(self.projection.clone(), opener, &file_schema)
    }

    fn table_schema(&self) -> &TableSchema {
        &self.table_schema
    }

    fn with_batch_size(&self, batch_size: usize) -> Arc<dyn FileSource> {
        let mut conf = self.clone();
        conf.batch_size = Some(batch_size);
        Arc::new(conf)
    }

    fn try_pushdown_projection(
        &self,
        projection: &ProjectionExprs,
    ) -> DfResult<Option<Arc<dyn FileSource>>> {
        let mut source = self.clone();
        let new_projection = self.projection.source.try_merge(projection)?;
        source.projection = SplitProjection::new(self.table_schema.file_schema(), &new_projection);
        Ok(Some(Arc::new(source)))
    }

    fn projection(&self) -> Option<&ProjectionExprs> {
        Some(&self.projection.source)
    }

    fn metrics(&self) -> &ExecutionPlanMetricsSet {
        &self.metrics
    }

    fn file_type(&self) -> &str {
        "json"
    }

    /// Always read each file whole from byte 0: JSON arrays cannot be split mid-file, and NDJSON
    /// needs 1-based line numbers from the start of the file. Parallelism across the files of a
    /// glob is unaffected.
    fn supports_repartitioning(&self) -> bool {
        false
    }

    fn apply_expressions(
        &self,
        f: &mut dyn FnMut(&Arc<dyn PhysicalExpr>) -> DfResult<TreeNodeRecursion>,
    ) -> DfResult<TreeNodeRecursion> {
        apply_expression_roots(self.projection.source.iter(), f)
    }
}

struct DataScriptJsonOpener {
    batch_size: usize,
    full_schema: SchemaRef,
    projected_indices: Vec<usize>,
    compression: FileCompressionType,
    object_store: Arc<dyn ObjectStore>,
    layout: JsonLayout,
}

impl FileOpener for DataScriptJsonOpener {
    fn open(&self, partitioned_file: PartitionedFile) -> DfResult<FileOpenFuture> {
        if partitioned_file.range.is_some() {
            return Err(DataFusionError::NotImplemented(
                "DataScript JSON reader does not support byte-range scanning".into(),
            ));
        }
        let store = Arc::clone(&self.object_store);
        let full_schema = Arc::clone(&self.full_schema);
        let projected_indices = self.projected_indices.clone();
        let batch_size = self.batch_size;
        let compression = self.compression;
        let layout = self.layout;

        Ok(Box::pin(async move {
            let location = partitioned_file.object_meta.location.clone();
            let file_label = location.to_string();
            let result = store.get_opts(&location, GetOptions::default()).await?;

            match result.payload {
                #[cfg(not(target_arch = "wasm32"))]
                GetResultPayload::File(file, _) => {
                    let reader = compression.convert_read(file)?;
                    let decoder =
                        BatchDecoder::try_new(full_schema, &projected_indices, batch_size)?;
                    let scanner = RecordScanner::new(file_label, layout);
                    let iter = SyncBatchIter {
                        reader,
                        scanner,
                        decoder,
                        read_buf: vec![0u8; READ_CHUNK_SIZE],
                        done: false,
                    };
                    Ok(futures::stream::iter(iter).boxed())
                }
                GetResultPayload::Stream(s) => {
                    let s = s.map_err(DataFusionError::from);
                    let decompressed = compression.convert_stream(s.boxed())?;
                    let decoder =
                        BatchDecoder::try_new(full_schema, &projected_indices, batch_size)?;
                    let scanner = RecordScanner::new(file_label, layout);
                    let state = AsyncBatchState {
                        stream: decompressed,
                        scanner,
                        decoder,
                        done: false,
                    };
                    let batches = futures::stream::try_unfold(state, |mut st| async move {
                        if st.done {
                            return Ok(None);
                        }
                        loop {
                            match st.scanner.next_record() {
                                Ok(Step::Record {
                                    bytes,
                                    line,
                                    record,
                                }) => {
                                    if let Err(kind) = st.decoder.push_record(bytes) {
                                        return Err(JsonReadError {
                                            file: st.scanner.file.clone(),
                                            line,
                                            record,
                                            looks_like_json_array: st.scanner.looks_like_json_array,
                                            kind,
                                        }
                                        .into_datafusion());
                                    }
                                    if st.decoder.is_full() {
                                        let batch = st.decoder.flush()?;
                                        return Ok(Some((batch, st)));
                                    }
                                }
                                Ok(Step::NeedMore) => match st.stream.next().await {
                                    Some(Ok(chunk)) => st.scanner.feed(chunk.as_ref()),
                                    Some(Err(e)) => return Err(e),
                                    None => st.scanner.set_eof(),
                                },
                                Ok(Step::Eof) => {
                                    st.done = true;
                                    if st.decoder.row_count() > 0 {
                                        let batch = st.decoder.flush()?;
                                        return Ok(Some((batch, st)));
                                    }
                                    return Ok(None);
                                }
                                Err(err) => return Err(err.into_datafusion()),
                            }
                        }
                    });
                    Ok(batches.boxed())
                }
            }
        }))
    }
}

/// Samples up to `infer_rows` records from an object in `store`.
pub(crate) async fn sample_object(
    store: &Arc<dyn ObjectStore>,
    location: &ObjectPath,
    file_label: &str,
    layout: JsonLayout,
    compression: FileCompressionType,
    infer_rows: usize,
) -> DfResult<SampledSchema> {
    let result = store.get(location).await?;
    match result.payload {
        #[cfg(not(target_arch = "wasm32"))]
        GetResultPayload::File(file, _) => {
            let reader = compression.convert_read(file)?;
            sample_from_read(reader, file_label, layout, infer_rows)
        }
        GetResultPayload::Stream(s) => {
            let s = s.map_err(DataFusionError::from);
            let mut decompressed = compression.convert_stream(s.boxed())?;
            let mut scanner = RecordScanner::new(file_label.to_string(), layout);
            let mut sampled = SampledSchema::default();
            let mut count = 0usize;
            while count < infer_rows {
                match scanner.next_record() {
                    Ok(Step::Record {
                        bytes,
                        line,
                        record,
                    }) => {
                        if let Err(kind) = sampled.observe_record(bytes) {
                            return Err(JsonReadError {
                                file: scanner.file.clone(),
                                line,
                                record,
                                looks_like_json_array: scanner.looks_like_json_array,
                                kind,
                            }
                            .into_datafusion());
                        }
                        count += 1;
                    }
                    Ok(Step::NeedMore) => match decompressed.next().await {
                        Some(Ok(chunk)) => scanner.feed(&chunk),
                        Some(Err(e)) => return Err(e),
                        None => scanner.set_eof(),
                    },
                    Ok(Step::Eof) => break,
                    Err(err) => return Err(err.into_datafusion()),
                }
            }
            Ok(sampled)
        }
    }
}

/// Samples up to `infer_rows` records from any synchronous `Read`.
#[cfg(any(not(target_arch = "wasm32"), test))]
fn sample_from_read<R: Read>(
    mut reader: R,
    file_label: &str,
    layout: JsonLayout,
    infer_rows: usize,
) -> DfResult<SampledSchema> {
    let mut scanner = RecordScanner::new(file_label.to_string(), layout);
    let mut sampled = SampledSchema::default();
    let mut read_buf = vec![0u8; READ_CHUNK_SIZE];
    let mut count = 0usize;
    while count < infer_rows {
        match scanner.next_record() {
            Ok(Step::Record {
                bytes,
                line,
                record,
            }) => {
                if let Err(kind) = sampled.observe_record(bytes) {
                    return Err(JsonReadError {
                        file: scanner.file.clone(),
                        line,
                        record,
                        looks_like_json_array: scanner.looks_like_json_array,
                        kind,
                    }
                    .into_datafusion());
                }
                count += 1;
            }
            Ok(Step::NeedMore) => {
                let n = reader.read(&mut read_buf)?;
                if n == 0 {
                    scanner.set_eof();
                } else {
                    scanner.feed(&read_buf[..n]);
                }
            }
            Ok(Step::Eof) => break,
            Err(err) => return Err(err.into_datafusion()),
        }
    }
    Ok(sampled)
}

#[cfg(not(target_arch = "wasm32"))]
struct SyncBatchIter<R: Read> {
    reader: R,
    scanner: RecordScanner,
    decoder: BatchDecoder,
    read_buf: Vec<u8>,
    done: bool,
}

#[cfg(not(target_arch = "wasm32"))]
impl<R: Read> Iterator for SyncBatchIter<R> {
    type Item = DfResult<RecordBatch>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        loop {
            match self.scanner.next_record() {
                Ok(Step::Record {
                    bytes,
                    line,
                    record,
                }) => {
                    if let Err(kind) = self.decoder.push_record(bytes) {
                        self.done = true;
                        return Some(Err(JsonReadError {
                            file: self.scanner.file.clone(),
                            line,
                            record,
                            looks_like_json_array: self.scanner.looks_like_json_array,
                            kind,
                        }
                        .into_datafusion()));
                    }
                    if self.decoder.is_full() {
                        return Some(self.decoder.flush());
                    }
                }
                Ok(Step::NeedMore) => match self.reader.read(&mut self.read_buf) {
                    Ok(0) => self.scanner.set_eof(),
                    Ok(n) => self.scanner.feed(&self.read_buf[..n]),
                    Err(e) => {
                        self.done = true;
                        return Some(Err(DataFusionError::IoError(e)));
                    }
                },
                Ok(Step::Eof) => {
                    self.done = true;
                    if self.decoder.row_count() > 0 {
                        return Some(self.decoder.flush());
                    }
                    return None;
                }
                Err(err) => {
                    self.done = true;
                    return Some(Err(err.into_datafusion()));
                }
            }
        }
    }
}

struct AsyncBatchState<S> {
    stream: S,
    scanner: RecordScanner,
    decoder: BatchDecoder,
    done: bool,
}

// ---------------------------------------------------------------------------------------------
// Streaming record scanner (BOM + Auto-detection + NDJSON / JSON Array)
// ---------------------------------------------------------------------------------------------

#[derive(Debug)]
enum Step<'a> {
    Record {
        bytes: &'a [u8],
        line: usize,
        record: Option<usize>,
    },
    NeedMore,
    Eof,
}

#[derive(Debug)]
enum ScannerPhase {
    Bom,
    Detect,
    Ndjson,
    Array(ArrayState),
    Finished,
}

#[derive(Debug)]
struct ArrayState {
    record: usize,
    record_start_line: usize,
    sub: ArraySubState,
}

impl ArrayState {
    fn new() -> Self {
        Self {
            record: 0,
            record_start_line: 1,
            sub: ArraySubState::ExpectElementOrClose,
        }
    }
}

#[derive(Debug)]
enum ArraySubState {
    ExpectElementOrClose,
    ExpectElement,
    InContainer {
        /// Expected closing delimiters (`b'}'` or `b']'`).
        stack: Vec<u8>,
        in_string: bool,
        escaped: bool,
    },
    InString {
        escaped: bool,
    },
    InPrimitive,
    AfterElement,
    Done,
}

/// Chunk-driven streaming scanner that turns raw UTF-8 JSON/NDJSON bytes into individual top-level
/// record slices with 1-based line numbers (and 1-based record indices for JSON arrays).
#[derive(Debug)]
struct RecordScanner {
    file: String,
    requested_layout: JsonLayout,
    phase: ScannerPhase,
    looks_like_json_array: bool,
    line: usize,
    buf: Vec<u8>,
    pos: usize,
    eof: bool,
    /// Scratch buffer holding the current element in JSON-array mode.
    record_buf: Vec<u8>,
}

impl RecordScanner {
    fn new(file: String, layout: JsonLayout) -> Self {
        Self {
            file,
            requested_layout: layout,
            phase: ScannerPhase::Bom,
            looks_like_json_array: false,
            line: 1,
            buf: Vec::new(),
            pos: 0,
            eof: false,
            record_buf: Vec::new(),
        }
    }

    fn feed(&mut self, chunk: &[u8]) {
        if self.pos > 0 {
            if self.pos == self.buf.len() {
                self.buf.clear();
                self.pos = 0;
            } else if self.pos >= READ_CHUNK_SIZE {
                self.buf.drain(..self.pos);
                self.pos = 0;
            }
        }
        self.buf.extend_from_slice(chunk);
    }

    fn set_eof(&mut self) {
        self.eof = true;
    }

    fn syntax_err(
        &self,
        line: usize,
        record: Option<usize>,
        detail: impl Into<String>,
    ) -> JsonReadError {
        JsonReadError {
            file: self.file.clone(),
            line,
            record,
            looks_like_json_array: self.looks_like_json_array,
            kind: JsonReadErrorKind::Syntax {
                detail: detail.into(),
            },
        }
    }

    /// Advances `self.pos` past JSON whitespace bytes (` `, `\t`, `\r`, `\n`), tracking `self.line`.
    fn skip_json_ws(&mut self) {
        while self.pos < self.buf.len() {
            match self.buf[self.pos] {
                b' ' | b'\t' | b'\r' => self.pos += 1,
                b'\n' => {
                    self.pos += 1;
                    self.line += 1;
                }
                _ => break,
            }
        }
    }

    fn next_record(&mut self) -> Result<Step<'_>, JsonReadError> {
        loop {
            match self.phase {
                ScannerPhase::Finished => return Ok(Step::Eof),
                ScannerPhase::Bom => {
                    let rem = &self.buf[self.pos..];
                    if rem.len() < 3 && !self.eof {
                        return Ok(Step::NeedMore);
                    }
                    if rem.starts_with(&[0xEF, 0xBB, 0xBF]) {
                        self.pos += 3;
                    }
                    self.phase = ScannerPhase::Detect;
                }
                ScannerPhase::Detect => {
                    self.skip_json_ws();
                    if self.pos == self.buf.len() {
                        if !self.eof {
                            return Ok(Step::NeedMore);
                        }
                        return match self.requested_layout {
                            JsonLayout::Array => {
                                self.phase = ScannerPhase::Finished;
                                Err(self.syntax_err(
                                    self.line,
                                    None,
                                    "unexpected end of input: expected `[`",
                                ))
                            }
                            JsonLayout::Auto | JsonLayout::Ndjson => {
                                self.phase = ScannerPhase::Finished;
                                Ok(Step::Eof)
                            }
                        };
                    }
                    let first = self.buf[self.pos];
                    match self.requested_layout {
                        JsonLayout::Auto => {
                            if first == b'[' {
                                self.pos += 1;
                                self.phase = ScannerPhase::Array(ArrayState::new());
                            } else {
                                self.phase = ScannerPhase::Ndjson;
                            }
                        }
                        JsonLayout::Ndjson => {
                            if first == b'[' {
                                self.looks_like_json_array = true;
                            }
                            self.phase = ScannerPhase::Ndjson;
                        }
                        JsonLayout::Array => {
                            if first == b'[' {
                                self.pos += 1;
                                self.phase = ScannerPhase::Array(ArrayState::new());
                            } else {
                                self.phase = ScannerPhase::Finished;
                                return Err(self.syntax_err(
                                    self.line,
                                    None,
                                    "expected `[` at start of JSON array",
                                ));
                            }
                        }
                    }
                }
                ScannerPhase::Ndjson => {
                    if let Some(rel) = self.buf[self.pos..].iter().position(|&b| b == b'\n') {
                        let start = self.pos;
                        let mut end = self.pos + rel;
                        self.pos = end + 1;
                        let record_line = self.line;
                        self.line += 1;
                        if end > start && self.buf[end - 1] == b'\r' {
                            end -= 1;
                        }
                        let (trim_start, trim_end) = trim_json_ws_bounds(&self.buf, start, end);
                        if trim_start == trim_end {
                            continue;
                        }
                        return Ok(Step::Record {
                            bytes: &self.buf[trim_start..trim_end],
                            line: record_line,
                            record: None,
                        });
                    }
                    if !self.eof {
                        return Ok(Step::NeedMore);
                    }
                    let start = self.pos;
                    let end = self.buf.len();
                    self.pos = end;
                    self.phase = ScannerPhase::Finished;
                    let (trim_start, trim_end) = trim_json_ws_bounds(&self.buf, start, end);
                    if trim_start == trim_end {
                        return Ok(Step::Eof);
                    }
                    return Ok(Step::Record {
                        bytes: &self.buf[trim_start..trim_end],
                        line: self.line,
                        record: None,
                    });
                }
                ScannerPhase::Array(_) => {
                    return self.next_array_record();
                }
            }
        }
    }

    fn next_array_record(&mut self) -> Result<Step<'_>, JsonReadError> {
        loop {
            // Pull `ArrayState` out temporarily so we can freely borrow `self.buf` and `self.line`.
            let ScannerPhase::Array(mut state) =
                std::mem::replace(&mut self.phase, ScannerPhase::Finished)
            else {
                unreachable!();
            };

            match state.sub {
                ArraySubState::ExpectElementOrClose | ArraySubState::ExpectElement => {
                    let allow_close = matches!(state.sub, ArraySubState::ExpectElementOrClose);
                    self.skip_json_ws();
                    if self.pos == self.buf.len() {
                        if !self.eof {
                            self.phase = ScannerPhase::Array(state);
                            return Ok(Step::NeedMore);
                        }
                        let detail = if allow_close {
                            "unexpected end of input: expected `]`"
                        } else {
                            "unexpected end of input after `,`"
                        };
                        return Err(self.syntax_err(self.line, None, detail));
                    }
                    let b = self.buf[self.pos];
                    if b == b']' {
                        if allow_close {
                            self.pos += 1;
                            state.sub = ArraySubState::Done;
                            self.phase = ScannerPhase::Array(state);
                            continue;
                        }
                        return Err(self.syntax_err(self.line, None, "trailing comma before `]`"));
                    }
                    if b == b',' {
                        return Err(self.syntax_err(self.line, None, "unexpected `,`"));
                    }
                    state.record += 1;
                    state.record_start_line = self.line;
                    self.record_buf.clear();
                    self.record_buf.push(b);
                    self.pos += 1;
                    state.sub = match b {
                        b'{' => ArraySubState::InContainer {
                            stack: vec![b'}'],
                            in_string: false,
                            escaped: false,
                        },
                        b'[' => ArraySubState::InContainer {
                            stack: vec![b']'],
                            in_string: false,
                            escaped: false,
                        },
                        b'"' => ArraySubState::InString { escaped: false },
                        _ => ArraySubState::InPrimitive,
                    };
                    self.phase = ScannerPhase::Array(state);
                }
                ArraySubState::InContainer {
                    mut stack,
                    mut in_string,
                    mut escaped,
                } => {
                    let mut finished = false;
                    while self.pos < self.buf.len() {
                        let b = self.buf[self.pos];
                        self.pos += 1;
                        if b == b'\n' {
                            self.line += 1;
                        }
                        self.record_buf.push(b);
                        if in_string {
                            if escaped {
                                escaped = false;
                            } else if b == b'\\' {
                                escaped = true;
                            } else if b == b'"' {
                                in_string = false;
                            }
                        } else {
                            match b {
                                b'"' => in_string = true,
                                b'{' => stack.push(b'}'),
                                b'[' => stack.push(b']'),
                                b'}' | b']' => {
                                    if stack.pop() != Some(b) {
                                        return Err(self.syntax_err(
                                            state.record_start_line,
                                            Some(state.record),
                                            "mismatched closing bracket",
                                        ));
                                    }
                                    if stack.is_empty() {
                                        finished = true;
                                        break;
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    if finished {
                        let line = state.record_start_line;
                        let rec = state.record;
                        state.sub = ArraySubState::AfterElement;
                        self.phase = ScannerPhase::Array(state);
                        return Ok(Step::Record {
                            bytes: &self.record_buf,
                            line,
                            record: Some(rec),
                        });
                    }
                    if self.eof {
                        return Err(self.syntax_err(
                            state.record_start_line,
                            Some(state.record),
                            "unexpected end of input",
                        ));
                    }
                    state.sub = ArraySubState::InContainer {
                        stack,
                        in_string,
                        escaped,
                    };
                    self.phase = ScannerPhase::Array(state);
                    return Ok(Step::NeedMore);
                }
                ArraySubState::InString { mut escaped } => {
                    let mut finished = false;
                    while self.pos < self.buf.len() {
                        let b = self.buf[self.pos];
                        self.pos += 1;
                        if b == b'\n' {
                            self.line += 1;
                        }
                        self.record_buf.push(b);
                        if escaped {
                            escaped = false;
                        } else if b == b'\\' {
                            escaped = true;
                        } else if b == b'"' {
                            finished = true;
                            break;
                        }
                    }
                    if finished {
                        let line = state.record_start_line;
                        let rec = state.record;
                        state.sub = ArraySubState::AfterElement;
                        self.phase = ScannerPhase::Array(state);
                        return Ok(Step::Record {
                            bytes: &self.record_buf,
                            line,
                            record: Some(rec),
                        });
                    }
                    if self.eof {
                        return Err(self.syntax_err(
                            state.record_start_line,
                            Some(state.record),
                            "unterminated string",
                        ));
                    }
                    state.sub = ArraySubState::InString { escaped };
                    self.phase = ScannerPhase::Array(state);
                    return Ok(Step::NeedMore);
                }
                ArraySubState::InPrimitive => {
                    let mut hit_delim = false;
                    while self.pos < self.buf.len() {
                        let b = self.buf[self.pos];
                        if is_json_ws(b) || b == b',' || b == b']' {
                            hit_delim = true;
                            break;
                        }
                        self.pos += 1;
                        self.record_buf.push(b);
                    }
                    if hit_delim || self.eof {
                        let line = state.record_start_line;
                        let rec = state.record;
                        state.sub = ArraySubState::AfterElement;
                        self.phase = ScannerPhase::Array(state);
                        return Ok(Step::Record {
                            bytes: &self.record_buf,
                            line,
                            record: Some(rec),
                        });
                    }
                    state.sub = ArraySubState::InPrimitive;
                    self.phase = ScannerPhase::Array(state);
                    return Ok(Step::NeedMore);
                }
                ArraySubState::AfterElement => {
                    self.skip_json_ws();
                    if self.pos == self.buf.len() {
                        if !self.eof {
                            self.phase = ScannerPhase::Array(state);
                            return Ok(Step::NeedMore);
                        }
                        return Err(self.syntax_err(
                            self.line,
                            None,
                            "unexpected end of input: expected `]`",
                        ));
                    }
                    match self.buf[self.pos] {
                        b',' => {
                            self.pos += 1;
                            state.sub = ArraySubState::ExpectElement;
                            self.phase = ScannerPhase::Array(state);
                        }
                        b']' => {
                            self.pos += 1;
                            state.sub = ArraySubState::Done;
                            self.phase = ScannerPhase::Array(state);
                        }
                        _ => {
                            return Err(self.syntax_err(self.line, None, "expected `,` or `]`"));
                        }
                    }
                }
                ArraySubState::Done => {
                    self.skip_json_ws();
                    if self.pos == self.buf.len() {
                        if !self.eof {
                            self.phase = ScannerPhase::Array(state);
                            return Ok(Step::NeedMore);
                        }
                        self.phase = ScannerPhase::Finished;
                        return Ok(Step::Eof);
                    }
                    return Err(self.syntax_err(
                        self.line,
                        None,
                        "unexpected trailing content after `]`",
                    ));
                }
            }
        }
    }
}

#[inline]
fn is_json_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n')
}

fn trim_json_ws_bounds(buf: &[u8], mut start: usize, mut end: usize) -> (usize, usize) {
    while start < end && is_json_ws(buf[start]) {
        start += 1;
    }
    while end > start && is_json_ws(buf[end - 1]) {
        end -= 1;
    }
    (start, end)
}

// ---------------------------------------------------------------------------------------------
// Schema-driven batch decoder with projection pushdown
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TargetColumnKind {
    Boolean,
    Int64,
    Float64,
    Utf8,
    ArrowJson,
}

impl TargetColumnKind {
    fn from_field(field: &Field) -> DfResult<Self> {
        let is_json_ext = field.extension_type_name() == Some(JSON_EXTENSION);
        match field.data_type() {
            DataType::Boolean => Ok(TargetColumnKind::Boolean),
            DataType::Int64 => Ok(TargetColumnKind::Int64),
            DataType::Float64 => Ok(TargetColumnKind::Float64),
            DataType::Utf8 if is_json_ext => Ok(TargetColumnKind::ArrowJson),
            DataType::Utf8 => Ok(TargetColumnKind::Utf8),
            other => Err(DataFusionError::NotImplemented(format!(
                "JSON reader does not support target column type {other}"
            ))),
        }
    }

    fn expected_data_type(self) -> DataType {
        match self {
            TargetColumnKind::Boolean => DataType::Boolean,
            TargetColumnKind::Int64 => DataType::Int64,
            TargetColumnKind::Float64 => DataType::Float64,
            TargetColumnKind::Utf8 | TargetColumnKind::ArrowJson => DataType::Utf8,
        }
    }
}

enum ColumnBuilder {
    Boolean(BooleanBuilder),
    Int64(Int64Builder),
    Float64(Float64Builder),
    Utf8(StringBuilder),
    ArrowJson(StringBuilder),
}

impl ColumnBuilder {
    fn new(kind: TargetColumnKind, batch_size: usize) -> Self {
        match kind {
            TargetColumnKind::Boolean => {
                ColumnBuilder::Boolean(BooleanBuilder::with_capacity(batch_size))
            }
            TargetColumnKind::Int64 => {
                ColumnBuilder::Int64(Int64Builder::with_capacity(batch_size))
            }
            TargetColumnKind::Float64 => {
                ColumnBuilder::Float64(Float64Builder::with_capacity(batch_size))
            }
            TargetColumnKind::Utf8 => {
                ColumnBuilder::Utf8(StringBuilder::with_capacity(batch_size, batch_size * 16))
            }
            TargetColumnKind::ArrowJson => {
                ColumnBuilder::ArrowJson(StringBuilder::with_capacity(batch_size, batch_size * 32))
            }
        }
    }

    fn append_null(&mut self) {
        match self {
            ColumnBuilder::Boolean(b) => b.append_null(),
            ColumnBuilder::Int64(b) => b.append_null(),
            ColumnBuilder::Float64(b) => b.append_null(),
            ColumnBuilder::Utf8(b) | ColumnBuilder::ArrowJson(b) => b.append_null(),
        }
    }

    fn finish(&mut self) -> ArrayRef {
        match self {
            ColumnBuilder::Boolean(b) => Arc::new(b.finish()),
            ColumnBuilder::Int64(b) => Arc::new(b.finish()),
            ColumnBuilder::Float64(b) => Arc::new(b.finish()),
            ColumnBuilder::Utf8(b) | ColumnBuilder::ArrowJson(b) => Arc::new(b.finish()),
        }
    }
}

struct TargetColumn {
    name: String,
    kind: TargetColumnKind,
}

/// Decodes JSON object records into Arrow `RecordBatch`es for a projected subset of `full_schema`
/// while validating every column in `full_schema` against the type-check table.
struct BatchDecoder {
    projected_schema: SchemaRef,
    projected_indices: Vec<usize>,
    target_columns: Vec<TargetColumn>,
    name_to_col: HashMap<String, usize>,
    builders: Vec<ColumnBuilder>,
    batch_size: usize,
    rows: usize,
    /// Reusable scratch buffer for string unescaping and JSON whitespace stripping.
    scratch: String,
}

impl BatchDecoder {
    fn try_new(
        full_schema: SchemaRef,
        projected_indices: &[usize],
        batch_size: usize,
    ) -> DfResult<Self> {
        let projected_schema = Arc::new(full_schema.project(projected_indices)?);
        let mut target_columns = Vec::with_capacity(full_schema.fields().len());
        let mut name_to_col = HashMap::with_capacity(full_schema.fields().len());
        for (i, field) in full_schema.fields().iter().enumerate() {
            let kind = TargetColumnKind::from_field(field)?;
            name_to_col.insert(field.name().clone(), i);
            target_columns.push(TargetColumn {
                name: field.name().clone(),
                kind,
            });
        }
        let builders = projected_indices
            .iter()
            .map(|&idx| ColumnBuilder::new(target_columns[idx].kind, batch_size))
            .collect();
        Ok(Self {
            projected_schema,
            projected_indices: projected_indices.to_vec(),
            target_columns,
            name_to_col,
            builders,
            batch_size: batch_size.max(1),
            rows: 0,
            scratch: String::new(),
        })
    }

    fn row_count(&self) -> usize {
        self.rows
    }

    fn is_full(&self) -> bool {
        self.rows >= self.batch_size
    }

    fn push_record(&mut self, bytes: &[u8]) -> Result<(), JsonReadErrorKind> {
        let text = std::str::from_utf8(bytes).map_err(|_| JsonReadErrorKind::Syntax {
            detail: "invalid UTF-8".into(),
        })?;
        // Resolve duplicate top-level keys in this record as last-wins.
        let mut slots: Vec<Option<ParsedValue<'_>>> = vec![None; self.target_columns.len()];
        parse_top_level_object(text, |key, val| {
            if let Some(&col_idx) = self.name_to_col.get(key.as_ref()) {
                slots[col_idx] = Some(val);
            }
        })?;

        // Check every column in the target schema (whether projected or not) before mutating any
        // builders so a type mismatch leaves the decoder state clean.
        for (col, slot) in self.target_columns.iter().zip(&slots) {
            if let Some(val) = slot {
                check_value_matches(col, *val)?;
            }
        }

        // Append values only for projected columns.
        for (builder, &full_idx) in self.builders.iter_mut().zip(&self.projected_indices) {
            match slots[full_idx] {
                None | Some(ParsedValue::Null) => builder.append_null(),
                Some(val) => match (builder, val) {
                    (ColumnBuilder::Boolean(b), ParsedValue::Bool(v)) => b.append_value(v),
                    (ColumnBuilder::Int64(b), ParsedValue::Int(v, _)) => b.append_value(v),
                    (ColumnBuilder::Float64(b), ParsedValue::Int(v, _)) => b.append_value(v as f64),
                    (ColumnBuilder::Float64(b), ParsedValue::Float(v, _)) => b.append_value(v),
                    (ColumnBuilder::Utf8(b), ParsedValue::String(raw)) => {
                        let inner = &raw[1..raw.len() - 1];
                        if !inner.contains('\\') {
                            b.append_value(inner);
                        } else {
                            self.scratch.clear();
                            unescape_validated_string_into(inner, &mut self.scratch);
                            b.append_value(&self.scratch);
                        }
                    }
                    (
                        ColumnBuilder::ArrowJson(b),
                        ParsedValue::Array(raw) | ParsedValue::Object(raw),
                    ) => {
                        self.scratch.clear();
                        strip_json_whitespace(raw, &mut self.scratch);
                        b.append_value(&self.scratch);
                    }
                    (ColumnBuilder::ArrowJson(b), other) => {
                        b.append_value(other.raw());
                    }
                    _ => unreachable!("value already validated by check_value_matches"),
                },
            }
        }

        self.rows += 1;
        Ok(())
    }

    fn flush(&mut self) -> DfResult<RecordBatch> {
        let row_count = self.rows;
        self.rows = 0;
        let arrays: Vec<ArrayRef> = self
            .builders
            .iter_mut()
            .map(ColumnBuilder::finish)
            .collect();
        let options = RecordBatchOptions::new().with_row_count(Some(row_count));
        RecordBatch::try_new_with_options(Arc::clone(&self.projected_schema), arrays, &options)
            .map_err(DataFusionError::from)
    }
}

fn check_value_matches(col: &TargetColumn, val: ParsedValue<'_>) -> Result<(), JsonReadErrorKind> {
    if matches!(val, ParsedValue::Null) {
        return Ok(());
    }
    let ok = match col.kind {
        TargetColumnKind::Boolean => matches!(val, ParsedValue::Bool(_)),
        TargetColumnKind::Int64 => matches!(val, ParsedValue::Int(_, _)),
        TargetColumnKind::Float64 => {
            matches!(val, ParsedValue::Int(_, _) | ParsedValue::Float(_, _))
        }
        TargetColumnKind::Utf8 => matches!(val, ParsedValue::String(_)),
        TargetColumnKind::ArrowJson => true,
    };
    if ok {
        Ok(())
    } else {
        Err(JsonReadErrorKind::TypeMismatch {
            column: col.name.clone(),
            expected: col.kind.expected_data_type(),
            found: val.kind(),
        })
    }
}

// ---------------------------------------------------------------------------------------------
// RFC 8259 JSON record parser and whitespace stripper
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum ParsedValue<'a> {
    Null,
    Bool(bool),
    Int(i64, &'a str),
    OutOfRangeInt(&'a str),
    Float(f64, &'a str),
    OverflowFloat(&'a str),
    /// Validated JSON string slice including its enclosing quotes.
    String(&'a str),
    /// Validated JSON array slice including `[` and `]`.
    Array(&'a str),
    /// Validated JSON object slice including `{` and `}`.
    Object(&'a str),
}

impl<'a> ParsedValue<'a> {
    fn kind(self) -> JsonValueKind {
        match self {
            ParsedValue::Null => JsonValueKind::Null,
            ParsedValue::Bool(_) => JsonValueKind::Boolean,
            ParsedValue::Int(_, _) => JsonValueKind::Integer,
            ParsedValue::OutOfRangeInt(_) => JsonValueKind::OutOfRangeInteger,
            ParsedValue::Float(_, _) => JsonValueKind::Float,
            ParsedValue::OverflowFloat(_) => JsonValueKind::OverflowFloat,
            ParsedValue::String(_) => JsonValueKind::String,
            ParsedValue::Array(_) => JsonValueKind::Array,
            ParsedValue::Object(_) => JsonValueKind::Object,
        }
    }

    fn raw(self) -> &'a str {
        match self {
            ParsedValue::Null => "null",
            ParsedValue::Bool(true) => "true",
            ParsedValue::Bool(false) => "false",
            ParsedValue::Int(_, raw)
            | ParsedValue::OutOfRangeInt(raw)
            | ParsedValue::Float(_, raw)
            | ParsedValue::OverflowFloat(raw)
            | ParsedValue::String(raw)
            | ParsedValue::Array(raw)
            | ParsedValue::Object(raw) => raw,
        }
    }
}

/// Parses one top-level JSON object record, invoking `on_entry` for every top-level key/value pair
/// in source order. If the record is valid JSON but not an object, returns
/// [`JsonReadErrorKind::NonObject`].
fn parse_top_level_object<'a>(
    src: &'a str,
    mut on_entry: impl FnMut(Cow<'a, str>, ParsedValue<'a>),
) -> Result<(), JsonReadErrorKind> {
    let mut p = JsonParser::new(src);
    p.skip_ws();
    match p.peek() {
        Some(b'{') => {
            p.pos += 1;
            p.skip_ws();
            if p.peek() == Some(b'}') {
                p.pos += 1;
                p.skip_ws();
                return p.expect_eof();
            }
            loop {
                let key = p.parse_string_decoded()?;
                p.skip_ws();
                if p.peek() != Some(b':') {
                    return Err(syntax("expected `:` after object key"));
                }
                p.pos += 1;
                p.skip_ws();
                let value = p.parse_value()?;
                on_entry(key, value);
                p.skip_ws();
                match p.peek() {
                    Some(b',') => {
                        p.pos += 1;
                        p.skip_ws();
                    }
                    Some(b'}') => {
                        p.pos += 1;
                        break;
                    }
                    Some(_) => return Err(syntax("expected `,` or `}` in object")),
                    None => return Err(syntax("unexpected end of input in object")),
                }
            }
            p.skip_ws();
            p.expect_eof()
        }
        Some(_) => {
            let value = p.parse_value()?;
            p.skip_ws();
            p.expect_eof()?;
            Err(JsonReadErrorKind::NonObject {
                found: value.kind(),
            })
        }
        None => Err(syntax("unexpected end of input")),
    }
}

struct JsonParser<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> JsonParser<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src,
            bytes: src.as_bytes(),
            pos: 0,
        }
    }

    #[inline]
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    #[inline]
    fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if is_json_ws(b) {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn expect_eof(&self) -> Result<(), JsonReadErrorKind> {
        if self.pos == self.bytes.len() {
            Ok(())
        } else {
            Err(syntax("unexpected trailing characters"))
        }
    }

    fn parse_value(&mut self) -> Result<ParsedValue<'a>, JsonReadErrorKind> {
        match self.peek() {
            Some(b'n') => {
                self.expect_literal(b"null")?;
                Ok(ParsedValue::Null)
            }
            Some(b't') => {
                self.expect_literal(b"true")?;
                Ok(ParsedValue::Bool(true))
            }
            Some(b'f') => {
                self.expect_literal(b"false")?;
                Ok(ParsedValue::Bool(false))
            }
            Some(b'"') => {
                let raw = self.validate_string()?;
                Ok(ParsedValue::String(raw))
            }
            Some(b'-' | b'0'..=b'9') => self.parse_number(),
            Some(b'[') => {
                let raw = self.validate_container()?;
                Ok(ParsedValue::Array(raw))
            }
            Some(b'{') => {
                let raw = self.validate_container()?;
                Ok(ParsedValue::Object(raw))
            }
            Some(_) => Err(syntax("unexpected character while parsing value")),
            None => Err(syntax("unexpected end of input while parsing value")),
        }
    }

    fn expect_literal(&mut self, lit: &[u8]) -> Result<(), JsonReadErrorKind> {
        if self.bytes[self.pos..].starts_with(lit) {
            self.pos += lit.len();
            Ok(())
        } else {
            Err(syntax("invalid JSON token"))
        }
    }

    fn parse_number(&mut self) -> Result<ParsedValue<'a>, JsonReadErrorKind> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek() {
            Some(b'0') => {
                self.pos += 1;
            }
            Some(b'1'..=b'9') => {
                self.pos += 1;
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.pos += 1;
                }
            }
            _ => return Err(syntax("invalid number")),
        }
        let mut is_float = false;
        if self.peek() == Some(b'.') {
            is_float = true;
            self.pos += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(syntax("invalid number: expected digit after `.`"));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            is_float = true;
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(syntax("invalid number: expected exponent digits"));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        let raw = &self.src[start..self.pos];
        if is_float {
            match raw.parse::<f64>() {
                Ok(v) if v.is_finite() => Ok(ParsedValue::Float(v, raw)),
                _ => Ok(ParsedValue::OverflowFloat(raw)),
            }
        } else {
            match raw.parse::<i64>() {
                Ok(v) => Ok(ParsedValue::Int(v, raw)),
                Err(_) => Ok(ParsedValue::OutOfRangeInt(raw)),
            }
        }
    }

    /// Validates a JSON string per RFC 8259 §7 without allocating, returning the raw slice
    /// including its enclosing double quotes.
    fn validate_string(&mut self) -> Result<&'a str, JsonReadErrorKind> {
        let start = self.pos;
        if self.peek() != Some(b'"') {
            return Err(syntax("expected `\"`"));
        }
        self.pos += 1;
        while let Some(b) = self.peek() {
            match b {
                b'"' => {
                    self.pos += 1;
                    return Ok(&self.src[start..self.pos]);
                }
                b'\\' => {
                    self.pos += 1;
                    let esc = self
                        .peek()
                        .ok_or_else(|| syntax("unterminated escape sequence in string"))?;
                    match esc {
                        b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => {
                            self.pos += 1;
                        }
                        b'u' => {
                            self.pos += 1;
                            let code = self.parse_hex4()?;
                            if (0xD800..=0xDBFF).contains(&code) {
                                // High surrogate must be immediately followed by a low surrogate.
                                if !self.bytes[self.pos..].starts_with(b"\\u") {
                                    return Err(syntax("unpaired UTF-16 surrogate in string"));
                                }
                                self.pos += 2;
                                let low = self.parse_hex4()?;
                                if !(0xDC00..=0xDFFF).contains(&low) {
                                    return Err(syntax("unpaired UTF-16 surrogate in string"));
                                }
                            } else if (0xDC00..=0xDFFF).contains(&code) {
                                return Err(syntax("unpaired UTF-16 surrogate in string"));
                            }
                        }
                        _ => return Err(syntax("invalid escape sequence in string")),
                    }
                }
                0x00..=0x1F => {
                    return Err(syntax("unescaped control character in string"));
                }
                _ => {
                    self.pos += 1;
                }
            }
        }
        Err(syntax("unterminated string"))
    }

    fn parse_string_decoded(&mut self) -> Result<Cow<'a, str>, JsonReadErrorKind> {
        let raw = self.validate_string()?;
        let inner = &raw[1..raw.len() - 1];
        if !inner.contains('\\') {
            Ok(Cow::Borrowed(inner))
        } else {
            let mut out = String::with_capacity(inner.len());
            unescape_validated_string_into(inner, &mut out);
            Ok(Cow::Owned(out))
        }
    }

    fn parse_hex4(&mut self) -> Result<u16, JsonReadErrorKind> {
        if self.pos + 4 > self.bytes.len() {
            return Err(syntax("incomplete `\\u` escape in string"));
        }
        let mut val: u16 = 0;
        for i in 0..4 {
            let d = hex_digit(self.bytes[self.pos + i])
                .ok_or_else(|| syntax("invalid hex digit in `\\u` escape"))?;
            val = (val << 4) | u16::from(d);
        }
        self.pos += 4;
        Ok(val)
    }

    /// Iteratively validates a nested JSON object or array without recursion (so hostile nesting
    /// cannot overflow the call stack), returning its raw slice.
    fn validate_container(&mut self) -> Result<&'a str, JsonReadErrorKind> {
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum Frame {
            Array,
            Object,
        }

        let start = self.pos;
        let mut stack: Vec<Frame> = Vec::new();

        'parse_value: loop {
            match self.peek() {
                Some(b'{') => {
                    self.pos += 1;
                    self.skip_ws();
                    if self.peek() == Some(b'}') {
                        self.pos += 1;
                    } else {
                        stack.push(Frame::Object);
                        self.validate_string()?;
                        self.skip_ws();
                        if self.peek() != Some(b':') {
                            return Err(syntax("expected `:` after object key"));
                        }
                        self.pos += 1;
                        self.skip_ws();
                        continue 'parse_value;
                    }
                }
                Some(b'[') => {
                    self.pos += 1;
                    self.skip_ws();
                    if self.peek() == Some(b']') {
                        self.pos += 1;
                    } else {
                        stack.push(Frame::Array);
                        continue 'parse_value;
                    }
                }
                Some(b'"') => {
                    self.validate_string()?;
                }
                Some(b'n') => self.expect_literal(b"null")?,
                Some(b't') => self.expect_literal(b"true")?,
                Some(b'f') => self.expect_literal(b"false")?,
                Some(b'-' | b'0'..=b'9') => {
                    self.parse_number()?;
                }
                Some(_) => return Err(syntax("unexpected character while parsing value")),
                None => return Err(syntax("unexpected end of input while parsing value")),
            }

            // Finished one value: advance through enclosing containers.
            loop {
                let Some(&top) = stack.last() else {
                    return Ok(&self.src[start..self.pos]);
                };
                self.skip_ws();
                match (top, self.peek()) {
                    (Frame::Array, Some(b',')) => {
                        self.pos += 1;
                        self.skip_ws();
                        continue 'parse_value;
                    }
                    (Frame::Array, Some(b']')) => {
                        self.pos += 1;
                        stack.pop();
                    }
                    (Frame::Object, Some(b',')) => {
                        self.pos += 1;
                        self.skip_ws();
                        self.validate_string()?;
                        self.skip_ws();
                        if self.peek() != Some(b':') {
                            return Err(syntax("expected `:` after object key"));
                        }
                        self.pos += 1;
                        self.skip_ws();
                        continue 'parse_value;
                    }
                    (Frame::Object, Some(b'}')) => {
                        self.pos += 1;
                        stack.pop();
                    }
                    (Frame::Array, Some(_)) => {
                        return Err(syntax("expected `,` or `]` in array"));
                    }
                    (Frame::Object, Some(_)) => {
                        return Err(syntax("expected `,` or `}` in object"));
                    }
                    (_, None) => return Err(syntax("unexpected end of input")),
                }
            }
        }
    }
}

fn syntax(detail: impl Into<String>) -> JsonReadErrorKind {
    JsonReadErrorKind::Syntax {
        detail: detail.into(),
    }
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Unescapes the interior of a string already validated by [`JsonParser::validate_string`].
fn unescape_validated_string_into(inner: &str, out: &mut String) {
    let bytes = inner.as_bytes();
    let mut i = 0;
    let mut chunk_start = 0;
    while i < bytes.len() {
        if bytes[i] != b'\\' {
            i += 1;
            continue;
        }
        if i > chunk_start {
            out.push_str(&inner[chunk_start..i]);
        }
        i += 1;
        match bytes[i] {
            b'"' => {
                out.push('"');
                i += 1;
            }
            b'\\' => {
                out.push('\\');
                i += 1;
            }
            b'/' => {
                out.push('/');
                i += 1;
            }
            b'b' => {
                out.push('\u{0008}');
                i += 1;
            }
            b'f' => {
                out.push('\u{000C}');
                i += 1;
            }
            b'n' => {
                out.push('\n');
                i += 1;
            }
            b'r' => {
                out.push('\r');
                i += 1;
            }
            b't' => {
                out.push('\t');
                i += 1;
            }
            b'u' => {
                i += 1;
                let code = decode_hex4(&bytes[i..i + 4]);
                i += 4;
                if (0xD800..=0xDBFF).contains(&code) {
                    // Validated surrogate pair: skip `\u` and read low surrogate.
                    i += 2;
                    let low = decode_hex4(&bytes[i..i + 4]);
                    i += 4;
                    let cp =
                        0x10000 + ((u32::from(code) - 0xD800) << 10) + (u32::from(low) - 0xDC00);
                    out.push(char::from_u32(cp).expect("validated surrogate pair"));
                } else {
                    out.push(char::from_u32(u32::from(code)).expect("validated BMP scalar"));
                }
            }
            _ => unreachable!("validated escape sequence"),
        }
        chunk_start = i;
    }
    if chunk_start < bytes.len() {
        out.push_str(&inner[chunk_start..]);
    }
}

fn decode_hex4(bytes: &[u8]) -> u16 {
    let mut val = 0u16;
    for &b in bytes {
        val = (val << 4) | u16::from(hex_digit(b).expect("validated hex digit"));
    }
    val
}

/// Copies `raw` into `out`, removing JSON whitespace (` `, `\t`, `\r`, `\n`) outside string
/// literals while keeping escapes, number spelling, key order, and string contents untouched.
fn strip_json_whitespace(raw: &str, out: &mut String) {
    let bytes = raw.as_bytes();
    let mut in_string = false;
    let mut escaped = false;
    let mut chunk_start = 0;
    for (i, &b) in bytes.iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
        } else if b == b'"' {
            in_string = true;
        } else if is_json_ws(b) {
            if i > chunk_start {
                out.push_str(&raw[chunk_start..i]);
            }
            chunk_start = i + 1;
        }
    }
    if chunk_start < bytes.len() {
        out.push_str(&raw[chunk_start..]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::arrow::array::{
        Array, AsArray, BooleanArray, Float64Array, Int64Array, StringArray,
    };
    use datafusion::datasource::physical_plan::FileGroup;
    use datafusion::object_store::ObjectStoreExt;
    use datafusion::object_store::memory::InMemory;
    use datafusion::physical_plan::collect;
    use datafusion::prelude::{SessionConfig, SessionContext};

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(f)
    }

    fn infer_str(input: &[u8], layout: JsonLayout, infer_rows: usize) -> DfResult<SchemaRef> {
        let sample = sample_from_read(input, "test.json", layout, infer_rows)?;
        Ok(sample.into_schema())
    }

    fn decode_all(
        input: &[u8],
        layout: JsonLayout,
        full_schema: SchemaRef,
        projected_indices: &[usize],
        batch_size: usize,
        chunk_size: usize,
    ) -> DfResult<Vec<RecordBatch>> {
        let mut scanner = RecordScanner::new("test.json".into(), layout);
        let mut decoder = BatchDecoder::try_new(full_schema, projected_indices, batch_size)?;
        let mut batches = Vec::new();
        let mut offset = 0;
        loop {
            match scanner.next_record() {
                Ok(Step::Record {
                    bytes,
                    line,
                    record,
                }) => {
                    if let Err(kind) = decoder.push_record(bytes) {
                        return Err(JsonReadError {
                            file: scanner.file.clone(),
                            line,
                            record,
                            looks_like_json_array: scanner.looks_like_json_array,
                            kind,
                        }
                        .into_datafusion());
                    }
                    if decoder.is_full() {
                        batches.push(decoder.flush()?);
                    }
                }
                Ok(Step::NeedMore) => {
                    if offset >= input.len() {
                        scanner.set_eof();
                    } else {
                        let end = (offset + chunk_size).min(input.len());
                        scanner.feed(&input[offset..end]);
                        offset = end;
                    }
                }
                Ok(Step::Eof) => {
                    if decoder.row_count() > 0 {
                        batches.push(decoder.flush()?);
                    }
                    return Ok(batches);
                }
                Err(err) => return Err(err.into_datafusion()),
            }
        }
    }

    fn json_err(err: &DataFusionError) -> &JsonReadError {
        crate::errors::find_cause::<JsonReadError>(err)
            .unwrap_or_else(|| panic!("expected JsonReadError, got: {err:?}"))
    }

    fn is_arrow_json(field: &Field) -> bool {
        field.data_type() == &DataType::Utf8 && field.extension_type_name() == Some(JSON_EXTENSION)
    }

    #[test]
    fn empty_files_blank_lines_and_bom() {
        // Empty and whitespace-only NDJSON/Auto files give an empty schema and 0 rows.
        for input in [b"".as_slice(), b"   \n\t\r\n  ", b"\xEF\xBB\xBF"] {
            let schema = infer_str(input, JsonLayout::Auto, 1000).unwrap();
            assert_eq!(schema.fields().len(), 0);
            let batches = decode_all(input, JsonLayout::Auto, schema, &[], 16, 1).unwrap();
            assert!(batches.is_empty());
        }

        // Empty JSON array gives an empty schema and 0 rows.
        let schema = infer_str(b"[\n  \n]", JsonLayout::Array, 1000).unwrap();
        assert_eq!(schema.fields().len(), 0);
        let batches = decode_all(b"[]", JsonLayout::Array, schema, &[], 16, 1).unwrap();
        assert!(batches.is_empty());

        // Blank lines in NDJSON are skipped while preserving exact 1-based line numbers.
        let input = b"\n  \r\n{\"a\": 1}\n\n{\"a\": \"bad\"}\n";
        let schema = infer_str(input, JsonLayout::Ndjson, 1).unwrap();
        let err = decode_all(input, JsonLayout::Ndjson, schema, &[0], 16, 1).unwrap_err();
        let je = json_err(&err);
        assert_eq!(je.line, 5);
        assert_eq!(je.record, None);
        assert!(matches!(je.kind, JsonReadErrorKind::TypeMismatch { .. }));

        // Leading UTF-8 BOM is skipped before detection and decoding (even 1 byte at a time).
        let bom_arr = b"\xEF\xBB\xBF[{\"id\": 1}, {\"id\": 2}]";
        let schema = infer_str(bom_arr, JsonLayout::Auto, 1000).unwrap();
        assert_eq!(schema.field(0).data_type(), &DataType::Int64);
        let batches = decode_all(bom_arr, JsonLayout::Auto, schema, &[0], 16, 1).unwrap();
        let ids = batches[0]
            .column(0)
            .as_primitive::<datafusion::arrow::datatypes::Int64Type>();
        assert_eq!(ids.values(), &[1, 2]);

        let bom_ndjson = b"\xEF\xBB\xBF{\"id\": 10}\n{\"id\": 20}\n";
        let schema = infer_str(bom_ndjson, JsonLayout::Auto, 1000).unwrap();
        let batches = decode_all(bom_ndjson, JsonLayout::Auto, schema, &[0], 16, 1).unwrap();
        let ids = batches[0]
            .column(0)
            .as_primitive::<datafusion::arrow::datatypes::Int64Type>();
        assert_eq!(ids.values(), &[10, 20]);
    }

    #[test]
    fn number_inference_and_precision_rules() {
        // Out-of-i64 integers and f64-overflowing numbers infer `arrow.json` with exact digits.
        let input = b"{\"u64\": 18446744073709551615, \"neg\": -9223372036854775809, \
                      \"huge\": 1e400, \"nhuge\": -1e400, \"dot\": 1.0, \"exp\": 1e3, \
                      \"exact\": 9007199254740993}\n";
        let schema = infer_str(input, JsonLayout::Ndjson, 1000).unwrap();
        assert!(is_arrow_json(schema.field_with_name("u64").unwrap()));
        assert!(is_arrow_json(schema.field_with_name("neg").unwrap()));
        assert!(is_arrow_json(schema.field_with_name("huge").unwrap()));
        assert!(is_arrow_json(schema.field_with_name("nhuge").unwrap()));
        assert_eq!(
            schema.field_with_name("dot").unwrap().data_type(),
            &DataType::Float64
        );
        assert_eq!(
            schema.field_with_name("exp").unwrap().data_type(),
            &DataType::Float64
        );
        assert_eq!(
            schema.field_with_name("exact").unwrap().data_type(),
            &DataType::Int64
        );

        let all_idx: Vec<usize> = (0..schema.fields().len()).collect();
        let batches = decode_all(input, JsonLayout::Ndjson, schema, &all_idx, 16, 64).unwrap();
        let b = &batches[0];
        assert_eq!(
            b.column(0).as_string::<i32>().value(0),
            "18446744073709551615"
        );
        assert_eq!(
            b.column(1).as_string::<i32>().value(0),
            "-9223372036854775809"
        );
        assert_eq!(b.column(2).as_string::<i32>().value(0), "1e400");
        assert_eq!(b.column(3).as_string::<i32>().value(0), "-1e400");
        assert_eq!(
            b.column(4)
                .as_primitive::<datafusion::arrow::datatypes::Float64Type>()
                .value(0),
            1.0
        );
        assert_eq!(
            b.column(5)
                .as_primitive::<datafusion::arrow::datatypes::Float64Type>()
                .value(0),
            1000.0
        );
        // 2^53 + 1 stays exact in an Int64 column.
        assert_eq!(
            b.column(6)
                .as_primitive::<datafusion::arrow::datatypes::Int64Type>()
                .value(0),
            9_007_199_254_740_993
        );

        // Mixed Int64 + Float64 rounds 2^53 + 1 in Float64 (accepted rounding).
        let mixed = b"{\"x\": 9007199254740993}\n{\"x\": 1.5}\n";
        let schema = infer_str(mixed, JsonLayout::Ndjson, 1000).unwrap();
        assert_eq!(schema.field(0).data_type(), &DataType::Float64);
        let batches = decode_all(mixed, JsonLayout::Ndjson, schema, &[0], 16, 64).unwrap();
        let col = batches[0]
            .column(0)
            .as_primitive::<datafusion::arrow::datatypes::Float64Type>();
        assert_eq!(col.value(0), 9_007_199_254_740_992.0);
        assert_eq!(col.value(1), 1.5);
    }

    #[test]
    fn duplicate_keys_whitespace_stripping_missing_and_extra_keys() {
        // Top-level duplicate keys: last wins (both inference and decoding).
        // Nested duplicate keys: kept as written minus whitespace.
        // Whitespace inside strings is preserved; outside strings is stripped.
        let input = b"{\"a\": \"ignored\", \"a\": 42, \"nested\": { \"k\": 1, \"k\": \"a  b\\t\\\"c\\\"\" }, \"extra\": 9}\n\
                      {\"nested\": [ 1 , \"x y\" ]}\n";
        let schema = infer_str(input, JsonLayout::Ndjson, 1).unwrap();
        assert_eq!(schema.fields().len(), 3);
        assert_eq!(
            schema.field_with_name("a").unwrap().data_type(),
            &DataType::Int64
        );
        assert!(is_arrow_json(schema.field_with_name("nested").unwrap()));

        // Late key first seen past `infer_rows` is dropped silently.
        let late = b"{\"a\": 1}\n{\"a\": 2, \"late_only\": \"dropped\"}\n";
        let late_schema = infer_str(late, JsonLayout::Ndjson, 1).unwrap();
        assert_eq!(late_schema.fields().len(), 1);
        let late_batches = decode_all(late, JsonLayout::Ndjson, late_schema, &[0], 16, 64).unwrap();
        assert_eq!(late_batches[0].num_rows(), 2);

        // Missing keys become null; extra keys are ignored; nested whitespace stripped only outside strings.
        let target = Arc::new(Schema::new(vec![
            Field::new("a", DataType::Int64, true),
            arrow_json_field("nested"),
        ]));
        let batches = decode_all(input, JsonLayout::Ndjson, target, &[0, 1], 16, 64).unwrap();
        let a = batches[0]
            .column(0)
            .as_primitive::<datafusion::arrow::datatypes::Int64Type>();
        assert_eq!(a.value(0), 42);
        assert!(a.is_null(1));
        let nested = batches[0].column(1).as_string::<i32>();
        assert_eq!(nested.value(0), r#"{"k":1,"k":"a  b\t\"c\""}"#);
        assert_eq!(nested.value(1), r#"[1,"x y"]"#);
    }

    #[test]
    fn type_check_table_every_row() {
        let check_accepts = |field: Field, json_val: &str| {
            let line = format!("{{\"c\": {json_val}}}\n");
            let schema = Arc::new(Schema::new(vec![field]));
            decode_all(line.as_bytes(), JsonLayout::Ndjson, schema, &[0], 16, 64)
                .unwrap_or_else(|e| panic!("expected {json_val} to succeed: {e}"));
        };
        let check_rejects = |field: Field, json_val: &str, expected_kind: JsonValueKind| {
            let line = format!("{{\"c\": {json_val}}}\n");
            let expected_dt = field.data_type().clone();
            let schema = Arc::new(Schema::new(vec![field]));
            let err =
                decode_all(line.as_bytes(), JsonLayout::Ndjson, schema, &[0], 16, 64).unwrap_err();
            let je = json_err(&err);
            assert_eq!(
                je.kind,
                JsonReadErrorKind::TypeMismatch {
                    column: "c".into(),
                    expected: expected_dt,
                    found: expected_kind,
                },
                "value {json_val}"
            );
        };

        // Boolean
        let f = || Field::new("c", DataType::Boolean, true);
        for ok in ["true", "false", "null"] {
            check_accepts(f(), ok);
        }
        for (bad, k) in [
            ("\"true\"", JsonValueKind::String),
            ("1", JsonValueKind::Integer),
            ("1.5", JsonValueKind::Float),
            ("[]", JsonValueKind::Array),
            ("{}", JsonValueKind::Object),
        ] {
            check_rejects(f(), bad, k);
        }

        // Int64
        let f = || Field::new("c", DataType::Int64, true);
        for ok in ["0", "-9223372036854775808", "9223372036854775807", "null"] {
            check_accepts(f(), ok);
        }
        for (bad, k) in [
            ("1.0", JsonValueKind::Float),
            ("1.5", JsonValueKind::Float),
            ("1e3", JsonValueKind::Float),
            ("18446744073709551615", JsonValueKind::OutOfRangeInteger),
            ("1e400", JsonValueKind::OverflowFloat),
            ("\"1\"", JsonValueKind::String),
            ("true", JsonValueKind::Boolean),
            ("[]", JsonValueKind::Array),
            ("{}", JsonValueKind::Object),
        ] {
            check_rejects(f(), bad, k);
        }

        // Float64
        let f = || Field::new("c", DataType::Float64, true);
        for ok in [
            "0",
            "42",
            "1.0",
            "1.5",
            "1e3",
            "-9223372036854775808",
            "null",
        ] {
            check_accepts(f(), ok);
        }
        for (bad, k) in [
            ("18446744073709551615", JsonValueKind::OutOfRangeInteger),
            ("1e400", JsonValueKind::OverflowFloat),
            ("\"1.5\"", JsonValueKind::String),
            ("false", JsonValueKind::Boolean),
            ("[]", JsonValueKind::Array),
            ("{}", JsonValueKind::Object),
        ] {
            check_rejects(f(), bad, k);
        }

        // Utf8 (no extension)
        let f = || Field::new("c", DataType::Utf8, true);
        for ok in ["\"\"", "\"hello\\n\\u00e9\\uD83D\\uDE00\"", "null"] {
            check_accepts(f(), ok);
        }
        for (bad, k) in [
            ("1", JsonValueKind::Integer),
            ("1.5", JsonValueKind::Float),
            ("true", JsonValueKind::Boolean),
            ("[]", JsonValueKind::Array),
            ("{}", JsonValueKind::Object),
        ] {
            check_rejects(f(), bad, k);
        }

        // Utf8 + arrow.json accepts everything.
        for ok in [
            "null",
            "true",
            "false",
            "42",
            "1.5",
            "18446744073709551615",
            "1e400",
            "\"hi\"",
            "[1, 2]",
            "{\"a\": 1}",
        ] {
            check_accepts(arrow_json_field("c"), ok);
        }
    }

    #[test]
    fn decode_into_wider_target_schema_and_join_equivalence() {
        let f1 = b"{\"a\": 1, \"b\": \"hi\", \"c\": null}\n";
        let f2 = b"{\"a\": 2.5, \"b\": {\"k\": 1}, \"c\": [1]}\n";
        let combined_bytes: Vec<u8> = [f1.as_slice(), f2.as_slice()].concat();

        let mut joined =
            sample_from_read(f1.as_slice(), "a.ndjson", JsonLayout::Ndjson, 1000).unwrap();
        let s2 = sample_from_read(f2.as_slice(), "b.ndjson", JsonLayout::Ndjson, 1000).unwrap();
        joined.merge(s2);
        let joined_schema = joined.into_schema();

        let single_schema = infer_str(&combined_bytes, JsonLayout::Ndjson, 1000).unwrap();
        assert_eq!(joined_schema, single_schema);

        // Decode f1 alone into the wider joined schema: `a` becomes Float64, `b` quotes the string.
        let batches =
            decode_all(f1, JsonLayout::Ndjson, joined_schema, &[0, 1, 2], 16, 64).unwrap();
        assert_eq!(
            batches[0]
                .column(0)
                .as_any()
                .downcast_ref::<Float64Array>()
                .unwrap()
                .value(0),
            1.0
        );
        assert_eq!(
            batches[0]
                .column(1)
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap()
                .value(0),
            "\"hi\""
        );
        assert!(batches[0].column(2).is_null(0));
    }

    #[test]
    fn multi_batch_streaming_and_array_scanner_edges() {
        // Multi-line array scanned 1 byte at a time across multiple batches.
        let arr = b"[\n  {\"id\": 1, \"ok\": true},\n  {\"id\": 2, \"ok\": false},\n  {\"id\": 3, \"ok\": null}\n]\n";
        let schema = infer_str(arr, JsonLayout::Array, 1000).unwrap();
        let batches = decode_all(arr, JsonLayout::Array, schema.clone(), &[0, 1], 2, 1).unwrap();
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].num_rows(), 2);
        assert_eq!(batches[1].num_rows(), 1);
        assert_eq!(
            batches[0]
                .column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap()
                .values(),
            &[1, 2]
        );
        assert!(
            batches[0]
                .column(1)
                .as_any()
                .downcast_ref::<BooleanArray>()
                .unwrap()
                .value(0)
        );
        assert_eq!(
            batches[1]
                .column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap()
                .values(),
            &[3]
        );
        assert!(batches[1].column(1).is_null(0));

        // Array scanner errors: trailing comma, missing `]`, trailing junk, non-object element.
        for bad in [
            b"[{\"id\": 1},]".as_slice(),
            b"[{\"id\": 1}".as_slice(),
            b"[{\"id\": 1}] junk".as_slice(),
            b"[,{\"id\": 1}]".as_slice(),
            b"[{\"id\": 1} {\"id\": 2}]".as_slice(),
        ] {
            let err = decode_all(bad, JsonLayout::Array, schema.clone(), &[0], 16, 1).unwrap_err();
            assert!(
                matches!(json_err(&err).kind, JsonReadErrorKind::Syntax { .. }),
                "expected syntax error for {:?}, got {:?}",
                String::from_utf8_lossy(bad),
                json_err(&err)
            );
        }

        // Non-object element in minified single-line array reports 1-based record index and line.
        let non_obj = b"[{\"id\":1},42]";
        let err = decode_all(non_obj, JsonLayout::Array, schema.clone(), &[0], 16, 1).unwrap_err();
        let je = json_err(&err);
        assert_eq!(je.line, 1);
        assert_eq!(je.record, Some(2));
        assert_eq!(
            je.kind,
            JsonReadErrorKind::NonObject {
                found: JsonValueKind::Integer
            }
        );

        // JSON array read as NDJSON sets `looks_like_json_array = true` for both minified and pretty.
        for arr_as_ndjson in [
            b"[{\"id\":1}]".as_slice(),
            b"[\n  {\"id\": 1}\n]".as_slice(),
        ] {
            let err = infer_str(arr_as_ndjson, JsonLayout::Ndjson, 1000).unwrap_err();
            let je = json_err(&err);
            assert!(je.looks_like_json_array, "{je:?}");
        }
    }

    #[test]
    fn projection_pushdown_skips_building_unprojected_but_type_checks_them() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, true),
            Field::new("n", DataType::Int64, true),
            arrow_json_field("blob"),
        ]));

        // Projecting only `id` builds a 1-column batch.
        let good = b"{\"id\": 1, \"n\": 10, \"blob\": {\"big\": [1, 2, 3]}}\n\
                     {\"id\": 2, \"n\": 20, \"blob\": {\"big\": [4, 5, 6]}}\n";
        let batches = decode_all(good, JsonLayout::Ndjson, schema.clone(), &[0], 16, 64).unwrap();
        assert_eq!(batches[0].num_columns(), 1);
        assert_eq!(batches[0].schema().field(0).name(), "id");
        assert_eq!(batches[0].num_rows(), 2);

        // Projecting 0 columns (for `count`) builds a 0-column batch with exact row count.
        let batches = decode_all(good, JsonLayout::Ndjson, schema.clone(), &[], 16, 64).unwrap();
        assert_eq!(batches[0].num_columns(), 0);
        assert_eq!(batches[0].num_rows(), 2);

        // Type mismatch in unprojected column `n` still fails, both for `[0]` and for `[]`.
        let bad = b"{\"id\": 1, \"n\": 10, \"blob\": {}}\n\
                    {\"id\": 2, \"n\": \"oops\", \"blob\": {}}\n";
        for proj in [&[0usize][..], &[][..]] {
            let err =
                decode_all(bad, JsonLayout::Ndjson, schema.clone(), proj, 16, 64).unwrap_err();
            let je = json_err(&err);
            assert_eq!(je.line, 2);
            assert_eq!(
                je.kind,
                JsonReadErrorKind::TypeMismatch {
                    column: "n".into(),
                    expected: DataType::Int64,
                    found: JsonValueKind::String,
                }
            );
        }
    }

    #[test]
    fn async_stream_and_file_format_infer_and_read_from_in_memory_store() {
        block_on(async {
            let store = Arc::new(InMemory::new());
            let p1 = ObjectPath::from("a.json");
            let p2 = ObjectPath::from("b.json");
            store
                .put(
                    &p1,
                    b"{\"id\": 1, \"v\": \"hi\"}\n{\"id\": 2, \"v\": \"yo\"}\n"
                        .as_slice()
                        .into(),
                )
                .await
                .unwrap();
            store
                .put(&p2, b"[{\"id\": 3, \"v\": {\"k\": 1}}]".as_slice().into())
                .await
                .unwrap();

            let format = DataScriptJsonFormat::new(
                JsonLayout::Auto,
                1000,
                FileCompressionType::UNCOMPRESSED,
            );
            let metas = vec![
                store.head(&p1).await.unwrap(),
                store.head(&p2).await.unwrap(),
            ];
            let ctx = SessionContext::new();
            let state = ctx.state();
            let schema = format
                .infer_schema(&state, &(store.clone() as Arc<dyn ObjectStore>), &metas)
                .await
                .unwrap();
            assert_eq!(schema.field(0).data_type(), &DataType::Int64);
            assert!(is_arrow_json(schema.field(1)));

            // Execute via FileSource + DataSourceExec with small batch size over InMemory stream.
            let source = format.file_source(TableSchema::from(schema.clone()));
            let store_url =
                datafusion::datasource::object_store::ObjectStoreUrl::parse("memory://").unwrap();
            let conf = FileScanConfigBuilder::new(store_url.clone(), source)
                .with_file_group(FileGroup::new(vec![
                    PartitionedFile::new_from_meta(metas[0].clone()),
                    PartitionedFile::new_from_meta(metas[1].clone()),
                ]))
                .build();
            ctx.register_object_store(store_url.as_ref(), store.clone());
            let plan = format.create_physical_plan(&state, conf).await.unwrap();
            let batches = collect(plan, ctx.task_ctx()).await.unwrap();
            let total_rows: usize = batches.iter().map(RecordBatch::num_rows).sum();
            assert_eq!(total_rows, 3);
        });
    }

    #[test]
    fn large_file_is_never_byte_range_split_and_keeps_exact_line_numbers() {
        block_on(async {
            // Build a file larger than a low `repartition_file_min_size` (64 bytes) with a type
            // mismatch on line 50, and run with `target_partitions = 4`.
            let mut body = String::new();
            for i in 1..50 {
                body.push_str(&format!("{{\"id\": {i}, \"n\": {i}}}\n"));
            }
            body.push_str("{\"id\": 50, \"n\": \"bad\"}\n");
            assert!(body.len() > 512);

            let dir = tempfile::TempDir::new().unwrap();
            let file_path = dir.path().join("big.ndjson");
            std::fs::write(&file_path, &body).unwrap();

            let config = SessionConfig::new()
                .with_target_partitions(4)
                .with_batch_size(8)
                .with_repartition_file_scans(true)
                .with_repartition_file_min_size(64);
            let ctx = SessionContext::new_with_config(config);

            let format = Arc::new(DataScriptJsonFormat::new(
                JsonLayout::Ndjson,
                10,
                FileCompressionType::UNCOMPRESSED,
            ));
            let url = datafusion::datasource::listing::ListingTableUrl::parse(
                file_path.to_string_lossy(),
            )
            .unwrap();
            let listing_opts = datafusion::datasource::listing::ListingOptions::new(format)
                .with_file_extension("");
            let schema = listing_opts.infer_schema(&ctx.state(), &url).await.unwrap();
            let table_cfg = datafusion::datasource::listing::ListingTableConfig::new(url)
                .with_listing_options(listing_opts)
                .with_schema(schema);
            let table = datafusion::datasource::listing::ListingTable::try_new(table_cfg).unwrap();
            ctx.register_table("t", Arc::new(table)).unwrap();

            let df = ctx
                .table("t")
                .await
                .unwrap()
                .select_columns(&["id"])
                .unwrap();
            let err = df.collect().await.unwrap_err();
            let je = json_err(&err);
            assert_eq!(je.line, 50);
            assert_eq!(je.record, None);
            assert_eq!(
                je.kind,
                JsonReadErrorKind::TypeMismatch {
                    column: "n".into(),
                    expected: DataType::Int64,
                    found: JsonValueKind::String,
                }
            );
        });
    }
}
