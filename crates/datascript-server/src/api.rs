//! HTTP JSON and streaming export endpoints under `/api/*`.

use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use arrow_array::RecordBatch;
use arrow_schema::Schema;
use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::{Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use datascript_compiler::capabilities::SourceFormat;
use datascript_compiler::ir::RenderSpec;
use datascript_compiler::source::format_of_path;
use datascript_diagnostics::codes::{DS1002, DS3001, DS3007};
use datascript_diagnostics::json::{JsonDiagnostic, JsonSpan};
use datascript_diagnostics::{Diagnostic, FileId, JsonReport, SourceFile, SourceMap};
use datascript_engine::QueryStream;
use datascript_engine::printer::result_truncated;
use datascript_engine::table::column_type_name;
use datascript_engine::writers::{json_rows, write_csv_header, write_csv_rows, write_ndjson};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::auth::api_error;
use crate::{API_VERSION, AppState};

const DEFAULT_FILENAME: &str = "query.ds";

// ---- shared DTOs ----

/// Column name and DataScript type (or `arrow:<DataType>` for untyped Arrow columns).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ColumnInfo {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
}

pub(crate) fn schema_columns(schema: &Schema) -> Vec<ColumnInfo> {
    schema
        .fields()
        .iter()
        .map(|field| ColumnInfo {
            name: field.name().clone(),
            ty: column_type_name(field),
        })
        .collect()
}

pub(crate) fn encode_arrow_ipc_base64(
    schema: &Schema,
    batches: &[RecordBatch],
) -> Result<String, io::Error> {
    let mut bytes = Vec::new();
    let mut writer =
        arrow_ipc::writer::FileWriter::try_new(&mut bytes, schema).map_err(io::Error::other)?;
    for batch in batches {
        writer.write(batch).map_err(io::Error::other)?;
    }
    writer.finish().map_err(io::Error::other)?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

pub(crate) fn diagnostics_to_json(
    diagnostics: &[Diagnostic],
    sources: &SourceMap,
) -> Vec<JsonDiagnostic> {
    JsonReport::new(diagnostics, sources).diagnostics
}

pub(crate) fn source_for_request(
    filename: Option<&str>,
    query: &str,
) -> (SourceMap, SourceFile, FileId) {
    let name = filename
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_FILENAME);
    let mut sources = SourceMap::new();
    let file = sources.add(name, query);
    let src = SourceFile::new(name, query);
    (sources, src, file)
}

// ---- GET /api/info ----

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct InfoResponse {
    version: u32,
    root: String,
    sandboxed: bool,
    default_row_limit: Option<usize>,
    engine: String,
    tables: Vec<String>,
    export_formats: &'static [&'static str],
}

pub(crate) async fn info(State(state): State<Arc<AppState>>) -> Response {
    Json(InfoResponse {
        version: API_VERSION,
        root: state.root_dir.display().to_string(),
        sandboxed: state.sandboxed,
        default_row_limit: state.default_row_limit,
        engine: state.session.engine().name().to_string(),
        tables: state.tables.clone(),
        export_formats: &["csv", "json", "ndjson", "arrow"],
    })
    .into_response()
}

