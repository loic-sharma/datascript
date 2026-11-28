//! WebSocket endpoint (`/api/ws`) for streaming query batches and mid-query cancellation
//! (D-055, M4-01).
//!
//! Each `run` message spawns a tokio task owning a [`RunStream`](datascript_engine::RunStream).
//! Sending `{ "type": "cancel", "id": "..." }` or closing the WebSocket aborts the task, dropping
//! the `RunStream` / `QueryStream` and cancelling the DataFusion execution immediately.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use datascript_compiler::ir::RenderSpec;
use datascript_diagnostics::Span;
use datascript_diagnostics::json::{JsonDiagnostic, JsonSpan};
use datascript_engine::printer::result_truncated;
use datascript_engine::writers::json_rows;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::api::{
    ColumnInfo, RunFormat, diagnostics_to_json, encode_arrow_ipc_base64, schema_columns,
    source_for_request,
};
use crate::{API_VERSION, AppState};

/// Upgrades an authenticated `/api/ws` request to a WebSocket session.
pub(crate) async fn ws_handler(
    State(state): State<Arc<AppState>>,
    ws: WebSocketUpgrade,
) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum ClientMessage {
    Run {
        id: String,
        query: String,
        #[serde(default)]
        filename: Option<String>,
        #[serde(default)]
        statement: Option<usize>,
        #[serde(default, rename = "rowLimit")]
        row_limit: Option<usize>,
        #[serde(default)]
        format: RunFormat,
    },
    Cancel {
        id: String,
    },
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum ServerMessage {
    Diagnostics {
        id: String,
        version: u32,
        #[serde(rename = "hasErrors")]
        has_errors: bool,
        diagnostics: Vec<JsonDiagnostic>,
    },
    #[serde(rename = "queryStart")]
    QueryStart {
        id: String,
        statement: usize,
        span: JsonSpan,
        columns: Vec<ColumnInfo>,
        #[serde(skip_serializing_if = "Option::is_none")]
        render: Option<RenderSpec>,
    },
    Batch {
        id: String,
        statement: usize,
        #[serde(rename = "rowCount")]
        row_count: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        rows: Option<Vec<Map<String, Value>>>,
        #[serde(rename = "arrowIpc", skip_serializing_if = "Option::is_none")]
        arrow_ipc: Option<String>,
    },
    #[serde(rename = "queryEnd")]
    QueryEnd {
        id: String,
        statement: usize,
        #[serde(rename = "rowCount")]
        row_count: usize,
        truncated: bool,
    },
    Cancelled {
        id: String,
    },
    Done {
        id: String,
        #[serde(rename = "hasErrors")]
        has_errors: bool,
    },
    Error {
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        code: String,
        message: String,
    },
}

async fn handle_socket(socket: WebSocket, state: Arc<AppState>) {
    let (mut ws_sink, mut ws_stream) = socket.split();
    let (tx, mut rx) = mpsc::channel::<ServerMessage>(64);

    let writer = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let Ok(text) = serde_json::to_string(&msg) else {
                continue;
            };
            if ws_sink.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });

    let mut active: HashMap<String, JoinHandle<()>> = HashMap::new();

    while let Some(frame) = ws_stream.next().await {
        let Ok(frame) = frame else {
            break;
        };
        let text = match frame {
            Message::Text(text) => text,
            Message::Close(_) => break,
            Message::Ping(_) | Message::Pong(_) | Message::Binary(_) => continue,
        };

        // Reap completed tasks before checking for duplicate IDs.
        active.retain(|_, handle| !handle.is_finished());

        let msg: ClientMessage = match serde_json::from_str(&text) {
            Ok(msg) => msg,
            Err(err) => {
                let _ = tx
                    .send(ServerMessage::Error {
                        id: None,
                        code: "invalid_message".into(),
                        message: format!("malformed WebSocket message: {err}"),
                    })
                    .await;
                continue;
            }
        };

        match msg {
            ClientMessage::Run {
                id,
                query,
                filename,
                statement,
                row_limit,
                format,
            } => {
                if active.contains_key(&id) {
                    let _ = tx
                        .send(ServerMessage::Error {
                            id: Some(id),
                            code: "duplicate_id".into(),
                            message: "a query with this request id is already running".into(),
                        })
                        .await;
                    continue;
                }
                let task_state = state.clone();
                let task_tx = tx.clone();
                let task_id = id.clone();
                let handle = tokio::spawn(async move {
                    run_streaming(
                        task_state, task_tx, task_id, query, filename, statement, row_limit, format,
                    )
                    .await;
                });
                active.insert(id, handle);
            }
            ClientMessage::Cancel { id } => {
                if let Some(handle) = active.remove(&id) {
                    handle.abort();
                }
                let _ = tx.send(ServerMessage::Cancelled { id }).await;
            }
        }
    }

    // Abort every in-flight query immediately on disconnect, dropping their RunStreams.
    for (_, handle) in active {
        handle.abort();
    }
    drop(tx);
    let _ = writer.await;
}

