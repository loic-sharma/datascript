//! End-to-end HTTP, WebSocket, sandbox, streaming export, dev-proxy, and file-watcher tests for
//! `datascript-server` (M4-01).

use std::io::Cursor;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use arrow_array::{Int64Array, RecordBatch};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use axum::routing::get;
use base64::Engine as _;
use datascript_compiler::{Capabilities, SourceError, SourceRef};
use datascript_datafusion::{DataFusionConfig, DataFusionEngine};
use datascript_diagnostics::Diagnostic;
use datascript_engine::{Engine, ExecContext, FixedClock, ResultStream, Session};
use datascript_server::{AppState, ServerConfig, TOKEN_HEADER, bind, router};
use futures::{SinkExt, Stream, StreamExt};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::oneshot;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tower::ServiceExt;

const TEST_TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn make_workspace() -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("root");
    let sub = root.join("sub");
    std::fs::create_dir_all(&sub).unwrap();

    std::fs::write(
        root.join("orders.csv"),
        "orderId,customerId,amount\no-1,c-1,10.5\no-2,c-2,25.0\no-3,c-1,5.25\n",
    )
    .unwrap();
    std::fs::write(root.join("numbers.csv"), "n\n1\n2\n3\n4\n5\n6\n7\n").unwrap();
    std::fs::write(sub.join("nested.csv"), "k,v\na,1\nb,2\n").unwrap();
    std::fs::write(root.join(".hidden.csv"), "x\n1\n").unwrap();
    std::fs::write(root.join("readme.txt"), "not a data file\n").unwrap();
    std::fs::write(dir.path().join("outside.csv"), "secret\n42\n").unwrap();

    (dir, root)
}

fn make_session(config: DataFusionConfig) -> Arc<Session> {
    Arc::new(Session::new(
        Arc::new(DataFusionEngine::new(config)),
        Arc::new(FixedClock(1_717_200_000_000_000)),
    ))
}

fn make_router(root: &Path, configure: impl FnOnce(&mut ServerConfig)) -> Router {
    let mut cfg = ServerConfig::new(root);
    cfg.token = Some(TEST_TOKEN.into());
    cfg.watch_files = false;
    configure(&mut cfg);
    let state = Arc::new(AppState::new(
        make_session(DataFusionConfig::default()),
        &cfg,
        4321,
    ));
    router(state)
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = resp
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    (status, headers, bytes)
}

async fn send_json(app: &Router, req: Request<Body>) -> (StatusCode, Value) {
    let (status, _, bytes) = send(app, req).await;
    let body: Value = serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        panic!(
            "expected JSON (status {status}), got `{}`: {e}",
            String::from_utf8_lossy(&bytes)
        )
    });
    (status, body)
}

fn authed_get(uri: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {TEST_TOKEN}"))
        .body(Body::empty())
        .unwrap()
}

fn authed_post(uri: &str, payload: Value) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {TEST_TOKEN}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap()
}