// ---- POST /api/check ----

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CheckRequest {
    pub query: String,
    #[serde(default)]
    pub filename: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CheckResponse {
    version: u32,
    has_errors: bool,
    diagnostics: Vec<JsonDiagnostic>,
    queries: Vec<CheckedQuery>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CheckedQuery {
    index: usize,
    span: JsonSpan,
    #[serde(skip_serializing_if = "Option::is_none")]
    render: Option<RenderSpec>,
}

pub(crate) async fn check(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CheckRequest>,
) -> Response {
    let (sources, src, file) = source_for_request(req.filename.as_deref(), &req.query);
    let ctx = state.exec_context(None);
    let analysis = state.session.check(&src, file, &ctx).await;
    let has_errors = analysis.has_errors();
    let diagnostics = diagnostics_to_json(&analysis.diagnostics, &sources);
    let queries = if has_errors {
        Vec::new()
    } else {
        match datascript_engine::catch_panic(Some(file), "planning the program", || {
            datascript_compiler::program_of(analysis, file)
        }) {
            Ok(Ok(program)) => program
                .queries
                .into_iter()
                .enumerate()
                .map(|(index, q)| CheckedQuery {
                    index,
                    span: JsonSpan::new(q.span, &sources),
                    render: q.render,
                })
                .collect(),
            _ => Vec::new(),
        }
    };
    Json(CheckResponse {
        version: API_VERSION,
        has_errors,
        diagnostics,
        queries,
    })
    .into_response()
}

// ---- POST /api/run ----

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum RunFormat {
    #[default]
    Json,
    Arrow,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RunRequest {
    pub query: String,
    #[serde(default)]
    pub filename: Option<String>,
    /// Optional 0-based query statement index.
    #[serde(default)]
    pub statement: Option<usize>,
    /// Per-query row limit. Omit for server default (`10_000`); `0` for unlimited.
    #[serde(default)]
    pub row_limit: Option<usize>,
    #[serde(default)]
    pub format: RunFormat,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RunResponse {
    version: u32,
    has_errors: bool,
    diagnostics: Vec<JsonDiagnostic>,
    results: Vec<RunQueryResult>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RunQueryResult {
    statement: usize,
    span: JsonSpan,
    columns: Vec<ColumnInfo>,
    row_count: usize,
    truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    render: Option<RenderSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rows: Option<Vec<Map<String, Value>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    arrow_ipc: Option<String>,
}

pub(crate) async fn run(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RunRequest>,
) -> Response {
    let (sources, src, file) = source_for_request(req.filename.as_deref(), &req.query);
    let row_limit = state.effective_row_limit(req.row_limit);
    let ctx = state.exec_context(row_limit);

    let mut stream = state.session.start(&src, file, &ctx).await;
    if stream.has_errors() {
        return Json(RunResponse {
            version: API_VERSION,
            has_errors: true,
            diagnostics: diagnostics_to_json(&stream.diagnostics, &sources),
            results: Vec::new(),
        })
        .into_response();
    }

    let query_count = stream.queries().len();
    if let Some(index) = req.statement {
        if index >= query_count {
            return api_error(
                StatusCode::BAD_REQUEST,
                "invalid_statement",
                format!(
                    "statement index {index} is out of range (program has {query_count} query statements)"
                ),
            );
        }
        stream.select(index);
    }

    let mut diagnostics = std::mem::take(&mut stream.diagnostics);
    let mut results = Vec::new();
    let mut has_errors = false;
    let mut loop_index = 0usize;

    while let Some(started) = stream.next_query().await {
        let statement_index = req.statement.unwrap_or(loop_index);
        loop_index += 1;
        let query = match started {
            Ok(query) => query,
            Err(diag) => {
                diagnostics.push(diag);
                has_errors = true;
                break;
            }
        };
        let collected = match query.collect(row_limit).await {
            Ok(result) => result,
            Err(diag) => {
                diagnostics.push(diag);
                has_errors = true;
                break;
            }
        };
        if collected.truncated
            && let Some(limit) = row_limit
        {
            diagnostics.push(result_truncated(limit, collected.statement, Some(file)));
        }
        let row_count: usize = collected.batches.iter().map(RecordBatch::num_rows).sum();
        let columns = schema_columns(&collected.schema);
        let span = collected
            .statement
            .map(|s| JsonSpan::new(s, &sources))
            .unwrap_or_else(|| {
                JsonSpan::new(datascript_diagnostics::Span::new(file, 0, 0), &sources)
            });

        let (rows, arrow_ipc) = match req.format {
            RunFormat::Json => (Some(json_rows(&collected.schema, &collected.batches)), None),
            RunFormat::Arrow => {
                match encode_arrow_ipc_base64(&collected.schema, &collected.batches) {
                    Ok(encoded) => (None, Some(encoded)),
                    Err(err) => {
                        return api_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "arrow_encode_error",
                            format!("failed to encode Arrow IPC: {err}"),
                        );
                    }
                }
            }
        };

        results.push(RunQueryResult {
            statement: statement_index,
            span,
            columns,
            row_count,
            truncated: collected.truncated,
            render: collected.render,
            rows,
            arrow_ipc,
        });
    }

    Json(RunResponse {
        version: API_VERSION,
        has_errors,
        diagnostics: diagnostics_to_json(&diagnostics, &sources),
        results,
    })
    .into_response()
}

// ---- POST /api/export (Q-007 / D-062 full untruncated streaming export) ----

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ExportFormat {
    #[default]
    Csv,
    Json,
    Ndjson,
    Arrow,
}

impl ExportFormat {
    fn content_type(self) -> &'static str {
        match self {
            ExportFormat::Csv => "text/csv; charset=utf-8",
            ExportFormat::Json => "application/json; charset=utf-8",
            ExportFormat::Ndjson => "application/x-ndjson; charset=utf-8",
            ExportFormat::Arrow => "application/vnd.apache.arrow.file",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            ExportFormat::Csv => "csv",
            ExportFormat::Json => "json",
            ExportFormat::Ndjson => "ndjson",
            ExportFormat::Arrow => "arrow",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExportRequest {
    pub query: String,
    #[serde(default)]
    pub filename: Option<String>,
    #[serde(default)]
    pub statement: Option<usize>,
    #[serde(default)]
    pub format: ExportFormat,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticsErrorResponse {
    version: u32,
    has_errors: bool,
    diagnostics: Vec<JsonDiagnostic>,
}

pub(crate) async fn export(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ExportRequest>,
) -> Response {
    let (sources, src, file) = source_for_request(req.filename.as_deref(), &req.query);
    // Export is always untruncated (Q-007 / D-062).
    let ctx = state.exec_context(None);

    let mut run = state.session.start(&src, file, &ctx).await;
    if run.has_errors() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(DiagnosticsErrorResponse {
                version: API_VERSION,
                has_errors: true,
                diagnostics: diagnostics_to_json(&run.diagnostics, &sources),
            }),
        )
            .into_response();
    }

    let query_count = run.queries().len();
    if query_count == 0 {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(DiagnosticsErrorResponse {
                version: API_VERSION,
                has_errors: false,
                diagnostics: diagnostics_to_json(&run.diagnostics, &sources),
            }),
        )
            .into_response();
    }

    match req.statement {
        Some(idx) if idx >= query_count => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "invalid_statement",
                format!(
                    "statement index {idx} is out of range (program has {query_count} query statements)"
                ),
            );
        }
        Some(idx) => run.select(idx),
        None if query_count > 1 => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "multiple_statements",
                format!(
                    "program has {query_count} query statements; pass `statement` (0-based index) to export one"
                ),
            );
        }
        None => {}
    }

    let mut query = match run.next_query().await {
        Some(Ok(query)) => query,
        Some(Err(diag)) => {
            let mut diagnostics = std::mem::take(&mut run.diagnostics);
            diagnostics.push(diag);
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(DiagnosticsErrorResponse {
                    version: API_VERSION,
                    has_errors: true,
                    diagnostics: diagnostics_to_json(&diagnostics, &sources),
                }),
            )
                .into_response();
        }
        None => {
            return api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "export_error",
                "no query available to export",
            );
        }
    };

    // Pull the first batch before committing the 200 streaming headers so immediate runtime errors
    // (such as a deleted file or constant division by zero) return structured diagnostics.
    let first_batch = match query.batches.next().await.transpose() {
        Ok(batch) => batch,
        Err(diag) => {
            let mut diagnostics = std::mem::take(&mut run.diagnostics);
            diagnostics.push(diag);
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(DiagnosticsErrorResponse {
                    version: API_VERSION,
                    has_errors: true,
                    diagnostics: diagnostics_to_json(&diagnostics, &sources),
                }),
            )
                .into_response();
        }
    };

    let format = req.format;
    let body = match build_export_stream(format, query, first_batch) {
        Ok(body) => body,
        Err(err) => {
            return api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "export_error",
                format!("failed to start export: {err}"),
            );
        }
    };

    let mut response = Response::new(body);
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(format.content_type()),
    );
    let disposition = format!("attachment; filename=\"result.{}\"", format.extension());
    if let Ok(val) = HeaderValue::from_str(&disposition) {
        response
            .headers_mut()
            .insert(header::CONTENT_DISPOSITION, val);
    }
    response
}

