//! In-process LSP tests: a JSON-RPC client talks to the server over an in-memory pipe, exactly as
//! an editor would over stdio, with the DataFusion engine reading real files.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use arrow_schema::SchemaRef;
use datascript_compiler::capabilities::Capabilities;
use datascript_compiler::ir::QueryPlan;
use datascript_compiler::{SourceError, SourceRef};
use datascript_datafusion::DataFusionEngine;
use datascript_diagnostics::Diagnostic;
use datascript_engine::{Engine, ExecContext, FixedClock, ResultStream, Session};
use datascript_lsp::Config;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, DuplexStream, WriteHalf};
use tokio::sync::{Mutex, mpsc};

/// How long to wait for a message the server must send.
const TIMEOUT: Duration = Duration::from_secs(20);

/// Settings for tests that expect one publish per check: the grace is long enough for any cold
/// schema read, so syntax diagnostics are never published on their own.
fn one_publish(debounce_ms: u64) -> Config {
    Config {
        debounce: Duration::from_millis(debounce_ms),
        syntax_grace: Duration::from_secs(30),
    }
}

/// A client speaking JSON-RPC to an in-process server.
struct Client {
    writer: Arc<Mutex<WriteHalf<DuplexStream>>>,
    incoming: mpsc::UnboundedReceiver<Value>,
    /// Notifications received while waiting for something else.
    queued: VecDeque<Value>,
    /// Requests the server sent us (answered with `null`), by method.
    server_requests: Arc<std::sync::Mutex<Vec<String>>>,
    next_id: i64,
    /// The workspace folder, a temporary directory holding the test's files.
    dir: tempfile::TempDir,
}

impl Client {
    async fn start(engine: Arc<dyn Engine>, config: Config, watch: bool) -> Client {
        Client::start_with(engine, config, watch, Value::Null).await
    }

    /// Starts a server, passing `options` as the `initializationOptions`.
    async fn start_with(
        engine: Arc<dyn Engine>,
        config: Config,
        watch: bool,
        options: Value,
    ) -> Client {
        Client::start_full(engine, config, watch, options, true).await
    }

    /// Starts a server; without `folder`, the client opens no workspace folder.
    async fn start_full(
        engine: Arc<dyn Engine>,
        config: Config,
        watch: bool,
        options: Value,
        folder: bool,
    ) -> Client {
        let (client_end, server_end) = tokio::io::duplex(1 << 20);
        let (server_read, server_write) = tokio::io::split(server_end);
        let session = Arc::new(Session::new(engine, Arc::new(FixedClock(0))));
        tokio::spawn(datascript_lsp::serve(
            server_read,
            server_write,
            session,
            config,
        ));
        let (read, write) = tokio::io::split(client_end);
        let writer = Arc::new(Mutex::new(write));
        let (sender, incoming) = mpsc::unbounded_channel();
        let server_requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        tokio::spawn(read_messages(
            BufReader::new(read),
            sender,
            Arc::clone(&writer),
            Arc::clone(&server_requests),
        ));
        let mut client = Client {
            writer,
            incoming,
            queued: VecDeque::new(),
            server_requests,
            next_id: 0,
            dir: tempfile::tempdir().unwrap(),
        };
        let root = uri_of(client.dir.path());
        let (root, folders) = if folder {
            (json!(root), json!([{ "uri": root, "name": "w" }]))
        } else {
            (Value::Null, Value::Null)
        };
        let result = client
            .request(
                "initialize",
                json!({
                    "processId": null,
                    "rootUri": root,
                    "workspaceFolders": folders,
                    "initializationOptions": options,
                    "capabilities": {
                        "workspace": { "didChangeWatchedFiles": { "dynamicRegistration": watch } }
                    }
                }),
            )
            .await;
        assert_eq!(
            result["capabilities"]["textDocumentSync"]["change"], 2,
            "incremental sync: {result}"
        );
        client.notify("initialized", json!({})).await;
        client
    }

    async fn send(&self, message: Value) {
        let body = message.to_string();
        let mut writer = self.writer.lock().await;
        let frame = format!("Content-Length: {}\r\n\r\n{body}", body.len());
        writer.write_all(frame.as_bytes()).await.unwrap();
        writer.flush().await.unwrap();
    }

    async fn notify(&self, method: &str, params: Value) {
        self.send(json!({ "jsonrpc": "2.0", "method": method, "params": params }))
            .await;
    }