fn diag_codes(body: &Value) -> Vec<&str> {
    body["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap())
        .collect()
}

// ---- Auth, Origin, Host, Assets ----

#[tokio::test]
async fn auth_rejects_missing_or_wrong_tokens_and_bad_origins() {
    let (_dir, root) = make_workspace();
    let app = make_router(&root, |_| {});

    // Missing token on all endpoints.
    for (method, uri) in [
        (Method::GET, "/api/info"),
        (Method::GET, "/api/files"),
        (Method::GET, "/api/schema?source=orders.csv"),
        (Method::POST, "/api/check"),
        (Method::POST, "/api/run"),
        (Method::POST, "/api/export"),
    ] {
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .unwrap();
        let (status, body) = send_json(&app, req).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{uri}: {body}");
        assert_eq!(body["error"]["code"], "missing_token");
    }

    // Wrong token via Bearer, header, and query param.
    for req in [
        Request::builder()
            .uri("/api/info")
            .header(header::AUTHORIZATION, "Bearer wrong-token")
            .body(Body::empty())
            .unwrap(),
        Request::builder()
            .uri("/api/info")
            .header(TOKEN_HEADER, "wrong-token")
            .body(Body::empty())
            .unwrap(),
        Request::builder()
            .uri("/api/info?token=wrong-token")
            .body(Body::empty())
            .unwrap(),
    ] {
        let (status, body) = send_json(&app, req).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
        assert_eq!(body["error"]["code"], "invalid_token");
    }

    // Valid token via all three mechanisms.
    for req in [
        authed_get("/api/info"),
        Request::builder()
            .uri("/api/info")
            .header(TOKEN_HEADER, TEST_TOKEN)
            .body(Body::empty())
            .unwrap(),
        Request::builder()
            .uri(format!("/api/info?token={TEST_TOKEN}"))
            .body(Body::empty())
            .unwrap(),
    ] {
        let (status, body) = send_json(&app, req).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["version"], 1);
        assert_eq!(body["sandboxed"], true);
        assert_eq!(body["defaultRowLimit"], 10_000);
    }

    // Disallowed Origin rejected with 403 even with valid token.
    for bad_origin in ["https://evil.example", "http://127.0.0.1:9999", "null"] {
        let req = Request::builder()
            .uri("/api/info")
            .header(header::AUTHORIZATION, format!("Bearer {TEST_TOKEN}"))
            .header(header::ORIGIN, bad_origin)
            .body(Body::empty())
            .unwrap();
        let (status, body) = send_json(&app, req).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{bad_origin}: {body}");
        assert_eq!(body["error"]["code"], "forbidden_origin");
    }

    // Allowed Origin succeeds.
    for ok_origin in ["http://127.0.0.1:4321", "http://localhost:4321"] {
        let req = Request::builder()
            .uri("/api/info")
            .header(header::AUTHORIZATION, format!("Bearer {TEST_TOKEN}"))
            .header(header::ORIGIN, ok_origin)
            .body(Body::empty())
            .unwrap();
        let (status, _) = send_json(&app, req).await;
        assert_eq!(status, StatusCode::OK, "{ok_origin}");
    }

    // Non-loopback Host header rejected (DNS rebinding protection).
    let req = Request::builder()
        .uri("/api/info")
        .header(header::AUTHORIZATION, format!("Bearer {TEST_TOKEN}"))
        .header(header::HOST, "evil.example:4321")
        .body(Body::empty())
        .unwrap();
    let (status, body) = send_json(&app, req).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"]["code"], "forbidden_host");

    // Unknown /api/* route returns 404 JSON, never index.html.
    let (status, body) = send_json(&app, authed_get("/api/does-not-exist")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "not_found");
}