enum ExportWriterState {
    Csv {
        query: QueryStream,
        first: Option<RecordBatch>,
        wrote_header: bool,
        done: bool,
    },
    Ndjson {
        query: QueryStream,
        first: Option<RecordBatch>,
        done: bool,
    },
    Json {
        query: QueryStream,
        first: Option<RecordBatch>,
        started: bool,
        wrote_row: bool,
        done: bool,
    },
    Arrow {
        query: QueryStream,
        first: Option<RecordBatch>,
        writer: Option<Box<arrow_ipc::writer::FileWriter<Vec<u8>>>>,
        done: bool,
    },
}

fn build_export_stream(
    format: ExportFormat,
    query: QueryStream,
    first: Option<RecordBatch>,
) -> io::Result<Body> {
    let state = match format {
        ExportFormat::Csv => ExportWriterState::Csv {
            query,
            first,
            wrote_header: false,
            done: false,
        },
        ExportFormat::Ndjson => ExportWriterState::Ndjson {
            query,
            first,
            done: false,
        },
        ExportFormat::Json => ExportWriterState::Json {
            query,
            first,
            started: false,
            wrote_row: false,
            done: false,
        },
        ExportFormat::Arrow => {
            let writer = arrow_ipc::writer::FileWriter::try_new(Vec::new(), &query.schema)
                .map_err(io::Error::other)?;
            ExportWriterState::Arrow {
                query,
                first,
                writer: Some(Box::new(writer)),
                done: false,
            }
        }
    };

    let stream = futures::stream::try_unfold(state, |mut state| async move {
        match &mut state {
            ExportWriterState::Csv {
                query,
                first,
                wrote_header,
                done,
            } => {
                if *done {
                    return Ok(None);
                }
                let mut buf = Vec::new();
                if !*wrote_header {
                    *wrote_header = true;
                    write_csv_header(&mut buf, &query.schema)?;
                    if let Some(batch) = first.take() {
                        write_csv_rows(&mut buf, &query.schema, &[batch])?;
                    } else {
                        *done = true;
                    }
                    return Ok(Some((Bytes::from(buf), state)));
                }
                match query.batches.next().await {
                    Some(Ok(batch)) => {
                        write_csv_rows(&mut buf, &query.schema, &[batch])?;
                        Ok(Some((Bytes::from(buf), state)))
                    }
                    Some(Err(diag)) => Err(io::Error::other(diag.message)),
                    None => {
                        *done = true;
                        Ok(None)
                    }
                }
            }
            ExportWriterState::Ndjson { query, first, done } => {
                if *done {
                    return Ok(None);
                }
                let next_batch = if let Some(batch) = first.take() {
                    Some(batch)
                } else {
                    match query.batches.next().await {
                        Some(Ok(batch)) => Some(batch),
                        Some(Err(diag)) => return Err(io::Error::other(diag.message)),
                        None => None,
                    }
                };
                match next_batch {
                    Some(batch) => {
                        let mut buf = Vec::new();
                        write_ndjson(&mut buf, &query.schema, &[batch])?;
                        Ok(Some((Bytes::from(buf), state)))
                    }
                    None => {
                        *done = true;
                        Ok(None)
                    }
                }
            }
            ExportWriterState::Json {
                query,
                first,
                started,
                wrote_row,
                done,
            } => {
                if *done {
                    return Ok(None);
                }
                let mut buf = Vec::new();
                if !*started {
                    *started = true;
                    buf.push(b'[');
                    if let Some(batch) = first.take() {
                        write_json_array_rows(&mut buf, &query.schema, &batch, wrote_row)?;
                        return Ok(Some((Bytes::from(buf), state)));
                    } else {
                        buf.extend_from_slice(b"]\n");
                        *done = true;
                        return Ok(Some((Bytes::from(buf), state)));
                    }
                }
                match query.batches.next().await {
                    Some(Ok(batch)) => {
                        write_json_array_rows(&mut buf, &query.schema, &batch, wrote_row)?;
                        Ok(Some((Bytes::from(buf), state)))
                    }
                    Some(Err(diag)) => Err(io::Error::other(diag.message)),
                    None => {
                        buf.extend_from_slice(b"]\n");
                        *done = true;
                        Ok(Some((Bytes::from(buf), state)))
                    }
                }
            }
            ExportWriterState::Arrow {
                query,
                first,
                writer,
                done,
            } => {
                if *done {
                    return Ok(None);
                }
                let next_batch = if let Some(batch) = first.take() {
                    Some(batch)
                } else {
                    match query.batches.next().await {
                        Some(Ok(batch)) => Some(batch),
                        Some(Err(diag)) => return Err(io::Error::other(diag.message)),
                        None => None,
                    }
                };
                match next_batch {
                    Some(batch) => {
                        let w = writer
                            .as_mut()
                            .ok_or_else(|| io::Error::other("arrow writer closed"))?;
                        w.write(&batch).map_err(io::Error::other)?;
                        w.flush().map_err(io::Error::other)?;
                        let chunk = std::mem::take(w.get_mut());
                        Ok(Some((Bytes::from(chunk), state)))
                    }
                    None => {
                        *done = true;
                        let mut w = writer
                            .take()
                            .ok_or_else(|| io::Error::other("arrow writer closed"))?;
                        w.finish().map_err(io::Error::other)?;
                        let chunk = std::mem::take(w.get_mut());
                        Ok(Some((Bytes::from(chunk), state)))
                    }
                }
            }
        }
    });

    Ok(Body::from_stream(stream))
}