    async fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .await;
        loop {
            let message = self.receive().await;
            if message["id"] == id && message.get("method").is_none() {
                assert!(message.get("error").is_none(), "{method} failed: {message}");
                return message["result"].clone();
            }
            self.queued.push_back(message);
        }
    }

    /// Sends a request and returns the whole response, `error` included.
    async fn request_response(&mut self, method: &str, params: Value) -> Value {
        let id = self.send_request(method, params).await;
        self.response(id).await
    }

    /// Sends a request without waiting for its response; returns its id.
    async fn send_request(&mut self, method: &str, params: Value) -> i64 {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .await;
        id
    }

    /// The response to request `id`.
    async fn response(&mut self, id: i64) -> Value {
        if let Some(index) = self
            .queued
            .iter()
            .position(|m| m["id"] == id && m.get("method").is_none())
        {
            return self.queued.remove(index).unwrap();
        }
        loop {
            let message = self.receive().await;
            if message["id"] == id && message.get("method").is_none() {
                return message;
            }
            self.queued.push_back(message);
        }
    }

    /// `datascript/run` on `uri`: the output, or the error's message.
    async fn run(&mut self, uri: &str, statement: Option<usize>) -> Result<String, String> {
        let response = self
            .request_response(
                "datascript/run",
                json!({ "textDocument": { "uri": uri }, "statement": statement }),
            )
            .await;
        match response.get("error") {
            Some(error) => Err(error["message"].as_str().unwrap().to_owned()),
            None => Ok(response["result"]["output"].as_str().unwrap().to_owned()),
        }
    }

    async fn receive(&mut self) -> Value {
        tokio::time::timeout(TIMEOUT, self.incoming.recv())
            .await
            .expect("the server sent nothing in time")
            .expect("the server closed the connection")
    }

    /// The next `publishDiagnostics` for `uri`.
    async fn diagnostics(&mut self, uri: &str) -> Published {
        let is_for_uri = |m: &Value| {
            m["method"] == "textDocument/publishDiagnostics" && m["params"]["uri"] == uri
        };
        if let Some(index) = self.queued.iter().position(is_for_uri) {
            return Published::from(self.queued.remove(index).unwrap());
        }
        loop {
            let message = self.receive().await;
            if is_for_uri(&message) {
                return Published::from(message);
            }
            self.queued.push_back(message);
        }
    }

    /// Writes a file into the workspace folder and returns its URI.
    fn write(&self, name: &str, text: &str) -> String {
        let path = self.dir.path().join(name);
        std::fs::write(&path, text).unwrap();
        uri_of(&path)
    }

    fn uri(&self, name: &str) -> String {
        uri_of(&self.dir.path().join(name))
    }

    async fn open(&self, uri: &str, text: &str) {
        self.notify(
            "textDocument/didOpen",
            json!({ "textDocument": { "uri": uri, "languageId": "datascript", "version": 1, "text": text } }),
        )
        .await;
    }

    /// Replaces `[start, end)` (line, UTF-16 character) with `text`.
    async fn edit(&self, uri: &str, version: i32, start: (u32, u32), end: (u32, u32), text: &str) {
        self.notify(
            "textDocument/didChange",
            json!({
                "textDocument": { "uri": uri, "version": version },
                "contentChanges": [{
                    "range": {
                        "start": { "line": start.0, "character": start.1 },
                        "end": { "line": end.0, "character": end.1 }
                    },
                    "text": text
                }]
            }),
        )
        .await;
    }

    async fn replace_all(&self, uri: &str, version: i32, text: &str) {
        self.notify(
            "textDocument/didChange",
            json!({
                "textDocument": { "uri": uri, "version": version },
                "contentChanges": [{ "text": text }]
            }),
        )
        .await;
    }

    async fn shutdown(mut self) {
        self.request("shutdown", Value::Null).await;
        self.notify("exit", Value::Null).await;
    }
}

/// Reads framed messages, answers the server's requests with `null`, and forwards the rest.
async fn read_messages(
    mut reader: BufReader<tokio::io::ReadHalf<DuplexStream>>,
    sender: mpsc::UnboundedSender<Value>,
    writer: Arc<Mutex<WriteHalf<DuplexStream>>>,
    server_requests: Arc<std::sync::Mutex<Vec<String>>>,
) {
    loop {
        let mut length = None;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                return;
            }
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if let Some(value) = line.strip_prefix("Content-Length: ") {
                length = Some(value.parse::<usize>().unwrap());
            }
        }
        let mut body = vec![0; length.expect("a Content-Length header")];
        reader.read_exact(&mut body).await.unwrap();
        let message: Value = serde_json::from_slice(&body).unwrap();
        if let (Some(id), Some(method)) = (message.get("id"), message["method"].as_str()) {
            server_requests.lock().unwrap().push(method.to_owned());
            let reply = json!({ "jsonrpc": "2.0", "id": id, "result": null }).to_string();
            let frame = format!("Content-Length: {}\r\n\r\n{reply}", reply.len());
            let mut writer = writer.lock().await;
            writer.write_all(frame.as_bytes()).await.unwrap();
            writer.flush().await.unwrap();
            continue;
        }
        if sender.send(message).is_err() {
            return;
        }
    }
}

fn uri_of(path: &Path) -> String {
    tower_lsp_server::ls_types::Uri::from_file_path(path)
        .unwrap()
        .as_str()
        .to_owned()
}