#[tokio::test]
async fn serves_embedded_placeholder_and_spa_fallback() {
    let (_dir, root) = make_workspace();
    let app = make_router(&root, |_| {});

    let (status, headers, bytes) = send(
        &app,
        Request::builder().uri("/").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        headers[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("text/html")
    );
    let html = String::from_utf8(bytes).unwrap();
    assert!(html.contains("DataScript"));

    // Extensionless SPA route falls back to index.html.
    let (status, _, bytes) = send(
        &app,
        Request::builder()
            .uri("/workspace/1")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(String::from_utf8(bytes).unwrap().contains("DataScript"));

    // Missing file with extension returns 404.
    let (status, _, _) = send(
        &app,
        Request::builder()
            .uri("/missing.js")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn dev_proxy_forwards_frontend_and_allows_dev_origin() {
    let (_dir, root) = make_workspace();

    // Start a tiny mock Vite dev server.
    let vite_app = Router::new().route("/", get(|| async { "mock-vite-index" }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let vite_addr = listener.local_addr().unwrap();
    let vite_origin = format!("http://{vite_addr}");
    let (stop_tx, stop_rx) = oneshot::channel::<()>();
    let vite_task = tokio::spawn(async move {
        axum::serve(listener, vite_app)
            .with_graceful_shutdown(async {
                let _ = stop_rx.await;
            })
            .await
            .unwrap();
    });

    let app = make_router(&root, |cfg| {
        cfg.dev_proxy = Some(vite_origin.clone());
    });

    // `/` is proxied to mock Vite.
    let (status, _, bytes) = send(
        &app,
        Request::builder().uri("/").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(String::from_utf8(bytes).unwrap(), "mock-vite-index");

    // `/api/info` stays local and accepts the Vite origin.
    let req = Request::builder()
        .uri("/api/info")
        .header(header::AUTHORIZATION, format!("Bearer {TEST_TOKEN}"))
        .header(header::ORIGIN, &vite_origin)
        .body(Body::empty())
        .unwrap();
    let (status, body) = send_json(&app, req).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["version"], 1);

    let _ = stop_tx.send(());
    let _ = vite_task.await;
}

// ---- /api/check and /api/run ----

#[tokio::test]
async fn check_returns_diagnostics_utf16_spans_and_query_metadata() {
    let (_dir, root) = make_workspace();
    let app = make_router(&root, |_| {});

    let (status, body) = send_json(
        &app,
        authed_post(
            "/api/check",
            json!({
                "query": "from 'orders.csv' | summarize total = sum(amount) by customerId | render columnchart"
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["hasErrors"], false);
    assert_eq!(body["diagnostics"].as_array().unwrap().len(), 0);
    let queries = body["queries"].as_array().unwrap();
    assert_eq!(queries.len(), 1);
    assert_eq!(queries[0]["index"], 0);
    assert_eq!(queries[0]["render"]["kind"], "columnchart");

    // Emoji before an unknown column verifies UTF-16 code-unit columns (D-025).
    // `// 😀\n` is 1 line; `from 'orders.csv' | project amout` is line 2, column 29.
    let (status, body) = send_json(
        &app,
        authed_post(
            "/api/check",
            json!({
                "query": "// 😀\nfrom 'orders.csv' | project amout",
                "filename": "emoji.ds"
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["hasErrors"], true);
    assert_eq!(diag_codes(&body), ["DS1001"]);
    let span = &body["diagnostics"][0]["primary"]["span"];
    assert_eq!(span["file"], "emoji.ds");
    assert_eq!(span["line"], 2);
    // `// 😀\n` is 6 UTF-16 code units (`/` `/` ` ` high-surrogate low-surrogate `\n`).
    assert_eq!(span["start"], 6 + 28);
}

#[tokio::test]
async fn run_supports_json_arrow_render_statement_selection_and_truncation() {
    let (_dir, root) = make_workspace();
    let app = make_router(&root, |cfg| {
        cfg.default_row_limit = Some(3);
    });

    // 1. JSON rows + RenderSpec.
    let (status, body) = send_json(
        &app,
        authed_post(
            "/api/run",
            json!({
                "query": "from 'orders.csv' | summarize total = sum(amount) by customerId | sort by customerId asc | render barchart with (title = 'Revenue')"
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["hasErrors"], false);
    assert!(body["diagnostics"].as_array().unwrap().is_empty());
    let res = &body["results"][0];
    assert_eq!(res["statement"], 0);
    assert_eq!(res["rowCount"], 2);
    assert_eq!(res["truncated"], false);
    assert_eq!(res["render"]["kind"], "barchart");
    assert_eq!(res["render"]["title"], "Revenue");
    assert_eq!(res["rows"][0]["customerId"], "c-1");
    assert_eq!(res["rows"][0]["total"], 15.75);

    // 2. Arrow IPC format decoded back with `arrow_ipc::reader::FileReader`.
    let (status, body) = send_json(
        &app,
        authed_post(
            "/api/run",
            json!({
                "query": "from 'numbers.csv' | sort by n asc | take 2",
                "format": "arrow"
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let res = &body["results"][0];
    assert!(res.get("rows").is_none());
    let b64 = res["arrowIpc"].as_str().unwrap();
    let ipc_bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .unwrap();
    let mut reader = arrow_ipc::reader::FileReader::try_new(Cursor::new(ipc_bytes), None).unwrap();
    let batch = reader.next().unwrap().unwrap();
    assert_eq!(batch.num_rows(), 2);
    let col = batch
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .unwrap();
    assert_eq!(col.values(), &[1, 2]);

    // 3. Default row limit (3) truncates `numbers.csv` (7 rows) and emits DS4060.
    let (status, body) = send_json(
        &app,
        authed_post(
            "/api/run",
            json!({ "query": "from 'numbers.csv' | sort by n asc" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["hasErrors"], false);
    assert_eq!(body["results"][0]["rowCount"], 3);
    assert_eq!(body["results"][0]["truncated"], true);
    assert_eq!(diag_codes(&body), ["DS4060"]);
    assert_eq!(body["diagnostics"][0]["severity"], "info");
    assert_eq!(
        body["diagnostics"][0]["message"],
        "result truncated to 3 rows"
    );

    // 4. `rowLimit: 0` returns all 7 rows untruncated with no DS4060.
    let (status, body) = send_json(
        &app,
        authed_post(
            "/api/run",
            json!({
                "query": "from 'numbers.csv' | sort by n asc",
                "rowLimit": 0
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["results"][0]["rowCount"], 7);
    assert_eq!(body["results"][0]["truncated"], false);
    assert!(body["diagnostics"].as_array().unwrap().is_empty());

    // 5. Multi-statement selection & out-of-range check.
    let (status, body) = send_json(
        &app,
        authed_post(
            "/api/run",
            json!({
                "query": "from 'numbers.csv' | take 1; from 'orders.csv' | count",
                "statement": 1
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["results"].as_array().unwrap().len(), 1);
    assert_eq!(body["results"][0]["statement"], 1);
    assert_eq!(body["results"][0]["rows"][0]["Count"], 3);

    let (status, body) = send_json(
        &app,
        authed_post(
            "/api/run",
            json!({
                "query": "from 'numbers.csv' | take 1",
                "statement": 5
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "invalid_statement");

    // 6. Runtime error returns DS4003.
    let (status, body) = send_json(
        &app,
        authed_post(
            "/api/run",
            json!({ "query": "from 'numbers.csv' | extend x = n / (n - n)" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["hasErrors"], true);
    assert_eq!(diag_codes(&body), ["DS4003"]);
}

// ---- D-016 Sandbox acceptance criteria ----

#[tokio::test]
async fn sandbox_rejects_etc_passwd_parent_dir_and_symlinks_with_ds3007() {
    let (dir, root) = make_workspace();

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(dir.path().join("outside.csv"), root.join("escape.csv"))
            .unwrap();
        std::os::unix::fs::symlink(dir.path(), root.join("escape_dir")).unwrap();
    }

    let app = make_router(&root, |_| {});

    let mut cases = vec![
        "from '/etc/passwd' | take 1",
        "from '../outside.csv'",
        "from '../x.csv'",
        "from '../*.csv'",
        "from '../*.nope.csv'",
        "from read_csv('/etc/passwd')",
    ];
    #[cfg(unix)]
    {
        cases.push("from 'escape.csv' | take 1");
        cases.push("from 'escape_dir/outside.csv'");
        cases.push("from 'escape_dir/missing.csv'");
    }

    for query in cases {
        let (status, body) =
            send_json(&app, authed_post("/api/run", json!({ "query": query }))).await;
        assert_eq!(status, StatusCode::OK, "{query}: {body}");
        assert_eq!(body["hasErrors"], true, "{query}: {body}");
        assert_eq!(diag_codes(&body), ["DS3007"], "{query}: {body}");
        assert!(body["results"].as_array().unwrap().is_empty());
    }

    // Without `from`, the query fails parsing with DS0150 and never reaches the sandbox (D-036).
    let (status, body) = send_json(
        &app,
        authed_post("/api/run", json!({ "query": "'/etc/passwd' | take 1" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["hasErrors"], true);
    assert_eq!(diag_codes(&body), ["DS0150"]);

    // With `--allow-outside-root` (`sandboxed = false`), `../outside.csv` is readable.
    let unsandboxed = make_router(&root, |cfg| {
        cfg.sandboxed = false;
    });
    let (status, body) = send_json(
        &unsandboxed,
        authed_post("/api/run", json!({ "query": "from '../outside.csv'" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["hasErrors"], false, "{body}");
    assert_eq!(body["results"][0]["rows"][0]["secret"], 42);
}

// ---- /api/files and /api/schema ----

#[tokio::test]
async fn files_lists_root_and_blocks_traversal_and_outside_symlinks() {
    let (dir, root) = make_workspace();

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(dir.path().join("outside.csv"), root.join("escape.csv"))
            .unwrap();
        std::os::unix::fs::symlink(dir.path(), root.join("escape_dir")).unwrap();
    }

    let app = make_router(&root, |_| {});

    let (status, body) = send_json(&app, authed_get("/api/files")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let entries = body["entries"].as_array().unwrap();
    let names: Vec<&str> = entries
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    // Directories first, then recognized data files; dotfiles, .txt, and outside symlinks excluded.
    assert_eq!(names, ["sub", "numbers.csv", "orders.csv"]);
    assert_eq!(entries[0]["kind"], "directory");
    assert_eq!(entries[1]["format"], "csv");

    // Subdirectory listing works.
    let (status, body) = send_json(&app, authed_get("/api/files?dir=sub")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["dir"], "sub");
    assert_eq!(body["entries"][0]["path"], "sub/nested.csv");

    // Path traversal attempts all fail with 403.
    let mut bad_dirs = vec![
        "/api/files?dir=..",
        "/api/files?dir=../outside",
        "/api/files?dir=sub/../../outside",
        "/api/files?dir=/etc",
        "/api/files?dir=..\\outside",
    ];
    #[cfg(unix)]
    {
        bad_dirs.push("/api/files?dir=escape_dir");
    }
    for uri in bad_dirs {
        let (status, body) = send_json(&app, authed_get(uri)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{uri}: {body}");
        assert_eq!(body["error"]["code"], "outside_root", "{uri}: {body}");
    }
}

#[tokio::test]
async fn schema_inspects_files_tables_and_enforces_sandbox() {
    let (_dir, root) = make_workspace();
    let engine = DataFusionEngine::default().with_table(
        "orders_tbl",
        SourceRef::for_path(root.join("orders.csv").to_string_lossy()),
    );
    let session = Arc::new(Session::new(Arc::new(engine), Arc::new(FixedClock(0))));
    let mut cfg = ServerConfig::new(&root);
    cfg.token = Some(TEST_TOKEN.into());
    cfg.watch_files = false;
    cfg.tables = vec!["orders_tbl".into()];
    let app = router(Arc::new(AppState::new(session, &cfg, 4321)));

    // File schema.
    let (status, body) = send_json(&app, authed_get("/api/schema?source=orders.csv")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["columns"],
        json!([
            { "name": "orderId", "type": "string" },
            { "name": "customerId", "type": "string" },
            { "name": "amount", "type": "real" },
        ])
    );

    // Host table schema.
    let (status, body) = send_json(&app, authed_get("/api/schema?source=orders_tbl")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["columns"].as_array().unwrap().len(), 3);

    // Outside sandbox -> 403 + DS3007.
    for uri in [
        "/api/schema?source=../outside.csv",
        "/api/schema?source=/etc/passwd",
    ] {
        let (status, body) = send_json(&app, authed_get(uri)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{uri}: {body}");
        assert_eq!(diag_codes(&body), ["DS3007"]);
    }

    // Missing file inside root -> 404 + DS3001.
    let (status, body) = send_json(&app, authed_get("/api/schema?source=nope.csv")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(diag_codes(&body), ["DS3001"]);

    // Unknown extension inside root -> 400 + DS3002.
    let (status, body) = send_json(&app, authed_get("/api/schema?source=readme.txt")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(diag_codes(&body), ["DS3002"]);
}

// ---- /api/export (Q-007 full untruncated streaming export) ----

#[tokio::test]
async fn export_streams_all_rows_in_csv_json_ndjson_and_arrow() {
    let (_dir, root) = make_workspace();
    // Set default row limit to 2 to prove export ignores it and writes all 7 rows.
    let app = make_router(&root, |cfg| {
        cfg.default_row_limit = Some(2);
    });

    // CSV
    let (status, headers, bytes) = send(
        &app,
        authed_post(
            "/api/export",
            json!({ "query": "from 'numbers.csv' | sort by n asc", "format": "csv" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        headers[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("text/csv")
    );
    assert_eq!(
        String::from_utf8(bytes).unwrap(),
        "n\n1\n2\n3\n4\n5\n6\n7\n"
    );

    // JSON
    let (status, _, bytes) = send(
        &app,
        authed_post(
            "/api/export",
            json!({ "query": "from 'numbers.csv' | sort by n asc", "format": "json" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let parsed: Vec<Value> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(parsed.len(), 7);
    assert_eq!(parsed[6]["n"], 7);

    // NDJSON
    let (status, _, bytes) = send(
        &app,
        authed_post(
            "/api/export",
            json!({ "query": "from 'numbers.csv' | sort by n asc", "format": "ndjson" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let text = String::from_utf8(bytes).unwrap();
    assert_eq!(text.lines().count(), 7);

    // Arrow IPC file
    let (status, headers, bytes) = send(
        &app,
        authed_post(
            "/api/export",
            json!({ "query": "from 'numbers.csv' | sort by n asc", "format": "arrow" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers[header::CONTENT_TYPE],
        "application/vnd.apache.arrow.file"
    );
    let reader = arrow_ipc::reader::FileReader::try_new(Cursor::new(bytes), None).unwrap();
    let total_rows: usize = reader.map(|b| b.unwrap().num_rows()).sum();
    assert_eq!(total_rows, 7);

    // Sandbox violation returns 422 + DS3007 JSON.
    let (status, body) = send_json(
        &app,
        authed_post(
            "/api/export",
            json!({ "query": "from '../outside.csv'", "format": "csv" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(diag_codes(&body), ["DS3007"]);
}

// ---- Concurrent requests on one shared Arc<Session> (D-011) ----

#[tokio::test]
async fn concurrent_requests_share_one_session() {
    let (_dir, root) = make_workspace();
    let app = make_router(&root, |_| {});

    let mut handles = Vec::new();
    for i in 0..8 {
        let app = app.clone();
        handles.push(tokio::spawn(async move {
            let (status, body) = send_json(
                &app,
                authed_post(
                    "/api/run",
                    json!({ "query": format!("from 'numbers.csv' | where n > {}", i % 4) }),
                ),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(body["hasErrors"], false);
            assert_eq!(body["results"][0]["rowCount"], 7 - (i % 4));
        }));
    }
    for h in handles {
        h.await.unwrap();
    }
}

// ---- WebSocket streaming and cancellation ----

/// A fake engine whose result stream yields one batch immediately and then blocks until dropped,
/// setting `cancelled` when dropped. Used to verify WebSocket mid-query cancellation.
struct SlowEngine {
    cancelled: Arc<AtomicBool>,
    started: Arc<tokio::sync::Notify>,
}

struct BlockingBatchStream {
    sent_first: bool,
    schema: SchemaRef,
    cancelled: Arc<AtomicBool>,
    started: Arc<tokio::sync::Notify>,
}

impl Drop for BlockingBatchStream {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
}

impl Stream for BlockingBatchStream {
    type Item = Result<RecordBatch, Diagnostic>;

    fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if !self.sent_first {
            self.sent_first = true;
            self.started.notify_one();
            let batch = RecordBatch::try_new(
                self.schema.clone(),
                vec![Arc::new(Int64Array::from(vec![1, 2]))],
            )
            .unwrap();
            Poll::Ready(Some(Ok(batch)))
        } else {
            Poll::Pending
        }
    }
}

#[async_trait::async_trait]
impl Engine for SlowEngine {
    fn name(&self) -> &str {
        "slow-test"
    }

    fn capabilities(&self) -> &Capabilities {
        static CAPS: std::sync::OnceLock<Capabilities> = std::sync::OnceLock::new();
        CAPS.get_or_init(Capabilities::full)
    }

    async fn schema_of(
        &self,
        _source: &SourceRef,
        _ctx: &ExecContext,
    ) -> Result<SchemaRef, SourceError> {
        Ok(Arc::new(Schema::new(vec![Field::new(
            "n",
            DataType::Int64,
            false,
        )])))
    }

    async fn execute(
        &self,
        plan: &datascript_compiler::ir::QueryPlan,
        _ctx: &ExecContext,
    ) -> Result<ResultStream, Diagnostic> {
        let schema = plan.output_schema.clone();
        Ok(ResultStream {
            schema: schema.clone(),
            batches: Box::pin(BlockingBatchStream {
                sent_first: false,
                schema,
                cancelled: self.cancelled.clone(),
                started: self.started.clone(),
            }),
        })
    }
}

#[tokio::test]
async fn websocket_streams_batches_enforces_auth_and_truncates() {
    let (_dir, root) = make_workspace();
    let session = make_session(DataFusionConfig {
        batch_size: 2,
        ..DataFusionConfig::default()
    });
    let mut cfg = ServerConfig::new(&root);
    cfg.watch_files = false;
    let server = bind(session, cfg).await.unwrap();
    let addr = server.addr();
    let token = server.token().to_string();

    let (stop_tx, stop_rx) = oneshot::channel::<()>();
    let server_task = tokio::spawn(async move {
        server
            .serve_with_graceful_shutdown(async {
                let _ = stop_rx.await;
            })
            .await
            .unwrap();
    });

    // Missing token fails WebSocket upgrade with 401.
    let err = tokio_tungstenite::connect_async(format!("ws://{addr}/api/ws"))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("401"), "{err}");

    // Bad Origin fails WebSocket upgrade with 403.
    let mut bad_req = format!("ws://{addr}/api/ws?token={token}")
        .into_client_request()
        .unwrap();
    bad_req
        .headers_mut()
        .insert(header::ORIGIN, "https://evil.example".parse().unwrap());
    let err = tokio_tungstenite::connect_async(bad_req).await.unwrap_err();
    assert!(err.to_string().contains("403"), "{err}");

    // Valid token + valid Origin succeeds and streams multiple batches, truncating at 3.
    let mut ok_req = format!("ws://{addr}/api/ws?token={token}")
        .into_client_request()
        .unwrap();
    ok_req
        .headers_mut()
        .insert(header::ORIGIN, format!("http://{addr}").parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(ok_req).await.unwrap();

    ws.send(WsMessage::Text(
        json!({
            "type": "run",
            "id": "r1",
            "query": "from 'numbers.csv' | sort by n asc",
            "rowLimit": 3
        })
        .to_string()
        .into(),
    ))
    .await
    .unwrap();

    let mut events = Vec::new();
    while let Some(msg) = ws.next().await {
        let WsMessage::Text(text) = msg.unwrap() else {
            continue;
        };
        let v: Value = serde_json::from_str(&text).unwrap();
        let is_done = v["type"] == "done";
        events.push(v);
        if is_done {
            break;
        }
    }

    assert_eq!(events[0]["type"], "queryStart");
    let total_rows: usize = events
        .iter()
        .filter(|e| e["type"] == "batch")
        .map(|e| e["rowCount"].as_u64().unwrap() as usize)
        .sum();
    assert_eq!(total_rows, 3);
    assert!(
        events
            .iter()
            .any(|e| e["type"] == "diagnostics" && e["diagnostics"][0]["code"] == "DS4060"),
        "{events:#?}"
    );
    let end = events.iter().find(|e| e["type"] == "queryEnd").unwrap();
    assert_eq!(end["truncated"], true);
    assert_eq!(end["rowCount"], 3);

    let _ = ws.close(None).await;
    let _ = stop_tx.send(());
    let _ = server_task.await;
}

#[tokio::test]
async fn websocket_cancel_drops_running_query_stream() {
    let (_dir, root) = make_workspace();
    let cancelled = Arc::new(AtomicBool::new(false));
    let started = Arc::new(tokio::sync::Notify::new());
    let engine = SlowEngine {
        cancelled: cancelled.clone(),
        started: started.clone(),
    };
    let session = Arc::new(Session::new(Arc::new(engine), Arc::new(FixedClock(0))));
    let mut cfg = ServerConfig::new(&root);
    cfg.watch_files = false;
    let server = bind(session, cfg).await.unwrap();
    let addr = server.addr();
    let token = server.token().to_string();

    let (stop_tx, stop_rx) = oneshot::channel::<()>();
    let server_task = tokio::spawn(async move {
        server
            .serve_with_graceful_shutdown(async {
                let _ = stop_rx.await;
            })
            .await
            .unwrap();
    });

    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/api/ws?token={token}"))
        .await
        .unwrap();

    ws.send(WsMessage::Text(
        json!({
            "type": "run",
            "id": "slow-1",
            "query": "from 'orders.csv'"
        })
        .to_string()
        .into(),
    ))
    .await
    .unwrap();

    // Wait until the first batch has been produced and sent.
    tokio::time::timeout(Duration::from_secs(5), started.notified())
        .await
        .expect("query should start");

    // Read queryStart and the first batch.
    let mut saw_batch = false;
    for _ in 0..2 {
        let WsMessage::Text(text) = ws.next().await.unwrap().unwrap() else {
            panic!("expected text frame");
        };
        let v: Value = serde_json::from_str(&text).unwrap();
        if v["type"] == "batch" {
            saw_batch = true;
        }
    }
    assert!(saw_batch);
    assert!(!cancelled.load(Ordering::SeqCst));

    // Send cancel and confirm the stream is dropped and `cancelled` ack arrives.
    ws.send(WsMessage::Text(
        json!({ "type": "cancel", "id": "slow-1" })
            .to_string()
            .into(),
    ))
    .await
    .unwrap();

    let ack = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(msg) = ws.next().await {
            if let WsMessage::Text(text) = msg.unwrap() {
                let v: Value = serde_json::from_str(&text).unwrap();
                if v["type"] == "cancelled" {
                    return v;
                }
            }
        }
        panic!("socket closed before cancelled ack");
    })
    .await
    .expect("cancel ack should arrive promptly");
    assert_eq!(ack["id"], "slow-1");

    // Give the aborted task a brief moment to run Drop.
    tokio::time::timeout(Duration::from_secs(2), async {
        while !cancelled.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("aborting the task should drop BlockingBatchStream");

    let _ = ws.close(None).await;
    let _ = stop_tx.send(());
    let _ = server_task.await;
}

// ---- File watcher invalidates schema cache ----

#[tokio::test]
async fn root_file_watcher_invalidates_cached_failed_lookup() {
    let (_dir, root) = make_workspace();
    let session = make_session(DataFusionConfig::default());
    let mut cfg = ServerConfig::new(&root);
    cfg.watch_files = true;
    let server = bind(session, cfg).await.unwrap();
    let app = router(server.state().clone());
    let token = server.token().to_string();

    let run_later = |app: Router, token: String| async move {
        let req = Request::builder()
            .method(Method::POST)
            .uri("/api/run")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({ "query": "from 'later.csv'" }).to_string(),
            ))
            .unwrap();
        send_json(&app, req).await
    };

    // Initially missing -> DS3001 is cached in Session.
    let (_, body) = run_later(app.clone(), token.clone()).await;
    assert_eq!(diag_codes(&body), ["DS3001"]);

    // Create the file; the watcher should invalidate the schema cache automatically.
    std::fs::write(root.join("later.csv"), "a\n99\n").unwrap();

    let ok = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let (_, body) = run_later(app.clone(), token.clone()).await;
            if body["hasErrors"] == false {
                return body;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("file watcher should invalidate the cached DS3001 after later.csv is created");
    assert_eq!(ok["results"][0]["rows"][0]["a"], 99);
}