fn write_json_array_rows(
    buf: &mut Vec<u8>,
    schema: &Schema,
    batch: &RecordBatch,
    wrote_row: &mut bool,
) -> io::Result<()> {
    for row in json_rows(schema, std::slice::from_ref(batch)) {
        if *wrote_row {
            buf.push(b',');
        }
        *wrote_row = true;
        serde_json::to_writer(&mut *buf, &row).map_err(io::Error::from)?;
    }
    Ok(())
}

// ---- GET /api/files ----

#[derive(Debug, Deserialize)]
pub(crate) struct FilesQuery {
    #[serde(default)]
    pub dir: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FilesResponse {
    version: u32,
    dir: String,
    entries: Vec<FileEntry>,
    tables: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FileEntry {
    name: String,
    path: String,
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    format: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<u64>,
}

pub(crate) async fn files(
    State(state): State<Arc<AppState>>,
    Query(query): Query<FilesQuery>,
) -> Response {
    let raw_dir = query.dir.unwrap_or_default();
    let rel_dir = match validate_relative_subpath(&raw_dir) {
        Ok(rel) => rel,
        Err(msg) => return api_error(StatusCode::FORBIDDEN, "outside_root", msg),
    };

    let root_canon = match std::fs::canonicalize(&state.root_dir) {
        Ok(p) => p,
        Err(err) => {
            return api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "io_error",
                format!("cannot read root directory: {err}"),
            );
        }
    };

    let target = if rel_dir.as_os_str().is_empty() {
        root_canon.clone()
    } else {
        root_canon.join(&rel_dir)
    };

    // Check symlink traversal even if the final target doesn't exist.
    if let Err(resp) = ensure_within_root(&target, &root_canon) {
        return resp;
    }

    let target_canon = match std::fs::canonicalize(&target) {
        Ok(p) => p,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return api_error(
                StatusCode::NOT_FOUND,
                "not_found",
                format!("directory `{raw_dir}` does not exist"),
            );
        }
        Err(err) => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "io_error",
                format!("cannot read directory `{raw_dir}`: {err}"),
            );
        }
    };

    if !target_canon.starts_with(&root_canon) {
        return api_error(
            StatusCode::FORBIDDEN,
            "outside_root",
            "directory resolves outside the root directory",
        );
    }
    if !target_canon.is_dir() {
        return api_error(
            StatusCode::BAD_REQUEST,
            "not_a_directory",
            format!("`{raw_dir}` is not a directory"),
        );
    }

    let read_dir = match std::fs::read_dir(&target_canon) {
        Ok(rd) => rd,
        Err(err) => {
            return api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "io_error",
                format!("cannot list directory: {err}"),
            );
        }
    };

    let rel_prefix = rel_dir.to_string_lossy().replace('\\', "/");

    let mut entries = Vec::new();
    for entry in read_dir.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        let Ok(canon) = std::fs::canonicalize(entry.path()) else {
            // Skip broken symlinks or unreadable entries.
            continue;
        };
        if !canon.starts_with(&root_canon) {
            // Never expose symlinks pointing outside the root directory.
            continue;
        }
        let Ok(meta) = std::fs::metadata(&canon) else {
            continue;
        };
        let entry_path = if rel_prefix.is_empty() {
            name.clone()
        } else {
            format!("{rel_prefix}/{name}")
        };
        if meta.is_dir() {
            entries.push(FileEntry {
                name,
                path: entry_path,
                kind: "directory",
                format: None,
                size: None,
            });
        } else if meta.is_file()
            && let Some(format) = format_of_path(&name)
        {
            entries.push(FileEntry {
                name,
                path: entry_path,
                kind: "file",
                format: Some(format_name(format)),
                size: Some(meta.len()),
            });
        }
    }

    // Directories first, then files, each sorted deterministically by name.
    entries.sort_by(|a, b| {
        let a_dir = a.kind == "directory";
        let b_dir = b.kind == "directory";
        b_dir.cmp(&a_dir).then_with(|| a.name.cmp(&b.name))
    });

    Json(FilesResponse {
        version: API_VERSION,
        dir: rel_prefix,
        entries,
        tables: state.tables.clone(),
    })
    .into_response()
}