/// One `publishDiagnostics` notification.
#[derive(Debug)]
struct Published {
    version: Option<i64>,
    diagnostics: Vec<Value>,
}

impl From<Value> for Published {
    fn from(message: Value) -> Self {
        let params = &message["params"];
        Published {
            version: params["version"].as_i64(),
            diagnostics: params["diagnostics"].as_array().unwrap().clone(),
        }
    }
}

impl Published {
    fn codes(&self) -> Vec<&str> {
        self.diagnostics
            .iter()
            .map(|d| d["code"].as_str().unwrap())
            .collect()
    }

    /// `line:character-line:character` of each diagnostic, 0-based as in the LSP.
    fn ranges(&self) -> Vec<String> {
        self.diagnostics
            .iter()
            .map(|d| {
                let r = &d["range"];
                format!(
                    "{}:{}-{}:{}",
                    r["start"]["line"],
                    r["start"]["character"],
                    r["end"]["line"],
                    r["end"]["character"]
                )
            })
            .collect()
    }
}

fn datafusion() -> Arc<dyn Engine> {
    Arc::new(DataFusionEngine::default())
}

const ORDERS: &str = "orderId,customerId,amount\n1,c-1,10.5\n2,c-2,20\n";

#[tokio::test(flavor = "multi_thread")]
async fn open_change_close() {
    let mut client = Client::start(datafusion(), one_publish(20), false).await;
    client.write("orders.csv", ORDERS);
    let uri = client.uri("query.ds");

    // A typo in a column name: the semantic check reads the file and reports DS1001.
    client
        .open(&uri, "from 'orders.csv'\n| summarize total = sum(amout)\n")
        .await;
    let published = client.diagnostics(&uri).await;
    assert_eq!(published.version, Some(1));
    assert_eq!(published.codes(), ["DS1001"]);
    assert_eq!(published.ranges(), ["1:24-1:29"]);
    let d = &published.diagnostics[0];
    assert_eq!(d["severity"], 1);
    assert_eq!(d["source"], "datascript");
    assert!(
        d["message"].as_str().unwrap().starts_with(
            "unknown column `amout`\nhelp: a column with a similar name exists: `amount`"
        ),
        "{d}"
    );
    assert!(
        d["codeDescription"]["href"]
            .as_str()
            .unwrap()
            .ends_with("DIAGNOSTICS.md#code-registry")
    );

    // Fixing it with an incremental edit clears the diagnostics.
    client.edit(&uri, 2, (1, 24), (1, 29), "amount").await;
    let published = client.diagnostics(&uri).await;
    assert_eq!(published.version, Some(2));
    assert_eq!(published.codes(), Vec::<&str>::new());

    // A syntax error, a warning and a semantic error after a non-ASCII line, in one check.
    client
        .replace_all(
            &uri,
            3,
            "// prix en €, 😀\nfrom 'orders.csv' | where amount = 10 | sort by orderId | take amout\n",
        )
        .await;
    let published = client.diagnostics(&uri).await;
    assert_eq!(published.version, Some(3));
    assert_eq!(published.codes(), ["DS0110", "DS2103", "DS2050"]);
    assert_eq!(published.ranges(), ["1:33-1:34", "1:48-1:55", "1:63-1:68"]);
    assert_eq!(published.diagnostics[1]["severity"], 2);

    // Closing the document clears its diagnostics.
    client
        .notify(
            "textDocument/didClose",
            json!({ "textDocument": { "uri": uri } }),
        )
        .await;
    let published = client.diagnostics(&uri).await;
    assert_eq!(published.codes(), Vec::<&str>::new());
    client.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_burst_of_changes_is_checked_once() {
    let mut client = Client::start(datafusion(), one_publish(300), false).await;
    client.write("orders.csv", ORDERS);
    let uri = client.uri("query.ds");
    client.open(&uri, "from 'orders.csv' | take 1\n").await;
    assert_eq!(client.diagnostics(&uri).await.codes(), Vec::<&str>::new());

    // Typing `| where x` one character at a time: every intermediate text but the last is
    // superseded before the debounce ends, so only the final one is published.
    let typed = " | where x";
    for (i, c) in typed.chars().enumerate() {
        let column = 26 + i as u32;
        client
            .edit(&uri, 2 + i as i32, (0, column), (0, column), &c.to_string())
            .await;
    }
    let published = client.diagnostics(&uri).await;
    assert_eq!(published.version, Some(1 + typed.len() as i64));
    assert_eq!(published.codes(), ["DS1001"]);
    client.shutdown().await;
}

/// An engine whose schema reads are slow, like a large file or a slow disk.
#[derive(Debug)]
struct SlowSchemas(DataFusionEngine);

#[async_trait::async_trait]
impl Engine for SlowSchemas {
    fn name(&self) -> &str {
        self.0.name()
    }

    fn capabilities(&self) -> &Capabilities {
        self.0.capabilities()
    }

    async fn schema_of(
        &self,
        source: &SourceRef,
        ctx: &ExecContext,
    ) -> Result<SchemaRef, SourceError> {
        tokio::time::sleep(Duration::from_millis(300)).await;
        self.0.schema_of(source, ctx).await
    }

    async fn execute(
        &self,
        plan: &QueryPlan,
        ctx: &ExecContext,
    ) -> Result<ResultStream, Diagnostic> {
        self.0.execute(plan, ctx).await
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn syntax_errors_show_before_schemas_resolve() {
    let config = Config {
        debounce: Duration::from_millis(10),
        syntax_grace: Duration::from_millis(20),
    };
    let engine = Arc::new(SlowSchemas(DataFusionEngine::default()));
    let mut client = Client::start(engine, config, false).await;
    client.write("orders.csv", ORDERS);
    let uri = client.uri("query.ds");
    client
        .open(
            &uri,
            "from 'orders.csv' | where amount = 1 | project amout\n",
        )
        .await;
    // The schema read takes 300 ms: the syntax error comes first, on its own...
    let first = client.diagnostics(&uri).await;
    assert_eq!(first.codes(), ["DS0110"]);
    // ...then the full set once the schema is known.
    let second = client.diagnostics(&uri).await;
    assert_eq!(second.codes(), ["DS0110", "DS1001"]);
    assert_eq!(second.version, Some(1));

    // With the schema cached, an edit is checked within the grace: one publish, complete.
    client.edit(&uri, 2, (0, 47), (0, 52), "amoun").await;
    let published = client.diagnostics(&uri).await;
    assert_eq!(published.version, Some(2));
    assert_eq!(published.codes(), ["DS0110", "DS1001"]);
    client.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn changed_data_files_invalidate_the_schema_cache() {
    let mut client = Client::start(datafusion(), one_publish(20), true).await;
    let data = client.write("orders.csv", ORDERS);
    let uri = client.uri("query.ds");
    client
        .open(
            &uri,
            "from 'orders.csv' | project region;\nfrom 'later.csv' | take 1\n",
        )
        .await;
    let published = client.diagnostics(&uri).await;
    assert_eq!(published.codes(), ["DS1001", "DS3001"]);
    assert!(
        client
            .server_requests
            .lock()
            .unwrap()
            .contains(&"client/registerCapability".to_owned()),
        "the server asks the client to watch files"
    );

    // The files change on disk. The cache still has the old answers until the client says so.
    std::fs::write(
        client.dir.path().join("orders.csv"),
        "orderId,region\n1,eu\n",
    )
    .unwrap();
    let later = client.write("later.csv", "a\n1\n");
    client.edit(&uri, 2, (2, 0), (2, 0), "\n").await;
    assert_eq!(client.diagnostics(&uri).await.codes(), ["DS1001", "DS3001"]);

    // A changed script is not data: nothing is re-checked for it.
    client
        .notify(
            "workspace/didChangeWatchedFiles",
            json!({ "changes": [{ "uri": client.uri("other.ds"), "type": 2 }] }),
        )
        .await;
    // Data files changed: the cache is cleared and open documents are checked again.
    client
        .notify(
            "workspace/didChangeWatchedFiles",
            json!({ "changes": [{ "uri": data, "type": 2 }, { "uri": later, "type": 1 }] }),
        )
        .await;
    let published = client.diagnostics(&uri).await;
    assert_eq!(
        published.version,
        Some(2),
        "the latest version is re-checked"
    );
    assert_eq!(published.codes(), Vec::<&str>::new());
    client.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn documents_that_are_not_files_resolve_paths_in_the_workspace() {
    let mut client = Client::start(datafusion(), one_publish(20), false).await;
    client.write("orders.csv", ORDERS);
    let uri = "untitled:Untitled-1";
    client
        .open(uri, "from 'orders.csv' | project amout\n")
        .await;
    assert_eq!(client.diagnostics(uri).await.codes(), ["DS1001"]);
    client.shutdown().await;
}

/// An engine that fails the test if it is asked to read anything.
#[derive(Debug)]
struct NoReads(DataFusionEngine);

#[async_trait::async_trait]
impl Engine for NoReads {
    fn name(&self) -> &str {
        self.0.name()
    }

    fn capabilities(&self) -> &Capabilities {
        self.0.capabilities()
    }

    async fn schema_of(
        &self,
        source: &SourceRef,
        _: &ExecContext,
    ) -> Result<SchemaRef, SourceError> {
        panic!("a syntax-only server read the schema of {source:?}");
    }

    async fn execute(&self, _: &QueryPlan, _: &ExecContext) -> Result<ResultStream, Diagnostic> {
        panic!("a syntax-only server ran a query");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn syntax_only_mode_reads_no_files() {
    let mut client = Client::start_with(
        Arc::new(NoReads(DataFusionEngine::default())),
        one_publish(20),
        true,
        json!({ "syntaxOnly": true }),
    )
    .await;
    client.write("orders.csv", ORDERS);
    let uri = client.uri("query.ds");

    // A syntax error and an unknown column: only the syntax error is reported, since finding the
    // other one means reading `orders.csv`.
    client
        .open(
            &uri,
            "from 'orders.csv' | where amount = 1 | project amout\n",
        )
        .await;
    let published = client.diagnostics(&uri).await;
    assert_eq!(published.version, Some(1));
    assert_eq!(published.codes(), ["DS0110"]);

    // Lexical errors are syntax too, and edits are still checked.
    client
        .replace_all(&uri, 2, "from 'orders.csv' | take 1 # x\n")
        .await;
    let published = client.diagnostics(&uri).await;
    assert_eq!(published.version, Some(2));
    assert_eq!(published.codes(), ["DS0001"]);

    // No data files are watched, and a reported change re-checks nothing.
    assert!(
        !client
            .server_requests
            .lock()
            .unwrap()
            .contains(&"client/registerCapability".to_owned()),
        "a syntax-only server does not watch files"
    );
    client
        .notify(
            "workspace/didChangeWatchedFiles",
            json!({ "changes": [{ "uri": client.uri("orders.csv"), "type": 2 }] }),
        )
        .await;
    client
        .replace_all(&uri, 3, "from 'orders.csv' | take 1\n")
        .await;
    let published = client.diagnostics(&uri).await;
    assert_eq!(published.version, Some(3));
    assert_eq!(published.codes(), Vec::<&str>::new());
    client.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_initialization_options_are_ignored() {
    // `syntaxOnly: false`, options of the wrong shape and options for other servers all leave
    // the full check on.
    for options in [
        json!({ "syntaxOnly": false }),
        json!({ "syntaxOnly": "yes", "other": 1 }),
        json!([1, 2]),
    ] {
        let mut client =
            Client::start_with(datafusion(), one_publish(20), false, options.clone()).await;
        client.write("orders.csv", ORDERS);
        let uri = client.uri("query.ds");
        client
            .open(&uri, "from 'orders.csv' | project amout\n")
            .await;
        assert_eq!(
            client.diagnostics(&uri).await.codes(),
            ["DS1001"],
            "options: {options}"
        );
        client.shutdown().await;
    }
}

/// The script the latency test checks: 100 five-line queries over a real file, 500 lines.
fn long_script() -> String {
    let mut script = String::new();
    for i in 0..100 {
        script.push_str(&format!(
            "from 'orders.csv'\n| where amount > {i} and customerId != 'c-{i}'\n\
             | extend twice = amount * 2, label = strcat(customerId, '-', tostring(orderId))\n\
             | summarize total = sum(twice), n = count() by customerId, label\n\
             | sort by total desc, customerId asc;\n"
        ));
    }
    script
}

/// Acceptance criterion: a re-check of a 500-line script with warm schemas takes under 100 ms.
/// Measured from the change to the publish, minus nothing: the debounce is zero here. The bound
/// holds for optimized builds (`cargo test --release`); debug builds only get a loose one.
#[tokio::test(flavor = "multi_thread")]
async fn recheck_latency_of_a_long_script() {
    let mut client = Client::start(datafusion(), one_publish(0), false).await;
    let data = repo_root().join("tests/data/orders.csv");
    std::fs::copy(data, client.dir.path().join("orders.csv")).unwrap();
    let uri = client.uri("long.ds");
    let script = long_script();
    assert_eq!(script.lines().count(), 500);
    client.open(&uri, &script).await;
    let published = client.diagnostics(&uri).await;
    assert_eq!(
        published.codes(),
        Vec::<&str>::new(),
        "{:?}",
        published.diagnostics
    );

    let mut times = Vec::new();
    for i in 0..10 {
        let version = 2 + i;
        // Toggle `amount > 99` and `amount > 98` in the last query, so every check sees new,
        // valid text.
        let replacement = if i % 2 == 0 { "98" } else { "99" };
        let start = Instant::now();
        client
            .edit(&uri, version, (496, 17), (496, 19), replacement)
            .await;
        let published = client.diagnostics(&uri).await;
        times.push(start.elapsed());
        assert_eq!(published.version, Some(i64::from(version)));
    }
    times.sort();
    let median = times[times.len() / 2];
    eprintln!(
        "re-check latency, 500 lines, warm schemas: median {median:?}, max {:?}",
        times[9]
    );
    let bound = if cfg!(debug_assertions) {
        Duration::from_millis(500)
    } else {
        Duration::from_millis(100)
    };
    assert!(median < bound, "median re-check took {median:?}");
    client.shutdown().await;
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

// ── Run (M3-05) ──────────────────────────────────────────────────────────────────────────────

/// A program with a `let`, two queries, a chart and a warning-free check.
const TWO_QUERIES: &str = "let big = from 'orders.csv' | where amount > 15;\n\
                           from 'orders.csv' | count;\n\
                           from big | project customerId, amount | render barchart\n";

#[tokio::test(flavor = "multi_thread")]
async fn run_prints_what_ds_run_prints() {
    let mut client = Client::start(datafusion(), one_publish(20), false).await;
    client.write("orders.csv", ORDERS);
    let uri = client.write("two.ds", TWO_QUERIES);
    client.open(&uri, TWO_QUERIES).await;
    assert_eq!(client.diagnostics(&uri).await.codes(), Vec::<&str>::new());

    // The whole file: the chart's info line first, as `ds run` writes it to standard error,
    // then a table per query. The `ds run` parity test is in the CLI's tests
    // (crates/datascript-cli/tests/lsp_run.rs), which can run the `ds` binary.
    let output = client.run(&uri, None).await.unwrap();
    let tables = "== query 1 (line 2) ==\n\
                  Count\n\
                  long\n\
                  -----\n\
                  \x20   2\n\
                  \n\
                  == query 2 (line 3) ==\n\
                  customerId | amount\n\
                  string     | real\n\
                  -----------+-------\n\
                  c-2        |   20.0\n";
    assert!(
        output.starts_with("info[DS5001]: charts are not drawn on the command line\n"),
        "{output}"
    );
    assert!(output.contains("┌─ two.ds:3:41\n"), "{output}");
    assert!(output.ends_with(tables), "{output}");
    assert!(!output.contains('\x1b'), "no color unless asked: {output}");

    // One statement, with the `let` it needs: no header, as with `ds run --statement 2`.
    let output = client.run(&uri, Some(2)).await.unwrap();
    assert!(
        output.ends_with("-----------+-------\nc-2        |   20.0\n"),
        "{output}"
    );
    assert!(!output.contains("== query"), "{output}");
    let output = client.run(&uri, Some(1)).await.unwrap();
    assert_eq!(output, "Count\nlong\n-----\n    2\n");

    // Out of range is refused with a reason.
    let error = client.run(&uri, Some(3)).await.unwrap_err();
    assert_eq!(
        error,
        "statement 3 is out of range: the document has 2 query statements"
    );
    // A document that is not open cannot run.
    let error = client
        .run(&client.uri("closed.ds"), None)
        .await
        .unwrap_err();
    assert!(error.contains("is not open"), "{error}");
    client.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn run_uses_max_rows_and_color() {
    let mut client = Client::start(datafusion(), one_publish(20), false).await;
    client.write("orders.csv", ORDERS);
    let uri = client.uri("q.ds");
    let text = "from 'orders.csv' | project orderId | where orderId > 'x'\n";
    client.open(&uri, text).await;
    client.diagnostics(&uri).await;
    let response = client
        .request(
            "datascript/run",
            json!({ "textDocument": { "uri": uri }, "color": true }),
        )
        .await;
    assert_eq!(response["failed"], true);
    let output = response["output"].as_str().unwrap();
    assert!(output.contains("\x1b["), "colored: {output:?}");

    let text = "from 'orders.csv' | project orderId\n";
    client.replace_all(&uri, 2, text).await;
    let response = client
        .request(
            "datascript/run",
            json!({ "textDocument": { "uri": uri }, "maxRows": 1 }),
        )
        .await;
    assert_eq!(response["failed"], false);
    assert_eq!(
        response["output"],
        "orderId\nlong\n-------\n      1\n… 1 more row (show them with --max-rows 2)\n"
    );
    client.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn run_uses_unsaved_edits() {
    let mut client = Client::start(datafusion(), one_publish(20), false).await;
    client.write("orders.csv", ORDERS);
    // On disk the script counts; in the editor it was changed to sum, and not saved.
    let uri = client.write("edited.ds", "from 'orders.csv' | count\n");
    client.open(&uri, "from 'orders.csv' | count\n").await;
    client.diagnostics(&uri).await;
    assert_eq!(
        client.run(&uri, None).await.unwrap(),
        "Count\nlong\n-----\n    2\n"
    );
    client
        .edit(&uri, 2, (0, 20), (0, 25), "summarize total = sum(amount)")
        .await;
    assert_eq!(
        client.run(&uri, None).await.unwrap(),
        "total\nreal\n-----\n 30.5\n"
    );
    client.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_errors_are_published_until_the_next_edit() {
    let mut client = Client::start(datafusion(), one_publish(20), false).await;
    client.write("big.csv", "x\n1\n10000\n");
    let uri = client.uri("overflow.ds");
    let text = "from 'big.csv' | count;\nfrom 'big.csv' | extend y = x * 1000000000000000\n";
    client.open(&uri, text).await;
    assert_eq!(client.diagnostics(&uri).await.codes(), Vec::<&str>::new());

    let output = client.run(&uri, None).await.unwrap();
    // The first table stays printed, and the error follows it, rendered as the CLI renders it.
    assert!(
        output.starts_with("== query 1 (line 1) ==\nCount\nlong\n-----\n    2\n"),
        "{output}"
    );
    assert!(output.contains("error[DS4003]"), "{output}");
    let published = client.diagnostics(&uri).await;
    assert_eq!(published.codes(), ["DS4003"]);
    assert_eq!(published.version, Some(1));

    // Running again publishes nothing new; the next check after an edit clears it.
    client.edit(&uri, 2, (1, 32), (1, 48), "2").await;
    assert_eq!(client.diagnostics(&uri).await.codes(), Vec::<&str>::new());
    client.shutdown().await;
}

/// An engine whose queries never finish, recording when they are dropped (cancelled).
#[derive(Debug)]
struct Endless {
    inner: DataFusionEngine,
    started: Arc<tokio::sync::Notify>,
    dropped: Arc<std::sync::atomic::AtomicBool>,
}

/// A stream that never yields, and sets a flag when it is dropped.
struct NeverEnds(Arc<std::sync::atomic::AtomicBool>);

impl futures::Stream for NeverEnds {
    type Item = Result<arrow_array::RecordBatch, Diagnostic>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        std::task::Poll::Pending
    }
}

impl Drop for NeverEnds {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl Engine for Endless {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn capabilities(&self) -> &Capabilities {
        self.inner.capabilities()
    }

    async fn schema_of(
        &self,
        source: &SourceRef,
        ctx: &ExecContext,
    ) -> Result<SchemaRef, SourceError> {
        self.inner.schema_of(source, ctx).await
    }

    async fn execute(
        &self,
        plan: &QueryPlan,
        ctx: &ExecContext,
    ) -> Result<ResultStream, Diagnostic> {
        let real = self.inner.execute(plan, ctx).await?;
        self.started.notify_one();
        Ok(ResultStream {
            schema: real.schema,
            batches: Box::pin(NeverEnds(Arc::clone(&self.dropped))),
        })
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_a_run_stops_the_query() {
    let started = Arc::new(tokio::sync::Notify::new());
    let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let engine = Arc::new(Endless {
        inner: DataFusionEngine::default(),
        started: Arc::clone(&started),
        dropped: Arc::clone(&dropped),
    });
    let mut client = Client::start(engine, one_publish(20), false).await;
    client.write("orders.csv", ORDERS);
    let uri = client.uri("slow.ds");
    client.open(&uri, "from 'orders.csv' | count\n").await;
    client.diagnostics(&uri).await;

    let id = client
        .send_request("datascript/run", json!({ "textDocument": { "uri": uri } }))
        .await;
    tokio::time::timeout(TIMEOUT, started.notified())
        .await
        .expect("the query started");
    assert!(!dropped.load(std::sync::atomic::Ordering::SeqCst));
    client.notify("$/cancelRequest", json!({ "id": id })).await;
    let response = client.response(id).await;
    assert_eq!(response["error"]["code"], -32800, "{response}");
    // The query itself was dropped, not just the reply.
    let deadline = Instant::now() + TIMEOUT;
    while !dropped.load(std::sync::atomic::Ordering::SeqCst) {
        assert!(Instant::now() < deadline, "the query kept running");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    client.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn code_lenses_sit_over_query_statements() {
    let mut client = Client::start(datafusion(), one_publish(20), false).await;
    client.write("orders.csv", ORDERS);
    let uri = client.uri("two.ds");
    client.open(&uri, TWO_QUERIES).await;
    let lenses = client
        .request(
            "textDocument/codeLens",
            json!({ "textDocument": { "uri": uri } }),
        )
        .await;
    let summary: Vec<String> = lenses
        .as_array()
        .unwrap()
        .iter()
        .map(|lens| {
            let r = &lens["range"];
            format!(
                "{}:{}-{}:{} {} {} {}",
                r["start"]["line"],
                r["start"]["character"],
                r["end"]["line"],
                r["end"]["character"],
                lens["command"]["title"].as_str().unwrap(),
                lens["command"]["command"].as_str().unwrap(),
                lens["command"]["arguments"]
            )
        })
        .collect();
    // The `let` on line 0 gets none; "Run file" sits over the first query.
    assert_eq!(
        summary,
        [
            format!("1:0-1:25 ▶ Run datascript.runStatement [\"{uri}\",1]"),
            format!("1:0-1:25 ▶ Run file datascript.runFile [\"{uri}\"]"),
            format!("2:0-2:55 ▶ Run datascript.runStatement [\"{uri}\",2]"),
        ]
    );

    // One query: no "Run file". Only `let`s: no lens.
    let one = client.uri("one.ds");
    client.open(&one, "from 'orders.csv' | count\n").await;
    let lenses = client
        .request(
            "textDocument/codeLens",
            json!({ "textDocument": { "uri": one } }),
        )
        .await;
    assert_eq!(lenses.as_array().unwrap().len(), 1);
    let lets = client.uri("lets.ds");
    client.open(&lets, "let a = 1;\n").await;
    let lenses = client
        .request(
            "textDocument/codeLens",
            json!({ "textDocument": { "uri": lets } }),
        )
        .await;
    assert_eq!(lenses, json!([]));
    client.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_syntax_only_server_runs_nothing() {
    let mut client = Client::start_with(
        Arc::new(NoReads(DataFusionEngine::default())),
        one_publish(20),
        false,
        json!({ "syntaxOnly": true }),
    )
    .await;
    let uri = client.uri("q.ds");
    client.open(&uri, "from 'orders.csv' | count\n").await;
    client.diagnostics(&uri).await;
    let lenses = client
        .request(
            "textDocument/codeLens",
            json!({ "textDocument": { "uri": uri } }),
        )
        .await;
    assert_eq!(lenses, Value::Null);
    let response = client
        .request_response("datascript/run", json!({ "textDocument": { "uri": uri } }))
        .await;
    assert_eq!(response["error"]["code"], -32803, "{response}");
    assert!(
        response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("do not trust"),
        "{response}"
    );
    client.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn untitled_documents_run_in_the_workspace_folder() {
    let mut client = Client::start(datafusion(), one_publish(20), false).await;
    client.write("orders.csv", ORDERS);
    let uri = "untitled:Untitled-1";
    client.open(uri, "from 'orders.csv' | count\n").await;
    client.diagnostics(uri).await;
    assert_eq!(
        client.run(uri, None).await.unwrap(),
        "Count\nlong\n-----\n    2\n"
    );
    client.shutdown().await;
}

/// With no folder open, relative paths of an untitled document resolve against the server's
/// working directory (D-056); the VS Code client starts the server in the home directory then.
#[tokio::test(flavor = "multi_thread")]
async fn untitled_documents_without_a_folder_run_in_the_working_directory() {
    let mut client =
        Client::start_full(datafusion(), one_publish(20), false, Value::Null, false).await;
    // Tests run in the crate's directory.
    let uri = "untitled:Untitled-1";
    client
        .open(
            uri,
            "from '../../tests/data/orders.csv' | take 1 | project orderId\n",
        )
        .await;
    assert_eq!(client.diagnostics(uri).await.codes(), Vec::<&str>::new());
    let output = client.run(uri, None).await.unwrap();
    assert!(output.starts_with("orderId\n"), "{output}");
    client.shutdown().await;
}

/// An engine whose queries compute for a long time without yielding, like a large aggregation
/// between two polls.
#[derive(Debug)]
struct Busy(DataFusionEngine);

/// Blocks its thread for two seconds on the first poll, then never ends.
struct BusyStream(bool);

impl futures::Stream for BusyStream {
    type Item = Result<arrow_array::RecordBatch, Diagnostic>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        if !self.0 {
            self.0 = true;
            std::thread::sleep(Duration::from_secs(2));
        }
        std::task::Poll::Pending
    }
}

#[async_trait::async_trait]
impl Engine for Busy {
    fn name(&self) -> &str {
        self.0.name()
    }

    fn capabilities(&self) -> &Capabilities {
        self.0.capabilities()
    }

    async fn schema_of(
        &self,
        source: &SourceRef,
        ctx: &ExecContext,
    ) -> Result<SchemaRef, SourceError> {
        self.0.schema_of(source, ctx).await
    }

    async fn execute(
        &self,
        plan: &QueryPlan,
        ctx: &ExecContext,
    ) -> Result<ResultStream, Diagnostic> {
        let real = self.0.execute(plan, ctx).await?;
        Ok(ResultStream {
            schema: real.schema,
            batches: Box::pin(BusyStream(false)),
        })
    }
}

/// The server keeps reading messages while a query computes, so a cancellation is answered at
/// once instead of after the query's next yield.
#[tokio::test(flavor = "multi_thread")]
async fn the_server_answers_while_a_query_computes() {
    let mut client = Client::start(
        Arc::new(Busy(DataFusionEngine::default())),
        one_publish(20),
        false,
    )
    .await;
    client.write("orders.csv", ORDERS);
    let uri = client.uri("busy.ds");
    client.open(&uri, "from 'orders.csv' | count\n").await;
    client.diagnostics(&uri).await;
    let id = client
        .send_request("datascript/run", json!({ "textDocument": { "uri": uri } }))
        .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let start = Instant::now();
    client.notify("$/cancelRequest", json!({ "id": id })).await;
    let response = client.response(id).await;
    assert_eq!(response["error"]["code"], -32800, "{response}");
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "the cancellation waited {:?} for the query",
        start.elapsed()
    );
    client.shutdown().await;
}