#[allow(clippy::too_many_arguments)]
async fn run_streaming(
    state: Arc<AppState>,
    tx: mpsc::Sender<ServerMessage>,
    id: String,
    query: String,
    filename: Option<String>,
    statement: Option<usize>,
    requested_row_limit: Option<usize>,
    format: RunFormat,
) {
    let (sources, src, file) = source_for_request(filename.as_deref(), &query);
    let row_limit = state.effective_row_limit(requested_row_limit);
    let ctx = state.exec_context(row_limit);

    let mut run = state.session.start(&src, file, &ctx).await;
    let check_has_errors = run.has_errors();
    let check_diags = diagnostics_to_json(&run.diagnostics, &sources);
    if (!check_diags.is_empty() || check_has_errors)
        && tx
            .send(ServerMessage::Diagnostics {
                id: id.clone(),
                version: API_VERSION,
                has_errors: check_has_errors,
                diagnostics: check_diags,
            })
            .await
            .is_err()
    {
        return;
    }

    if check_has_errors {
        let _ = tx
            .send(ServerMessage::Done {
                id,
                has_errors: true,
            })
            .await;
        return;
    }

    let query_count = run.queries().len();
    if let Some(index) = statement {
        if index >= query_count {
            let _ = tx
                .send(ServerMessage::Error {
                    id: Some(id),
                    code: "invalid_statement".into(),
                    message: format!(
                        "statement index {index} is out of range (program has {query_count} query statements)"
                    ),
                })
                .await;
            return;
        }
        run.select(index);
    }

    let mut has_errors = false;
    let mut loop_index = 0usize;

    while let Some(started) = run.next_query().await {
        let statement_index = statement.unwrap_or(loop_index);
        loop_index += 1;

        let mut qstream = match started {
            Ok(q) => q,
            Err(diag) => {
                has_errors = true;
                let _ = tx
                    .send(ServerMessage::Diagnostics {
                        id: id.clone(),
                        version: API_VERSION,
                        has_errors: true,
                        diagnostics: diagnostics_to_json(&[diag], &sources),
                    })
                    .await;
                break;
            }
        };

        let span = JsonSpan::new(qstream.statement, &sources);
        let columns = schema_columns(&qstream.schema);
        if tx
            .send(ServerMessage::QueryStart {
                id: id.clone(),
                statement: statement_index,
                span,
                columns,
                render: qstream.render.clone(),
            })
            .await
            .is_err()
        {
            return;
        }

        let mut rows_sent = 0usize;
        let mut truncated = false;
        let mut failed = false;

        while let Some(item) = qstream.batches.next().await {
            let batch = match item {
                Ok(batch) => batch,
                Err(diag) => {
                    has_errors = true;
                    failed = true;
                    let _ = tx
                        .send(ServerMessage::Diagnostics {
                            id: id.clone(),
                            version: API_VERSION,
                            has_errors: true,
                            diagnostics: diagnostics_to_json(&[diag], &sources),
                        })
                        .await;
                    break;
                }
            };

            let batch = if let Some(limit) = row_limit {
                let room = limit.saturating_sub(rows_sent);
                if batch.num_rows() > room {
                    truncated = true;
                    if room == 0 {
                        break;
                    }
                    batch.slice(0, room)
                } else {
                    batch
                }
            } else {
                batch
            };

            let batch_rows = batch.num_rows();
            if batch_rows > 0 {
                let (rows, arrow_ipc) = match format {
                    RunFormat::Json => (
                        Some(json_rows(&qstream.schema, std::slice::from_ref(&batch))),
                        None,
                    ),
                    RunFormat::Arrow => {
                        match encode_arrow_ipc_base64(&qstream.schema, std::slice::from_ref(&batch))
                        {
                            Ok(encoded) => (None, Some(encoded)),
                            Err(err) => {
                                has_errors = true;
                                failed = true;
                                let _ = tx
                                    .send(ServerMessage::Error {
                                        id: Some(id.clone()),
                                        code: "arrow_encode_error".into(),
                                        message: format!("failed to encode Arrow IPC: {err}"),
                                    })
                                    .await;
                                break;
                            }
                        }
                    }
                };
                rows_sent += batch_rows;
                if tx
                    .send(ServerMessage::Batch {
                        id: id.clone(),
                        statement: statement_index,
                        row_count: batch_rows,
                        rows,
                        arrow_ipc,
                    })
                    .await
                    .is_err()
                {
                    return;
                }
            }

            if truncated {
                // Dropping `qstream` cancels the underlying DataFusion stream.
                break;
            }
        }

        let stmt_span: Span = qstream.statement;
        drop(qstream);

        if failed {
            break;
        }

        if truncated && let Some(limit) = row_limit {
            let diag = result_truncated(limit, Some(stmt_span), Some(file));
            if tx
                .send(ServerMessage::Diagnostics {
                    id: id.clone(),
                    version: API_VERSION,
                    has_errors: false,
                    diagnostics: diagnostics_to_json(&[diag], &sources),
                })
                .await
                .is_err()
            {
                return;
            }
        }

        if tx
            .send(ServerMessage::QueryEnd {
                id: id.clone(),
                statement: statement_index,
                row_count: rows_sent,
                truncated,
            })
            .await
            .is_err()
        {
            return;
        }
    }

    let _ = tx.send(ServerMessage::Done { id, has_errors }).await;
}