/// Validates that `raw` is a safe relative sub-path containing no `..`, root, drive prefix, or
/// NUL byte.
fn validate_relative_subpath(raw: &str) -> Result<PathBuf, &'static str> {
    if raw.contains('\0') {
        return Err("path contains a NUL byte");
    }
    // Reject Windows-style backslashes that could sneak traversal past POSIX path parsing.
    let normalized = raw.replace('\\', "/");
    if normalized.starts_with('/') {
        return Err("path must be relative to the root directory");
    }
    let mut out = PathBuf::new();
    for comp in Path::new(&normalized).components() {
        match comp {
            Component::CurDir => {}
            Component::Normal(seg) => out.push(seg),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err("path traversal outside the root directory is not allowed");
            }
        }
    }
    Ok(out)
}

/// Walks existing ancestors of `target` to verify no intermediate symlink leaves `root_canon`.
fn ensure_within_root(target: &Path, root_canon: &Path) -> Result<(), Response> {
    let mut cur = target.to_path_buf();
    loop {
        if let Ok(canon) = std::fs::canonicalize(&cur) {
            if !canon.starts_with(root_canon) {
                return Err(api_error(
                    StatusCode::FORBIDDEN,
                    "outside_root",
                    "directory resolves outside the root directory",
                ));
            }
            return Ok(());
        }
        if std::fs::symlink_metadata(&cur).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(api_error(
                StatusCode::FORBIDDEN,
                "outside_root",
                "symbolic link cannot be followed outside the root directory",
            ));
        }
        if !cur.pop() || cur == root_canon {
            return Ok(());
        }
    }
}

fn format_name(format: SourceFormat) -> &'static str {
    match format {
        SourceFormat::Csv => "csv",
        SourceFormat::Tsv => "tsv",
        SourceFormat::Parquet => "parquet",
        SourceFormat::Json => "json",
        SourceFormat::Ndjson => "ndjson",
    }
}

// ---- GET /api/schema ----

#[derive(Debug, Deserialize)]
pub(crate) struct SchemaQuery {
    pub source: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SchemaResponse {
    version: u32,
    source: String,
    columns: Vec<ColumnInfo>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SchemaErrorResponse {
    version: u32,
    source: String,
    diagnostics: Vec<JsonDiagnostic>,
}

pub(crate) async fn schema(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SchemaQuery>,
) -> Response {
    let Some(source) = query.source.filter(|s| !s.is_empty()) else {
        return api_error(
            StatusCode::BAD_REQUEST,
            "missing_source",
            "query parameter `source` is required",
        );
    };

    let is_table = match query.kind.as_deref() {
        Some("table") => true,
        Some("file") | None => query.kind.is_none() && state.tables.iter().any(|t| t == &source),
        Some(other) => {
            return api_error(
                StatusCode::BAD_REQUEST,
                "invalid_kind",
                format!("`kind` must be `file` or `table`, got `{other}`"),
            );
        }
    };

    let program_text = if is_table {
        format!("from ['{}']", escape_ds_single_quoted(&source))
    } else {
        format!("from '{}'", escape_ds_single_quoted(&source))
    };

    let (sources, src, file) = source_for_request(Some("<schema>"), &program_text);
    let ctx = state.exec_context(None);
    let analysis = state.session.check(&src, file, &ctx).await;

    if analysis.has_errors() {
        let status = match analysis
            .diagnostics
            .iter()
            .find(|d| d.is_error())
            .map(|d| d.code)
        {
            Some(DS3007) => StatusCode::FORBIDDEN,
            Some(DS3001 | DS1002) => StatusCode::NOT_FOUND,
            _ => StatusCode::BAD_REQUEST,
        };
        return (
            status,
            Json(SchemaErrorResponse {
                version: API_VERSION,
                source,
                diagnostics: diagnostics_to_json(&analysis.diagnostics, &sources),
            }),
        )
            .into_response();
    }

    match datascript_compiler::program_of(analysis, file) {
        Ok(program) if !program.queries.is_empty() => Json(SchemaResponse {
            version: API_VERSION,
            source,
            columns: schema_columns(&program.queries[0].output_schema),
        })
        .into_response(),
        Ok(_) => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "schema_error",
            "schema query produced no plan",
        ),
        Err(diagnostics) => (
            StatusCode::BAD_REQUEST,
            Json(SchemaErrorResponse {
                version: API_VERSION,
                source,
                diagnostics: diagnostics_to_json(&diagnostics, &sources),
            }),
        )
            .into_response(),
    }
}

fn escape_ds_single_quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out
}

// ---- /api/* 404 fallback ----

pub(crate) async fn not_found() -> Response {
    api_error(StatusCode::NOT_FOUND, "not_found", "unknown API endpoint")
}
