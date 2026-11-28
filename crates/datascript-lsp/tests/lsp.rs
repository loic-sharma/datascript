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
use datascript_diagnostics::{Diagnostic, FileId, codes};
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
                        "workspace": { "didChangeWatchedFiles": { "dynamicRegistration": watch } },
                        "textDocument": {
                            "completion": { "completionItem": { "snippetSupport": true } }
                        }
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

    /// Waits `quiet`, then takes every `publishDiagnostics` for `uri` received so far. For
    /// asserting that nothing was published: call it after whatever could have published has
    /// had its chance.
    async fn publishes_within(&mut self, uri: &str, quiet: Duration) -> Vec<Published> {
        tokio::time::sleep(quiet).await;
        while let Ok(message) = self.incoming.try_recv() {
            self.queued.push_back(message);
        }
        let mut found = Vec::new();
        self.queued.retain(|m| {
            let for_uri =
                m["method"] == "textDocument/publishDiagnostics" && m["params"]["uri"] == uri;
            if for_uri {
                found.push(Published::from(m.clone()));
            }
            !for_uri
        });
        found
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

    /// Replaces the text of `uri` with `marked` minus its `‸`, and sends `method` at the `‸`.
    async fn at_cursor(&mut self, method: &str, uri: &str, version: i32, marked: &str) -> Value {
        let offset = marked.find('‸').expect("a cursor mark");
        let before = &marked[..offset];
        let line = before.matches('\n').count() as u32;
        let column = before[before.rfind('\n').map_or(0, |i| i + 1)..]
            .encode_utf16()
            .count() as u32;
        self.replace_all(uri, version, &marked.replace('‸', ""))
            .await;
        self.request(
            method,
            json!({
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": column }
            }),
        )
        .await
    }

    /// The labels of the completion items at the `‸` in `marked`.
    async fn complete(&mut self, uri: &str, version: i32, marked: &str) -> Vec<String> {
        let items = self
            .at_cursor("textDocument/completion", uri, version, marked)
            .await;
        items
            .as_array()
            .unwrap_or(&Vec::new())
            .iter()
            .map(|i| i["label"].as_str().unwrap().to_owned())
            .collect()
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
    let other = client.uri("other.ds");
    client
        .open(
            &uri,
            "from 'orders.csv' | project region;\nfrom 'later.csv' | take 1\n",
        )
        .await;
    assert_eq!(client.diagnostics(&uri).await.codes(), ["DS1001", "DS3001"]);
    client
        .open(&other, "from 'orders.csv' | project region\n")
        .await;
    assert_eq!(client.diagnostics(&other).await.codes(), ["DS1001"]);
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

    // Changed scripts are not data, whatever the case of the extension: nothing is re-checked
    // within many debounces...
    client
        .notify(
            "workspace/didChangeWatchedFiles",
            json!({ "changes": [
                { "uri": client.uri("script.ds"), "type": 2 },
                { "uri": client.uri("NOTES.DS"), "type": 1 }
            ] }),
        )
        .await;
    let quiet = Duration::from_millis(300);
    let late = client.publishes_within(&uri, quiet).await;
    assert!(late.is_empty(), "a script change re-checked: {late:?}");
    let late = client.publishes_within(&other, Duration::ZERO).await;
    assert!(late.is_empty(), "a script change re-checked: {late:?}");
    // ...and the cache is kept: an edit is still checked against the old schemas.
    client.edit(&uri, 3, (3, 0), (3, 0), "\n").await;
    let published = client.diagnostics(&uri).await;
    assert_eq!(published.version, Some(3));
    assert_eq!(published.codes(), ["DS1001", "DS3001"]);

    // Data files changed: the cache is cleared and every open document is checked again, at
    // its latest version.
    client
        .notify(
            "workspace/didChangeWatchedFiles",
            json!({ "changes": [{ "uri": data, "type": 2 }, { "uri": later, "type": 1 }] }),
        )
        .await;
    let published = client.diagnostics(&uri).await;
    assert_eq!(
        published.version,
        Some(3),
        "the latest version is re-checked"
    );
    assert_eq!(published.codes(), Vec::<&str>::new());
    let published = client.diagnostics(&other).await;
    assert_eq!(published.version, Some(1));
    assert_eq!(published.codes(), Vec::<&str>::new());

    // A deleted data file is missing again for the document that reads it.
    std::fs::remove_file(client.dir.path().join("later.csv")).unwrap();
    client
        .notify(
            "workspace/didChangeWatchedFiles",
            json!({ "changes": [{ "uri": later, "type": 3 }] }),
        )
        .await;
    let published = client.diagnostics(&uri).await;
    assert_eq!(published.version, Some(3));
    assert_eq!(published.codes(), ["DS3001"]);
    assert_eq!(client.diagnostics(&other).await.codes(), Vec::<&str>::new());
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

    // Running again fails the same way and publishes nothing new. The server publishes before
    // it answers, so anything it published is in by now; the wait covers reordering.
    let output = client.run(&uri, None).await.unwrap();
    assert!(output.contains("error[DS4003]"), "{output}");
    let again = client
        .publishes_within(&uri, Duration::from_millis(200))
        .await;
    assert!(again.is_empty(), "the second run published: {again:?}");

    // The next check after an edit clears it.
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

// --- Completion, hover and signature help (M3-02) ---

#[track_caller]
fn assert_has(labels: &[String], expected: &[&str]) {
    for e in expected {
        assert!(labels.iter().any(|l| l == e), "missing `{e}` in {labels:?}");
    }
}

#[track_caller]
fn assert_lacks(labels: &[String], unexpected: &[&str]) {
    for e in unexpected {
        assert!(
            !labels.iter().any(|l| l == e),
            "unexpected `{e}` in {labels:?}"
        );
    }
}

/// One case per completion context, typed into an open document whose sources are real files.
#[tokio::test(flavor = "multi_thread")]
async fn completion_in_each_context() {
    let mut client = Client::start(datafusion(), one_publish(10_000), false).await;
    client.write("orders.csv", ORDERS);
    std::fs::create_dir(client.dir.path().join("data")).unwrap();
    client.write("data/more.parquet", "");
    client.write("notes.txt", "");
    let uri = client.uri("query.ds");
    client.open(&uri, "").await;
    let columns = ["orderId", "customerId", "amount"];
    let lets = "let big = from 'orders.csv';\nlet p = 'orders.csv';\nlet n = 1;\n";
    let mut version = 1;
    let mut complete = async |client: &mut Client, marked: &str| {
        version += 1;
        client.complete(&uri, version, marked).await
    };

    // Statement start, and operators after `|`.
    assert_eq!(complete(&mut client, "fr‸").await, ["from", "let"]);
    let ops = complete(&mut client, "from 'orders.csv'\n| ‸").await;
    assert_has(&ops, &["where", "project-away", "summarize", "render"]);
    assert_eq!(ops.len(), datascript_syntax::OPERATOR_NAMES.len());

    // Columns in incomplete code, from the stage before the cursor.
    let l = complete(&mut client, "from 'orders.csv' | where am‸").await;
    assert_eq!(l[..3], columns);
    assert_has(&l, &["strlen", "tolower"]);
    assert_lacks(&l, &["sum"]);
    let l = complete(&mut client, "from 'orders.csv' | project amount | where ‸").await;
    assert_has(&l, &["amount"]);
    assert_lacks(&l, &["orderId"]);
    assert_has(
        &complete(&mut client, "from 'orders.csv' | summarize ‸").await,
        &["sum", "amount"],
    );
    assert_has(
        &complete(&mut client, "from 'orders.csv' | where amount > 1 ‸").await,
        &["and", "or"],
    );

    // Tables after `from`: lets, readers and files, and nothing else.
    let l = complete(&mut client, &format!("{lets}from ‸")).await;
    assert_eq!(
        l,
        [
            "big",
            "p",
            "read_csv",
            "read_json",
            "read_parquet",
            "data/",
            "orders.csv"
        ],
        "`n` is not a path and notes.txt is not a data file"
    );
    // The right side of `join` and `union`: no readers.
    let l = complete(&mut client, &format!("{lets}from 'orders.csv' | join ‸")).await;
    assert_has(&l, &["kind =", "big", "p", "orders.csv"]);
    assert_lacks(&l, &["read_csv"]);
    let l = complete(&mut client, &format!("{lets}from 'orders.csv' | union ‸")).await;
    assert_has(&l, &["big", "orders.csv"]);
    assert_has(
        &complete(&mut client, "from 'orders.csv' | join kind=‸").await,
        &["leftouter"],
    );

    // Inside a path string: the directory typed so far.
    assert_eq!(
        complete(&mut client, "from 'or‸").await,
        ["data/", "orders.csv"]
    );
    assert_eq!(
        complete(&mut client, "from 'data/‸'").await,
        ["more.parquet"]
    );
    assert_eq!(
        complete(&mut client, "from read_csv(\"‸").await,
        ["data/", "orders.csv"]
    );
    assert!(
        complete(&mut client, "from 'orders.csv' | where orderId == '‸")
            .await
            .is_empty()
    );

    // Render kinds, properties and values.
    assert_has(
        &complete(&mut client, "from 'orders.csv' | render ‸").await,
        &["barchart", "card"],
    );
    assert_eq!(
        complete(&mut client, "from 'orders.csv' | render table with (‸").await,
        ["title"]
    );
    assert_eq!(
        complete(
            &mut client,
            "from 'orders.csv' | render barchart with (xcolumn = ‸"
        )
        .await,
        columns
    );
    assert_eq!(
        complete(
            &mut client,
            "from 'orders.csv' | render barchart with (legend = ‸"
        )
        .await,
        ["hidden", "visible"]
    );

    // Reader options and their values.
    let l = complete(&mut client, "from read_csv('orders.csv', ‸").await;
    assert_has(&l, &["delimiter", "header"]);
    assert_eq!(
        complete(&mut client, "from read_csv('orders.csv', header = ‸").await,
        ["false", "true"]
    );

    // Scalar lets in expressions; nothing in comments.
    let l = complete(&mut client, &format!("{lets}from big | where amount > ‸")).await;
    assert_has(&l, &["n", "amount"]);
    assert_lacks(&l, &["big"]);
    assert!(complete(&mut client, "// from ‸").await.is_empty());
    client.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn completion_items_edit_the_word_and_use_snippets() {
    let mut client = Client::start(datafusion(), one_publish(10_000), false).await;
    client.write("orders.csv", ORDERS);
    let uri = client.uri("query.ds");
    client.open(&uri, "").await;
    let items = client
        .at_cursor(
            "textDocument/completion",
            &uri,
            2,
            "from 'orders.csv'\n| where str‸",
        )
        .await;
    let strlen = items
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["label"] == "strlen")
        .unwrap()
        .clone();
    assert_eq!(strlen["kind"], 3, "function");
    assert_eq!(strlen["insertTextFormat"], 2, "snippet");
    assert_eq!(strlen["textEdit"]["newText"], "strlen($0)");
    assert_eq!(
        strlen["textEdit"]["range"],
        json!({ "start": { "line": 1, "character": 8 }, "end": { "line": 1, "character": 11 } })
    );
    assert_eq!(strlen["detail"], "strlen(source: string) -> long");
    let amount = items
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["label"] == "amount")
        .unwrap()
        .clone();
    assert_eq!(amount["kind"], 5, "field");
    assert_eq!(amount["detail"], "real");
    client.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn hover_and_signature_help() {
    let mut client = Client::start(datafusion(), one_publish(10_000), false).await;
    client.write("orders.csv", ORDERS);
    let uri = client.uri("query.ds");
    client.open(&uri, "").await;

    let hover = client
        .at_cursor(
            "textDocument/hover",
            &uri,
            2,
            "from 'orders.csv'\n| where amo‸unt > 1",
        )
        .await;
    assert_eq!(hover["contents"]["kind"], "markdown");
    assert_eq!(
        hover["contents"]["value"],
        "```ds\namount: real\n```\ncolumn"
    );
    assert_eq!(
        hover["range"],
        json!({ "start": { "line": 1, "character": 8 }, "end": { "line": 1, "character": 14 } })
    );
    let hover = client
        .at_cursor(
            "textDocument/hover",
            &uri,
            3,
            "from 'orders.csv' | extend n = strl‸en(orderId)",
        )
        .await;
    let value = hover["contents"]["value"].as_str().unwrap();
    assert!(value.contains("strlen(source: string) -> long"), "{value}");
    let hover = client
        .at_cursor(
            "textDocument/hover",
            &uri,
            4,
            "from 'orders.csv' | where 1 > 2‸",
        )
        .await;
    assert_eq!(hover, Value::Null);

    let help = client
        .at_cursor(
            "textDocument/signatureHelp",
            &uri,
            5,
            "from 'orders.csv' | extend s = substring(orderId, ‸",
        )
        .await;
    let sig = &help["signatures"][help["activeSignature"].as_u64().unwrap() as usize];
    let label = sig["label"].as_str().unwrap();
    let active = &sig["parameters"][sig["activeParameter"].as_u64().unwrap() as usize]["label"];
    let (start, end) = (
        active[0].as_u64().unwrap() as usize,
        active[1].as_u64().unwrap() as usize,
    );
    assert_eq!(&label[start..end], "start: integer");
    let help = client
        .at_cursor(
            "textDocument/signatureHelp",
            &uri,
            6,
            "from 'orders.csv' | take 1‸",
        )
        .await;
    assert_eq!(help, Value::Null);
    client.shutdown().await;
}

/// A syntax-only server reads nothing for completion, hover or signature help: no columns, no
/// files, but keywords, operators, functions and `let` names.
#[tokio::test(flavor = "multi_thread")]
async fn syntax_only_completion_reads_no_files() {
    let mut client = Client::start_with(
        Arc::new(NoReads(DataFusionEngine::default())),
        one_publish(10_000),
        false,
        json!({ "syntaxOnly": true }),
    )
    .await;
    client.write("orders.csv", ORDERS);
    let uri = client.uri("query.ds");
    client.open(&uri, "").await;
    let l = client
        .complete(&uri, 2, "from 'orders.csv' | where am‸")
        .await;
    assert_has(&l, &["strlen", "true"]);
    assert_lacks(&l, &["amount"]);
    let l = client.complete(&uri, 3, "from 'orders.csv' | ‸").await;
    assert_has(&l, &["where", "take"]);
    let l = client
        .complete(&uri, 4, "let big = from 'orders.csv';\nlet n = 1;\nfrom ‸")
        .await;
    assert_eq!(
        l,
        ["big", "read_csv", "read_json", "read_parquet"],
        "no files"
    );
    let l = client
        .complete(&uri, 5, "let n = 1;\nfrom 'orders.csv' | take ‸")
        .await;
    assert_has(&l, &["n"]);
    assert!(client.complete(&uri, 6, "from 'or‸").await.is_empty());
    let hover = client
        .at_cursor(
            "textDocument/hover",
            &uri,
            7,
            "from 'orders.csv' | where amo‸unt > 1",
        )
        .await;
    assert_eq!(hover, Value::Null, "the column's type needs the schema");
    let hover = client
        .at_cursor(
            "textDocument/hover",
            &uri,
            8,
            "from 'orders.csv' | where strl‸en('a') > 1",
        )
        .await;
    assert!(
        hover["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("strlen(")
    );
    let help = client
        .at_cursor(
            "textDocument/signatureHelp",
            &uri,
            9,
            "from 'orders.csv' | where strlen(‸",
        )
        .await;
    assert_eq!(
        help["signatures"][0]["label"],
        "strlen(source: string) -> long"
    );
    client.shutdown().await;
}

// ── Diagnostics, document sync, run and CodeLens gaps (M3-08) ────────────────────────────────

fn format_range(range: &Value) -> String {
    format!(
        "{}:{}-{}:{}",
        range["start"]["line"],
        range["start"]["character"],
        range["end"]["line"],
        range["end"]["character"]
    )
}

fn lens_summary(lenses: &Value) -> Vec<String> {
    lenses
        .as_array()
        .unwrap()
        .iter()
        .map(|lens| {
            format!(
                "{} {} {} {}",
                format_range(&lens["range"]),
                lens["command"]["title"].as_str().unwrap(),
                lens["command"]["command"].as_str().unwrap(),
                lens["command"]["arguments"]
            )
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn diagnostics_publish_related_information_for_secondary_spans() {
    let mut client = Client::start(datafusion(), one_publish(20), false).await;
    client.write("orders.csv", ORDERS);
    let uri = client.uri("query.ds");

    // DS1003: redefined `let`, with secondary span pointing at the earlier definition on line 0.
    client
        .open(&uri, "let x = 1;\nlet x = 2;\nfrom 'orders.csv' | take x\n")
        .await;
    let published = client.diagnostics(&uri).await;
    assert_eq!(published.version, Some(1));
    assert_eq!(published.codes(), ["DS1003"]);
    assert_eq!(published.ranges(), ["1:4-1:5"]);
    let related = published.diagnostics[0]["relatedInformation"]
        .as_array()
        .expect("DS1003 relatedInformation");
    assert_eq!(related.len(), 2, "{related:?}");
    assert_eq!(related[0]["location"]["uri"], uri);
    assert_eq!(format_range(&related[0]["location"]["range"]), "1:4-1:5");
    assert_eq!(related[0]["message"], "defined again here");
    assert_eq!(related[1]["location"]["uri"], uri);
    assert_eq!(format_range(&related[1]["location"]["range"]), "0:4-0:5");
    assert_eq!(related[1]["message"], "first defined here");

    // DS2003: binary operator type mismatch, with primary and secondary operand spans.
    client
        .replace_all(
            &uri,
            2,
            "from 'orders.csv'\n| extend bad = customerId + 1\n",
        )
        .await;
    let published = client.diagnostics(&uri).await;
    assert_eq!(published.version, Some(2));
    assert_eq!(published.codes(), ["DS2003"]);
    assert_eq!(published.ranges(), ["1:15-1:25"]);
    let related = published.diagnostics[0]["relatedInformation"]
        .as_array()
        .expect("DS2003 relatedInformation");
    assert_eq!(related.len(), 2, "{related:?}");
    assert_eq!(related[0]["location"]["uri"], uri);
    assert_eq!(format_range(&related[0]["location"]["range"]), "1:15-1:25");
    assert!(
        related[0]["message"]
            .as_str()
            .unwrap()
            .starts_with("this is `string`"),
        "{related:?}"
    );
    assert_eq!(related[1]["location"]["uri"], uri);
    assert_eq!(format_range(&related[1]["location"]["range"]), "1:28-1:29");
    assert_eq!(related[1]["message"], "this is `long`");

    client.shutdown().await;
}

/// An engine whose `schema_of` panics on a chosen path and whose `execute` returns a
/// location-less `DS4002` error, exercising location-less diagnostics over JSON-RPC.
#[derive(Debug)]
struct LocationlessFailures(DataFusionEngine);

#[async_trait::async_trait]
impl Engine for LocationlessFailures {
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
        if source.path() == Some("boom.csv") {
            panic!("synthetic schema failure");
        }
        self.0.schema_of(source, ctx).await
    }

    async fn execute(&self, _: &QueryPlan, _: &ExecContext) -> Result<ResultStream, Diagnostic> {
        Err(Diagnostic::error(codes::DS4002, "untraceable engine failure").in_file(FileId(0)))
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn location_less_diagnostics_and_empty_program() {
    let mut client = Client::start(
        Arc::new(LocationlessFailures(DataFusionEngine::default())),
        one_publish(20),
        false,
    )
    .await;
    client.write("orders.csv", ORDERS);

    // An empty or let-only program is valid for `check` (LANGUAGE.md §2: no DS0004 published),
    // while `datascript/run` reports DS0004 in its output without failing.
    for (name, text) in [("empty.ds", ""), ("lets_only.ds", "let a = 1;\n")] {
        let uri = client.uri(name);
        client.open(&uri, text).await;
        let published = client.diagnostics(&uri).await;
        assert_eq!(published.version, Some(1));
        assert_eq!(published.codes(), Vec::<&str>::new());
        let ran = client
            .request("datascript/run", json!({ "textDocument": { "uri": uri } }))
            .await;
        assert_eq!(ran["failed"], false);
        assert!(
            ran["output"]
                .as_str()
                .unwrap()
                .contains("warning[DS0004]: program has no query to run"),
            "{name}: {ran}"
        );
    }

    // A location-less diagnostic from `check` (caught panic -> DS9001 with `primary: None`)
    // is published at 0:0-0:0.
    let crash = client.uri("crash.ds");
    client.open(&crash, "from 'boom.csv' | count\n").await;
    let published = client.diagnostics(&crash).await;
    assert_eq!(published.version, Some(1));
    assert_eq!(published.codes(), ["DS9001"]);
    assert_eq!(published.ranges(), ["0:0-0:0"]);
    assert_eq!(published.diagnostics[0]["relatedInformation"], Value::Null);

    // A location-less runtime diagnostic (`DS4002` with `primary: None`) from `datascript/run`
    // is likewise published at 0:0-0:0.
    let query = client.uri("query.ds");
    client.open(&query, "from 'orders.csv' | count\n").await;
    assert_eq!(client.diagnostics(&query).await.codes(), Vec::<&str>::new());
    let ran = client
        .request(
            "datascript/run",
            json!({ "textDocument": { "uri": query } }),
        )
        .await;
    assert_eq!(ran["failed"], true);
    assert!(
        ran["output"]
            .as_str()
            .unwrap()
            .contains("error[DS4002]: untraceable engine failure"),
        "{ran}"
    );
    let published = client.diagnostics(&query).await;
    assert_eq!(published.version, Some(1));
    assert_eq!(published.codes(), ["DS4002"]);
    assert_eq!(published.ranges(), ["0:0-0:0"]);
    assert_eq!(published.diagnostics[0]["relatedInformation"], Value::Null);

    client.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn did_change_applies_multiple_content_changes_in_order() {
    let mut client = Client::start(datafusion(), one_publish(20), false).await;
    client.write("orders.csv", ORDERS);
    let uri = client.uri("query.ds");

    client
        .open(&uri, "from 'orders.csv'\n| project amout\n")
        .await;
    let published = client.diagnostics(&uri).await;
    assert_eq!(published.codes(), ["DS1001"]);
    assert_eq!(published.ranges(), ["1:10-1:15"]);

    // One `didChange` with three incremental edits where each range depends on the text
    // produced by the previous edit.
    client
        .notify(
            "textDocument/didChange",
            json!({
                "textDocument": { "uri": uri, "version": 2 },
                "contentChanges": [
                    {
                        "range": {
                            "start": { "line": 1, "character": 0 },
                            "end": { "line": 1, "character": 0 }
                        },
                        "text": "| where amount > 15\n"
                    },
                    {
                        "range": {
                            "start": { "line": 2, "character": 10 },
                            "end": { "line": 2, "character": 15 }
                        },
                        "text": "amount"
                    },
                    {
                        "range": {
                            "start": { "line": 2, "character": 16 },
                            "end": { "line": 2, "character": 16 }
                        },
                        "text": " | take 1"
                    }
                ]
            }),
        )
        .await;
    let published = client.diagnostics(&uri).await;
    assert_eq!(published.version, Some(2));
    assert_eq!(published.codes(), Vec::<&str>::new());
    assert_eq!(
        client.run(&uri, None).await.unwrap(),
        "amount\nreal\n------\n  20.0\n"
    );

    // Full replacement followed by an incremental edit in one `didChange`.
    client
        .notify(
            "textDocument/didChange",
            json!({
                "textDocument": { "uri": uri, "version": 3 },
                "contentChanges": [
                    { "text": "from 'orders.csv'\n| project x\n" },
                    {
                        "range": {
                            "start": { "line": 1, "character": 10 },
                            "end": { "line": 1, "character": 11 }
                        },
                        "text": "badcol"
                    }
                ]
            }),
        )
        .await;
    let published = client.diagnostics(&uri).await;
    assert_eq!(published.version, Some(3));
    assert_eq!(published.codes(), ["DS1001"]);
    assert_eq!(published.ranges(), ["1:10-1:16"]);

    client.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn crlf_document_diagnostics_and_code_lenses() {
    let mut client = Client::start(datafusion(), one_publish(20), false).await;
    client.write("orders.csv", ORDERS);
    let uri = client.uri("crlf.ds");
    let text = "let big = from 'orders.csv' | where amount > 15;\r\n\
                from 'orders.csv'\r\n\
                | where amount > 0\r\n\
                | project amout;\r\n\
                from big\r\n\
                | count\r\n";
    client.open(&uri, text).await;

    let published = client.diagnostics(&uri).await;
    assert_eq!(published.version, Some(1));
    assert_eq!(published.codes(), ["DS1001"]);
    assert_eq!(published.ranges(), ["3:10-3:15"]);

    let lenses = client
        .request(
            "textDocument/codeLens",
            json!({ "textDocument": { "uri": uri } }),
        )
        .await;
    assert_eq!(
        lens_summary(&lenses),
        [
            format!("1:0-3:15 ▶ Run datascript.runStatement [\"{uri}\",1]"),
            format!("1:0-3:15 ▶ Run file datascript.runFile [\"{uri}\"]"),
            format!("4:0-5:7 ▶ Run datascript.runStatement [\"{uri}\",2]"),
        ]
    );

    // An incremental edit on a CRLF document updates the right line and clears the diagnostic.
    client.edit(&uri, 2, (3, 10), (3, 15), "amount").await;
    let published = client.diagnostics(&uri).await;
    assert_eq!(published.version, Some(2));
    assert_eq!(published.codes(), Vec::<&str>::new());
    let output = client.run(&uri, None).await.unwrap();
    assert!(output.contains("== query 1 (line 2) ==\n"), "{output}");
    assert!(output.contains("== query 2 (line 5) ==\n"), "{output}");

    // Join lines 2 and 3 across the `\r\n` boundary (`(2, 18)..(3, 0)`), then join lines 3 and 4
    // using a character past the end of line 3 (`(3, 99)..(4, 0)`). Clamping before `\r` rather
    // than `\n` is required so neither edit leaves a stray `\r` splitting the joined line.
    client.edit(&uri, 3, (2, 18), (3, 0), " ").await;
    let published = client.diagnostics(&uri).await;
    assert_eq!(published.version, Some(3));
    assert_eq!(published.codes(), Vec::<&str>::new());

    client.edit(&uri, 4, (3, 99), (4, 0), " ").await;
    let published = client.diagnostics(&uri).await;
    assert_eq!(published.version, Some(4));
    assert_eq!(published.codes(), Vec::<&str>::new());

    let lenses = client
        .request(
            "textDocument/codeLens",
            json!({ "textDocument": { "uri": uri } }),
        )
        .await;
    assert_eq!(
        lens_summary(&lenses),
        [
            format!("1:0-2:35 ▶ Run datascript.runStatement [\"{uri}\",1]"),
            format!("1:0-2:35 ▶ Run file datascript.runFile [\"{uri}\"]"),
            format!("3:0-3:16 ▶ Run datascript.runStatement [\"{uri}\",2]"),
        ]
    );
    let output = client.run(&uri, None).await.unwrap();
    assert!(output.contains("== query 1 (line 2) ==\n"), "{output}");
    assert!(output.contains("== query 2 (line 4) ==\n"), "{output}");

    // Insert a multi-line query at the start of line 3 (after three CRLF lines) whose diagnostic
    // span itself crosses `\r\n` line breaks.
    client
        .edit(
            &uri,
            5,
            (3, 0),
            (3, 0),
            "from 'orders.csv'\r\n| where (\r\n    amount + 1\r\n);\r\n",
        )
        .await;
    let published = client.diagnostics(&uri).await;
    assert_eq!(published.version, Some(5));
    assert_eq!(published.codes(), ["DS2001"]);
    assert_eq!(published.ranges(), ["4:8-6:1"]);
    let lenses = client
        .request(
            "textDocument/codeLens",
            json!({ "textDocument": { "uri": uri } }),
        )
        .await;
    assert_eq!(
        lens_summary(&lenses),
        [
            format!("1:0-2:35 ▶ Run datascript.runStatement [\"{uri}\",1]"),
            format!("1:0-2:35 ▶ Run file datascript.runFile [\"{uri}\"]"),
            format!("3:0-6:1 ▶ Run datascript.runStatement [\"{uri}\",2]"),
            format!("7:0-7:16 ▶ Run datascript.runStatement [\"{uri}\",3]"),
        ]
    );

    client.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn runtime_error_keeps_check_warnings_in_published_diagnostics() {
    let mut client = Client::start(datafusion(), one_publish(20), false).await;
    client.write("big.csv", "x\n1\n10000\n");
    let uri = client.uri("warn_and_overflow.ds");
    // `sort by x` without `asc`/`desc` triggers DS2103 (warning); the multiplication overflows
    // at run time (DS4003).
    let text = "from 'big.csv' | sort by x | extend y = x * 1000000000000000\n";
    client.open(&uri, text).await;
    let published = client.diagnostics(&uri).await;
    assert_eq!(published.version, Some(1));
    assert_eq!(published.codes(), ["DS2103"]);
    assert_eq!(published.diagnostics[0]["severity"], 2);

    let ran = client
        .request("datascript/run", json!({ "textDocument": { "uri": uri } }))
        .await;
    assert_eq!(ran["failed"], true);
    let output = ran["output"].as_str().unwrap();
    assert!(output.contains("warning[DS2103]"), "{output}");
    assert!(output.contains("error[DS4003]"), "{output}");

    // Both the check warning and the runtime error are published together.
    let published = client.diagnostics(&uri).await;
    assert_eq!(published.version, Some(1));
    assert_eq!(published.codes(), ["DS2103", "DS4003"]);
    assert_eq!(published.diagnostics[0]["severity"], 2);
    assert_eq!(published.diagnostics[1]["severity"], 1);

    // Editing to fix the overflow clears only the runtime error; the warning stays.
    client.edit(&uri, 2, (0, 44), (0, 60), "2").await;
    let published = client.diagnostics(&uri).await;
    assert_eq!(published.version, Some(2));
    assert_eq!(published.codes(), ["DS2103"]);

    client.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn run_rejects_statement_zero_and_allows_unlimited_max_rows() {
    let mut client = Client::start(datafusion(), one_publish(20), false).await;
    let mut csv = String::from("n\n");
    for i in 1..=105 {
        csv.push_str(&format!("{i}\n"));
    }
    client.write("many.csv", &csv);
    let uri = client.uri("many.ds");
    client.open(&uri, "from 'many.csv' | sort by n asc\n").await;
    assert_eq!(client.diagnostics(&uri).await.codes(), Vec::<&str>::new());

    // `statement: 0` is 1-based out of range and refused with REQUEST_FAILED (-32803).
    let response = client
        .request_response(
            "datascript/run",
            json!({ "textDocument": { "uri": uri }, "statement": 0 }),
        )
        .await;
    assert_eq!(response["error"]["code"], -32803, "{response}");
    assert_eq!(
        response["error"]["message"],
        "statement 0 is out of range: the document has 1 query statement"
    );

    // Default `maxRows` caps output at 100 rows.
    let default_run = client
        .request("datascript/run", json!({ "textDocument": { "uri": uri } }))
        .await;
    assert_eq!(default_run["failed"], false);
    let default_out = default_run["output"].as_str().unwrap();
    assert!(
        default_out.contains("… 5 more rows (show them with --max-rows 105)"),
        "{default_out}"
    );
    assert!(!default_out.contains("105\n"), "{default_out}");

    // `maxRows: 0` lifts the limit and returns every row.
    let unlimited = client
        .request(
            "datascript/run",
            json!({ "textDocument": { "uri": uri }, "maxRows": 0 }),
        )
        .await;
    assert_eq!(unlimited["failed"], false);
    let unlimited_out = unlimited["output"].as_str().unwrap();
    assert!(!unlimited_out.contains("more row"), "{unlimited_out}");
    assert!(unlimited_out.ends_with("105\n"), "{unlimited_out}");
    assert_eq!(
        unlimited_out.lines().count(),
        3 + 105,
        "header + type + separator + 105 rows"
    );

    client.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn run_with_compile_error_fails_without_adding_runtime_diagnostics() {
    let mut client = Client::start(datafusion(), one_publish(20), false).await;
    client.write("orders.csv", ORDERS);
    let uri = client.uri("bad.ds");
    client
        .open(&uri, "from 'orders.csv' | project amout\n")
        .await;
    let published = client.diagnostics(&uri).await;
    assert_eq!(published.version, Some(1));
    assert_eq!(published.codes(), ["DS1001"]);

    let ran = client
        .request("datascript/run", json!({ "textDocument": { "uri": uri } }))
        .await;
    assert_eq!(ran["failed"], true);
    assert!(
        ran["output"].as_str().unwrap().contains("error[DS1001]"),
        "{ran}"
    );

    // No extra `publishDiagnostics` notification was emitted for version 1: the very next
    // notification for `uri` is version 2's check after fixing the typo.
    client.edit(&uri, 2, (0, 28), (0, 33), "amount").await;
    let published = client.diagnostics(&uri).await;
    assert_eq!(published.version, Some(2));
    assert_eq!(published.codes(), Vec::<&str>::new());

    client.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn code_lens_covers_multiline_query_statements() {
    let mut client = Client::start(datafusion(), one_publish(20), false).await;
    client.write("orders.csv", ORDERS);
    let uri = client.uri("multiline.ds");
    let text = "// header comment\n\
                from 'orders.csv'\n\
                | where amount > 10\n\
                | summarize total = sum(amount) by customerId\n\
                | sort by total desc;\n\
                \n\
                from 'orders.csv'\n\
                | take 5\n";
    client.open(&uri, text).await;

    let lenses = client
        .request(
            "textDocument/codeLens",
            json!({ "textDocument": { "uri": uri } }),
        )
        .await;
    assert_eq!(
        lens_summary(&lenses),
        [
            format!("1:0-4:20 ▶ Run datascript.runStatement [\"{uri}\",1]"),
            format!("1:0-4:20 ▶ Run file datascript.runFile [\"{uri}\"]"),
            format!("6:0-7:8 ▶ Run datascript.runStatement [\"{uri}\",2]"),
        ]
    );

    client.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn code_lens_on_syntax_errors_and_closed_or_unknown_uri() {
    let mut client = Client::start(datafusion(), one_publish(20), false).await;
    client.write("orders.csv", ORDERS);

    // Unknown URI (never opened): null.
    let unknown = client.uri("never_opened.ds");
    let lenses = client
        .request(
            "textDocument/codeLens",
            json!({ "textDocument": { "uri": unknown } }),
        )
        .await;
    assert_eq!(lenses, Value::Null);

    // Recoverable syntax errors inside a query still produce lenses over the parsed queries.
    let uri = client.uri("broken.ds");
    client
        .open(
            &uri,
            "from 'orders.csv' | where amount = 10;\nfrom 'orders.csv' | count\n",
        )
        .await;
    assert_eq!(client.diagnostics(&uri).await.codes(), ["DS0110"]);
    let lenses = client
        .request(
            "textDocument/codeLens",
            json!({ "textDocument": { "uri": uri } }),
        )
        .await;
    assert_eq!(
        lens_summary(&lenses),
        [
            format!("0:0-0:37 ▶ Run datascript.runStatement [\"{uri}\",1]"),
            format!("0:0-0:37 ▶ Run file datascript.runFile [\"{uri}\"]"),
            format!("1:0-1:25 ▶ Run datascript.runStatement [\"{uri}\",2]"),
        ]
    );

    // Unparseable tokens that form no query statement yield an empty list, not an error.
    client.replace_all(&uri, 2, "=== ;;;\n").await;
    let published = client.diagnostics(&uri).await;
    assert_eq!(published.version, Some(2));
    assert!(!published.codes().is_empty());
    let lenses = client
        .request(
            "textDocument/codeLens",
            json!({ "textDocument": { "uri": uri } }),
        )
        .await;
    assert_eq!(lenses, json!([]));

    // After closing the document, `codeLens` returns null.
    client
        .notify(
            "textDocument/didClose",
            json!({ "textDocument": { "uri": uri } }),
        )
        .await;
    assert_eq!(client.diagnostics(&uri).await.codes(), Vec::<&str>::new());
    let lenses = client
        .request(
            "textDocument/codeLens",
            json!({ "textDocument": { "uri": uri } }),
        )
        .await;
    assert_eq!(lenses, Value::Null);

    client.shutdown().await;
}

// ── Quick fixes, navigation, rename, symbols and semantic tokens (M3-03) ────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn code_actions_offer_quick_fixes_from_diagnostics() {
    let mut client = Client::start(datafusion(), one_publish(20), false).await;
    client.write("orders.csv", ORDERS);
    let uri = client.uri("fixes.ds");
    // Line 0: syntax error DS0110 (`=`), lint warning DS2103 (`sort by`), semantic error DS1001 (`amout`).
    // Line 1: unknown format DS3002 (`'data.txt'`, `MaybeIncorrect` fix).
    let text = "from 'orders.csv' | where amount = 10 | sort by amount | project amout;\n\
                from 'data.txt' | count\n";
    client.open(&uri, text).await;
    let published = client.diagnostics(&uri).await;
    assert!(
        published.codes().contains(&"DS0110"),
        "{:?}",
        published.codes()
    );
    assert!(
        published.codes().contains(&"DS2103"),
        "{:?}",
        published.codes()
    );
    assert!(
        published.codes().contains(&"DS1001"),
        "{:?}",
        published.codes()
    );
    assert!(
        published.codes().contains(&"DS3002"),
        "{:?}",
        published.codes()
    );

    // Quick fix for `=` (DS0110) is preferred and replaces `=` with `==`.
    let eq_col = text.find("= 10").unwrap() as u32;
    let actions = client
        .request(
            "textDocument/codeAction",
            json!({
                "textDocument": { "uri": uri },
                "range": {
                    "start": { "line": 0, "character": eq_col },
                    "end": { "line": 0, "character": eq_col + 1 }
                },
                "context": { "diagnostics": [] }
            }),
        )
        .await;
    let arr = actions.as_array().unwrap();
    assert_eq!(arr.len(), 1, "{actions}");
    assert_eq!(arr[0]["kind"], "quickfix");
    assert_eq!(arr[0]["isPreferred"], true);
    assert_eq!(arr[0]["diagnostics"][0]["code"], "DS0110");
    assert_eq!(arr[0]["edit"]["changes"][&uri][0]["newText"], "==");

    // Quick fix for `sort by amount` (DS2103) inserts ` asc`.
    let sort_col = text.find("by amount").unwrap() as u32 + 3;
    let actions = client
        .request(
            "textDocument/codeAction",
            json!({
                "textDocument": { "uri": uri },
                "range": {
                    "start": { "line": 0, "character": sort_col },
                    "end": { "line": 0, "character": sort_col }
                },
                "context": { "diagnostics": [] }
            }),
        )
        .await;
    let arr = actions.as_array().unwrap();
    assert_eq!(arr.len(), 1, "{actions}");
    assert_eq!(arr[0]["isPreferred"], true);
    assert_eq!(arr[0]["diagnostics"][0]["code"], "DS2103");
    assert_eq!(arr[0]["edit"]["changes"][&uri][0]["newText"], " asc");

    // Quick fix for typo `amout` (DS1001) replaces with `amount`.
    let typo_col = text.find("amout").unwrap() as u32;
    let actions = client
        .request(
            "textDocument/codeAction",
            json!({
                "textDocument": { "uri": uri },
                "range": {
                    "start": { "line": 0, "character": typo_col },
                    "end": { "line": 0, "character": typo_col + 5 }
                },
                "context": { "diagnostics": [] }
            }),
        )
        .await;
    let arr = actions.as_array().unwrap();
    assert_eq!(arr.len(), 1, "{actions}");
    assert_eq!(arr[0]["isPreferred"], true);
    assert_eq!(arr[0]["diagnostics"][0]["code"], "DS1001");
    assert_eq!(arr[0]["edit"]["changes"][&uri][0]["newText"], "amount");

    // `MaybeIncorrect` suggestion (DS3002) is offered with `isPreferred: false`.
    let actions = client
        .request(
            "textDocument/codeAction",
            json!({
                "textDocument": { "uri": uri },
                "range": {
                    "start": { "line": 1, "character": 5 },
                    "end": { "line": 1, "character": 15 }
                },
                "context": { "diagnostics": [] }
            }),
        )
        .await;
    let arr = actions.as_array().unwrap();
    assert_eq!(arr.len(), 1, "{actions}");
    assert_eq!(arr[0]["isPreferred"], false);
    assert_eq!(arr[0]["diagnostics"][0]["code"], "DS3002");
    assert_eq!(
        arr[0]["edit"]["changes"][&uri][0]["newText"],
        "read_csv('data.txt')"
    );

    // Asking for non-quickfix or narrower quickfix sub-kinds returns empty.
    for only in [json!(["refactor"]), json!(["quickfix.other"])] {
        let filtered = client
            .request(
                "textDocument/codeAction",
                json!({
                    "textDocument": { "uri": uri },
                    "range": {
                        "start": { "line": 0, "character": eq_col },
                        "end": { "line": 0, "character": eq_col + 1 }
                    },
                    "context": { "diagnostics": [], "only": only }
                }),
            )
            .await;
        assert_eq!(filtered, json!([]));
    }

    // Multi-edit quick fix: DS2103 on `sort by amount, customerId` produces 2 edits in one action.
    let multi_uri = client.uri("multi_edit.ds");
    client
        .open(
            &multi_uri,
            "from 'orders.csv' | sort by amount, customerId\n",
        )
        .await;
    assert_eq!(client.diagnostics(&multi_uri).await.codes(), ["DS2103"]);
    let actions = client
        .request(
            "textDocument/codeAction",
            json!({
                "textDocument": { "uri": multi_uri },
                "range": {
                    "start": { "line": 0, "character": 28 },
                    "end": { "line": 0, "character": 34 }
                },
                "context": { "diagnostics": [] }
            }),
        )
        .await;
    let arr = actions.as_array().unwrap();
    assert_eq!(arr.len(), 1, "{actions}");
    assert_eq!(arr[0]["isPreferred"], true);
    let edits = arr[0]["edit"]["changes"][&multi_uri].as_array().unwrap();
    assert_eq!(edits.len(), 2, "{actions}");
    assert_eq!(edits[0]["newText"], " asc");
    assert_eq!(edits[1]["newText"], " asc");

    client.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn goto_definition_references_and_rename_for_lets() {
    let mut client = Client::start(datafusion(), one_publish(20), false).await;
    client.write("orders.csv", ORDERS);
    let uri = client.uri("nav.ds");
    let text = "let cutoff = 15;\n\
                let big = from 'orders.csv' | where amount > cutoff;\n\
                from big | where amount > cutoff | count\n";
    client.open(&uri, text).await;
    assert_eq!(client.diagnostics(&uri).await.codes(), Vec::<&str>::new());

    // Go to definition from `big` on line 2 -> line 1 col 4..7.
    let def = client
        .request(
            "textDocument/definition",
            json!({
                "textDocument": { "uri": uri },
                "position": { "line": 2, "character": 6 }
            }),
        )
        .await;
    assert_eq!(
        def,
        json!({
            "uri": uri,
            "range": {
                "start": { "line": 1, "character": 4 },
                "end": { "line": 1, "character": 7 }
            }
        })
    );

    // Go to definition on a column (`amount`) returns null.
    let none = client
        .request(
            "textDocument/definition",
            json!({
                "textDocument": { "uri": uri },
                "position": { "line": 1, "character": 38 }
            }),
        )
        .await;
    assert_eq!(none, Value::Null);

    // Find references for `cutoff`, with and without declaration.
    let refs_with_decl = client
        .request(
            "textDocument/references",
            json!({
                "textDocument": { "uri": uri },
                "position": { "line": 2, "character": 28 },
                "context": { "includeDeclaration": true }
            }),
        )
        .await;
    let ref_lines: Vec<u64> = refs_with_decl
        .as_array()
        .unwrap()
        .iter()
        .map(|loc| loc["range"]["start"]["line"].as_u64().unwrap())
        .collect();
    assert_eq!(ref_lines, [0, 1, 2]);

    let refs_no_decl = client
        .request(
            "textDocument/references",
            json!({
                "textDocument": { "uri": uri },
                "position": { "line": 0, "character": 5 },
                "context": { "includeDeclaration": false }
            }),
        )
        .await;
    let ref_lines: Vec<u64> = refs_no_decl
        .as_array()
        .unwrap()
        .iter()
        .map(|loc| loc["range"]["start"]["line"].as_u64().unwrap())
        .collect();
    assert_eq!(ref_lines, [1, 2]);

    // Prepare rename on `cutoff` returns range + placeholder; on a column returns null.
    let prep = client
        .request(
            "textDocument/prepareRename",
            json!({
                "textDocument": { "uri": uri },
                "position": { "line": 1, "character": 46 }
            }),
        )
        .await;
    assert_eq!(
        prep,
        json!({
            "range": {
                "start": { "line": 1, "character": 45 },
                "end": { "line": 1, "character": 51 }
            },
            "placeholder": "cutoff"
        })
    );
    let prep_col = client
        .request(
            "textDocument/prepareRename",
            json!({
                "textDocument": { "uri": uri },
                "position": { "line": 1, "character": 38 }
            }),
        )
        .await;
    assert_eq!(prep_col, Value::Null);

    // Rename `cutoff` -> `min amount` (auto-quoted) returns 3 edits.
    let edit = client
        .request(
            "textDocument/rename",
            json!({
                "textDocument": { "uri": uri },
                "position": { "line": 0, "character": 5 },
                "newName": "min amount"
            }),
        )
        .await;
    let edits = edit["changes"][&uri].as_array().unwrap();
    assert_eq!(edits.len(), 3, "{edit}");
    for e in edits {
        assert_eq!(e["newText"], "['min amount']");
    }

    // Renaming `cutoff` to `big` (already defined) returns an invalid_params error.
    let err = client
        .request_response(
            "textDocument/rename",
            json!({
                "textDocument": { "uri": uri },
                "position": { "line": 0, "character": 5 },
                "newName": "big"
            }),
        )
        .await;
    assert!(
        err["error"]["message"]
            .as_str()
            .unwrap()
            .contains("already defined"),
        "{err}"
    );

    // Renaming `cutoff` to `amount` (a column in scope at line 2) returns an invalid_params error.
    let col_err = client
        .request_response(
            "textDocument/rename",
            json!({
                "textDocument": { "uri": uri },
                "position": { "line": 0, "character": 5 },
                "newName": "amount"
            }),
        )
        .await;
    assert!(
        col_err["error"]["message"]
            .as_str()
            .unwrap()
            .contains("is a column at line 2"),
        "{col_err}"
    );

    // Column shadowing in full mode: `let amount = 5;` is NOT referenced by `where amount > 0`.
    let shadow_uri = client.uri("shadow.ds");
    client
        .open(
            &shadow_uri,
            "let amount = 5;\nfrom 'orders.csv' | where amount > 0 | take amount\n",
        )
        .await;
    assert_eq!(
        client.diagnostics(&shadow_uri).await.codes(),
        Vec::<&str>::new()
    );
    let shadow_refs = client
        .request(
            "textDocument/references",
            json!({
                "textDocument": { "uri": shadow_uri },
                "position": { "line": 0, "character": 5 },
                "context": { "includeDeclaration": false }
            }),
        )
        .await;
    let arr = shadow_refs.as_array().unwrap();
    assert_eq!(
        arr.len(),
        1,
        "only `take amount`, not `where amount`: {shadow_refs}"
    );
    assert_eq!(arr[0]["range"]["start"]["character"], 44);

    client.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn document_symbols_lists_lets_and_queries() {
    let mut client = Client::start(datafusion(), one_publish(20), false).await;
    client.write("orders.csv", ORDERS);
    let uri = client.uri("symbols.ds");
    let text = "let cutoff = 15;\n\
                let big = from 'orders.csv' | where amount > cutoff;\n\
                from big | count;\n\
                from 'orders.csv' | take 1\n";
    client.open(&uri, text).await;
    assert_eq!(client.diagnostics(&uri).await.codes(), Vec::<&str>::new());

    let symbols = client
        .request(
            "textDocument/documentSymbol",
            json!({ "textDocument": { "uri": uri } }),
        )
        .await;
    let arr = symbols.as_array().unwrap();
    assert_eq!(arr.len(), 4, "{symbols}");
    assert_eq!(arr[0]["name"], "cutoff");
    assert_eq!(arr[0]["kind"], 13); // SymbolKind::VARIABLE
    assert_eq!(arr[0]["detail"], "long");
    assert_eq!(arr[1]["name"], "big");
    assert_eq!(arr[1]["kind"], 23); // SymbolKind::STRUCT
    assert_eq!(arr[1]["detail"], "table (3 columns)");
    assert_eq!(arr[2]["name"], "query 1");
    assert_eq!(arr[2]["kind"], 19); // SymbolKind::OBJECT
    assert_eq!(arr[2]["detail"], "from big");
    assert_eq!(arr[3]["name"], "query 2");
    assert_eq!(arr[3]["detail"], "from 'orders.csv'");

    client.shutdown().await;
}

/// Decodes LSP delta-encoded semantic tokens into `(line, start, len, type_index, modifiers)`.
fn decode_semantic_tokens(result: &Value) -> Vec<(u32, u32, u32, u32, u32)> {
    let data = result["data"].as_array().unwrap();
    let (chunks, remainder) = data.as_chunks::<5>();
    assert!(remainder.is_empty());
    let mut out = Vec::with_capacity(chunks.len());
    let mut line = 0u32;
    let mut start = 0u32;
    for chunk in chunks {
        let dl = chunk[0].as_u64().unwrap() as u32;
        let ds = chunk[1].as_u64().unwrap() as u32;
        let len = chunk[2].as_u64().unwrap() as u32;
        let ty = chunk[3].as_u64().unwrap() as u32;
        let mods = chunk[4].as_u64().unwrap() as u32;
        line += dl;
        start = if dl == 0 { start + ds } else { ds };
        out.push((line, start, len, ty, mods));
    }
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn semantic_tokens_full_encodes_tokens_and_fixes_contextual_keywords() {
    let mut client = Client::start(datafusion(), one_publish(20), false).await;
    let uri = client.uri("tokens.ds");
    // Includes multi-line block comment, emoji UTF-16 offset, contextual keywords as columns,
    // and a hyphenated operator (`project-away`).
    let text = "/* line 1\nline 2 */\n\
                from '😀.csv' | where by > 1 and has == on | project-away by\n";
    client.open(&uri, text).await;

    let result = client
        .request(
            "textDocument/semanticTokens/full",
            json!({ "textDocument": { "uri": uri } }),
        )
        .await;
    let tokens = decode_semantic_tokens(&result);

    // Multi-line block comment is split across lines 0 and 1.
    assert_eq!(tokens[0], (0, 0, 9, 0, 0)); // `/* line 1` -> comment
    assert_eq!(tokens[1], (1, 0, 9, 0, 0)); // `line 2 */` -> comment

    // Line 2: `from` (keyword=1), `'😀.csv'` (string=9, UTF-16 len 8), `|` (operator=2),
    // `where` (keyword=1, defaultLibrary=4), `by` (variable=6), `>` (operator=2), `1` (number=8),
    // `and` (operator=2), `has` (variable=6), `==` (operator=2), `on` (variable=6),
    // `|` (operator=2), `project-away` (keyword=1, defaultLibrary=4), `by` (variable=6).
    let line2: Vec<_> = tokens.into_iter().filter(|t| t.0 == 2).collect();
    assert_eq!(
        line2,
        [
            (2, 0, 4, 1, 0),   // from
            (2, 5, 8, 9, 0),   // '😀.csv' (2 UTF-16 units for 😀)
            (2, 14, 1, 2, 0),  // |
            (2, 16, 5, 1, 4),  // where
            (2, 22, 2, 6, 0),  // by (column!)
            (2, 25, 1, 2, 0),  // >
            (2, 27, 1, 8, 0),  // 1
            (2, 29, 3, 2, 0),  // and
            (2, 33, 3, 6, 0),  // has (column!)
            (2, 37, 2, 2, 0),  // ==
            (2, 40, 2, 6, 0),  // on (column!)
            (2, 43, 1, 2, 0),  // |
            (2, 45, 12, 1, 4), // project-away
            (2, 58, 2, 6, 0),  // by (column!)
        ]
    );

    client.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn extend_and_join_side_completion_in_lsp() {
    let mut client = Client::start(datafusion(), one_publish(20), false).await;
    client.write("orders.csv", ORDERS);
    client.write("customers.csv", "id,name\nc-1,Alice\n");
    let uri = client.uri("comp.ds");
    client.open(&uri, "").await;

    // `extend` item sees earlier items.
    let items = client
        .complete(
            &uri,
            2,
            "from 'orders.csv' | extend total = amount * 2, big = ‸",
        )
        .await;
    assert!(items.contains(&"total".to_owned()), "{items:?}");
    assert!(items.contains(&"amount".to_owned()), "{items:?}");

    // `$left.` and `$right.` in `join` conditions offer only that side's columns.
    let right = client
        .complete(
            &uri,
            3,
            "from 'orders.csv' | join 'customers.csv' on $left.customerId == $right.‸",
        )
        .await;
    assert_eq!(right, ["id", "name"]);

    let left = client
        .complete(
            &uri,
            4,
            "from 'orders.csv' | join 'customers.csv' on $left.‸",
        )
        .await;
    assert_eq!(left, ["orderId", "customerId", "amount"]);

    client.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn syntax_only_mode_supports_navigation_rename_symbols_tokens_and_syntax_fixes() {
    // `NoReads` panics on any schema lookup: every feature below must work from the parse alone.
    let mut client = Client::start_with(
        Arc::new(NoReads(DataFusionEngine::default())),
        one_publish(20),
        false,
        json!({ "syntaxOnly": true }),
    )
    .await;
    let uri = client.uri("untrusted.ds");
    let text = "let limit = 15;\n\
                let amount = 5;\n\
                from 'orders.csv' | where by = amount | take limit | project amout\n";
    client.open(&uri, text).await;
    assert_eq!(client.diagnostics(&uri).await.codes(), ["DS0110"]);

    // Code actions return syntax quick fix (DS0110), and NOT semantic fixes.
    let eq_col = text.lines().nth(2).unwrap().find("= amount").unwrap() as u32;
    let actions = client
        .request(
            "textDocument/codeAction",
            json!({
                "textDocument": { "uri": uri },
                "range": {
                    "start": { "line": 2, "character": eq_col },
                    "end": { "line": 2, "character": eq_col + 1 }
                },
                "context": { "diagnostics": [] }
            }),
        )
        .await;
    let arr = actions.as_array().unwrap();
    assert_eq!(arr.len(), 1, "{actions}");
    assert_eq!(arr[0]["diagnostics"][0]["code"], "DS0110");

    // Go to definition, references, prepareRename and rename work without reading files for
    // no-column positions (`take limit`).
    let limit_col = text.lines().nth(2).unwrap().find("limit").unwrap() as u32;
    let def = client
        .request(
            "textDocument/definition",
            json!({
                "textDocument": { "uri": uri },
                "position": { "line": 2, "character": limit_col + 1 }
            }),
        )
        .await;
    assert_eq!(def["range"]["start"]["line"], 0);
    assert_eq!(def["range"]["start"]["character"], 4);

    let refs = client
        .request(
            "textDocument/references",
            json!({
                "textDocument": { "uri": uri },
                "position": { "line": 0, "character": 5 },
                "context": { "includeDeclaration": true }
            }),
        )
        .await;
    assert_eq!(refs.as_array().unwrap().len(), 2);

    let prep = client
        .request(
            "textDocument/prepareRename",
            json!({
                "textDocument": { "uri": uri },
                "position": { "line": 2, "character": limit_col + 1 }
            }),
        )
        .await;
    assert_eq!(prep["placeholder"], "limit");

    let renamed = client
        .request(
            "textDocument/rename",
            json!({
                "textDocument": { "uri": uri },
                "position": { "line": 2, "character": limit_col + 1 },
                "newName": "max_rows"
            }),
        )
        .await;
    assert_eq!(renamed["changes"][&uri].as_array().unwrap().len(), 2);

    // Row-expression `amount` in syntax-only mode has unknown schema: excluded from definition/
    // references, and prepareRename/rename refuse rather than risk rewriting a column.
    let amount_col = text.lines().nth(2).unwrap().find("amount").unwrap() as u32;
    let row_def = client
        .request(
            "textDocument/definition",
            json!({
                "textDocument": { "uri": uri },
                "position": { "line": 2, "character": amount_col + 1 }
            }),
        )
        .await;
    assert_eq!(row_def, Value::Null);
    let prep_err = client
        .request_response(
            "textDocument/prepareRename",
            json!({
                "textDocument": { "uri": uri },
                "position": { "line": 1, "character": 5 }
            }),
        )
        .await;
    assert!(
        prep_err["error"]["message"]
            .as_str()
            .unwrap()
            .contains("input schema"),
        "{prep_err}"
    );
    let rename_err = client
        .request_response(
            "textDocument/rename",
            json!({
                "textDocument": { "uri": uri },
                "position": { "line": 1, "character": 5 },
                "newName": "threshold"
            }),
        )
        .await;
    assert!(
        rename_err["error"]["message"]
            .as_str()
            .unwrap()
            .contains("input schema"),
        "{rename_err}"
    );

    // Document symbols work from the parse alone.
    let symbols = client
        .request(
            "textDocument/documentSymbol",
            json!({ "textDocument": { "uri": uri } }),
        )
        .await;
    let arr = symbols.as_array().unwrap();
    assert_eq!(arr.len(), 3);
    assert_eq!(arr[0]["name"], "limit");
    assert_eq!(arr[0]["detail"], "scalar");
    assert_eq!(arr[1]["name"], "amount");
    assert_eq!(arr[1]["detail"], "scalar");
    assert_eq!(arr[2]["name"], "query 1");

    // Semantic tokens work from the parse alone and classify `by` in `where by = amount` as variable,
    // without falsely marking ambiguous `amount` as `readonly`.
    let tokens = client
        .request(
            "textDocument/semanticTokens/full",
            json!({ "textDocument": { "uri": uri } }),
        )
        .await;
    let decoded = decode_semantic_tokens(&tokens);
    let by_col = text.lines().nth(2).unwrap().find("by").unwrap() as u32;
    assert!(
        decoded.contains(&(2, by_col, 2, 6, 0)),
        "`by` should be variable (6): {decoded:?}"
    );
    assert!(
        decoded.contains(&(2, amount_col, 6, 6, 0)),
        "ambiguous `amount` should not have readonly modifier: {decoded:?}"
    );
    assert!(
        decoded.contains(&(2, limit_col, 5, 6, 2)),
        "unambiguous `take limit` should have readonly modifier (2): {decoded:?}"
    );

    client.shutdown().await;
}

// ── In-flight checks, UTF-16 and panics in Run (M3-10) ───────────────────────────────────────

/// The notifications between a test and a [`Gated`] engine.
#[derive(Debug, Clone, Default)]
struct Gate {
    /// The first schema read has started and is waiting.
    entered: Arc<tokio::sync::Notify>,
    /// Lets the first schema read go on.
    release: Arc<tokio::sync::Notify>,
    /// The first schema read has returned.
    left: Arc<tokio::sync::Notify>,
}

/// An engine whose first schema read waits until the test releases it, so a check stays in
/// flight for exactly as long as the test needs. Later reads go straight through.
#[derive(Debug)]
struct Gated {
    inner: DataFusionEngine,
    gate: Gate,
    first: std::sync::atomic::AtomicBool,
}

impl Gated {
    fn engine(gate: &Gate) -> Arc<dyn Engine> {
        Arc::new(Gated {
            inner: DataFusionEngine::default(),
            gate: gate.clone(),
            first: std::sync::atomic::AtomicBool::new(true),
        })
    }
}

#[async_trait::async_trait]
impl Engine for Gated {
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
        if !self.first.swap(false, std::sync::atomic::Ordering::SeqCst) {
            return self.inner.schema_of(source, ctx).await;
        }
        self.gate.entered.notify_one();
        self.gate.release.notified().await;
        let found = self.inner.schema_of(source, ctx).await;
        self.gate.left.notify_one();
        found
    }

    async fn execute(
        &self,
        plan: &QueryPlan,
        ctx: &ExecContext,
    ) -> Result<ResultStream, Diagnostic> {
        self.inner.execute(plan, ctx).await
    }
}

/// Timing for the in-flight tests: a short grace, so the held check's syntax-only publish
/// comes due while it waits on the gate.
fn short_grace() -> Config {
    Config {
        debounce: Duration::from_millis(10),
        syntax_grace: Duration::from_millis(200),
    }
}

/// Waits for `notify`, failing the test after [`TIMEOUT`].
async fn wait_for(notify: &tokio::sync::Notify, what: &str) {
    tokio::time::timeout(TIMEOUT, notify.notified())
        .await
        .unwrap_or_else(|_| panic!("timed out waiting until {what}"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_change_during_schema_resolution_supersedes_the_check() {
    let gate = Gate::default();
    let mut client = Client::start(Gated::engine(&gate), short_grace(), false).await;
    client.write("orders.csv", ORDERS);
    let uri = client.uri("query.ds");
    // Version 1 has a syntax error, so its syntax-only publish would not be empty.
    client
        .open(&uri, "from 'orders.csv' | where amount = 1\n")
        .await;
    wait_for(&gate.entered, "the check of version 1 reads the schema").await;

    // Version 2 arrives while version 1's check waits on the schema; its own check reads
    // through.
    client
        .replace_all(&uri, 2, "from 'orders.csv' | project amout\n")
        .await;
    let mut before = Vec::new();
    loop {
        let published = client.diagnostics(&uri).await;
        if published.version == Some(2) && published.codes() == ["DS1001"] {
            break;
        }
        before.push(published);
    }
    // Version 1's syntax-only publish may come first if the change took longer than the grace
    // to arrive; that is in order. Anything else would be a stale publish.
    assert!(
        before
            .iter()
            .all(|p| p.version == Some(2) || p.codes() == ["DS0110"]),
        "{before:?}"
    );

    // Let version 1's grace run out while its check is held, then let the check finish:
    // neither its syntax-only publish nor its final one may overwrite version 2.
    tokio::time::sleep(Duration::from_millis(400)).await;
    gate.release.notify_one();
    wait_for(&gate.left, "the check of version 1 has its schema").await;
    let late = client
        .publishes_within(&uri, Duration::from_millis(300))
        .await;
    assert!(late.is_empty(), "a superseded check published: {late:?}");
    client.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn closing_a_document_during_a_check_suppresses_its_late_publishes() {
    let gate = Gate::default();
    let mut client = Client::start(Gated::engine(&gate), short_grace(), false).await;
    client.write("orders.csv", ORDERS);
    let uri = client.uri("query.ds");
    client
        .open(&uri, "from 'orders.csv' | where amount = 1\n")
        .await;
    wait_for(&gate.entered, "the check reads the schema").await;

    client
        .notify(
            "textDocument/didClose",
            json!({ "textDocument": { "uri": uri } }),
        )
        .await;
    // The clear, possibly after the check's syntax-only publish (if the close took longer than
    // the grace to arrive).
    let mut before = Vec::new();
    loop {
        let published = client.diagnostics(&uri).await;
        if published.version.is_none() && published.diagnostics.is_empty() {
            break;
        }
        before.push(published);
    }
    assert!(
        before
            .iter()
            .all(|p| p.version == Some(1) && p.codes() == ["DS0110"]),
        "{before:?}"
    );

    // Nothing is published about the closed document after the clear: not the grace's
    // syntax-only publish, not the check's final one.
    tokio::time::sleep(Duration::from_millis(400)).await;
    gate.release.notify_one();
    wait_for(&gate.left, "the check has its schema").await;
    let late = client
        .publishes_within(&uri, Duration::from_millis(300))
        .await;
    assert!(
        late.is_empty(),
        "a closed document got diagnostics: {late:?}"
    );

    // Opening it again checks it afresh.
    client
        .open(&uri, "from 'orders.csv' | project amout\n")
        .await;
    let published = client.diagnostics(&uri).await;
    assert_eq!(published.version, Some(1));
    assert_eq!(published.codes(), ["DS1001"]);
    client.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn surrogate_pairs_count_as_two_utf16_units_on_the_same_line() {
    let mut client = Client::start(datafusion(), one_publish(10_000), false).await;
    client.write("orders.csv", ORDERS);
    let uri = client.uri("emoji.ds");

    // `😀` is 4 bytes and 2 UTF-16 units. A diagnostic on it and one after it on the same line
    // are both in UTF-16 units: `'😀'` is 33..37 and `amout` 52..57 (bytes: 33..39, 54..59).
    client
        .open(
            &uri,
            "from 'orders.csv' | extend bad = '😀' + 1 | project amout\n",
        )
        .await;
    let published = client.diagnostics(&uri).await;
    assert_eq!(published.codes(), ["DS2003", "DS1001"]);
    assert_eq!(published.ranges(), ["0:33-0:37", "0:52-0:57"]);

    // A request position after `😀` on its line is read in UTF-16 units too, and the answer's
    // range is given in them. With seven `😀` before it, `amount` sits at UTF-16 56..62, bytes
    // 70..76 and chars 49..55, so the request position 59 (`amo‸unt`) lands inside `amount` only
    // when read as UTF-16 (as bytes it is inside the string; as chars it is past `> 1`).
    let hover = client
        .at_cursor(
            "textDocument/hover",
            &uri,
            2,
            "from 'orders.csv' | extend e = '😀😀😀😀😀😀😀' | where amo‸unt > 1",
        )
        .await;
    assert_eq!(
        hover["contents"]["value"],
        "```ds\namount: real\n```\ncolumn"
    );
    assert_eq!(
        hover["range"],
        json!({ "start": { "line": 0, "character": 56 }, "end": { "line": 0, "character": 62 } })
    );
    client.shutdown().await;
}

/// An engine whose first query panics as it starts; later queries run normally.
#[derive(Debug)]
struct PanicsOnce {
    inner: DataFusionEngine,
    armed: std::sync::atomic::AtomicBool,
}

#[async_trait::async_trait]
impl Engine for PanicsOnce {
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
        if self.armed.swap(false, std::sync::atomic::Ordering::SeqCst) {
            panic!("synthetic engine crash");
        }
        self.inner.execute(plan, ctx).await
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_panic_during_run_becomes_ds9001_and_the_server_stays_up() {
    let engine = Arc::new(PanicsOnce {
        inner: DataFusionEngine::default(),
        armed: std::sync::atomic::AtomicBool::new(true),
    });
    let mut client = Client::start(engine, one_publish(20), false).await;
    client.write("orders.csv", ORDERS);
    let uri = client.uri("crash.ds");
    client.open(&uri, "from 'orders.csv' | count\n").await;
    assert_eq!(client.diagnostics(&uri).await.codes(), Vec::<&str>::new());

    // The engine panics: the run fails with the bug report, not a JSON-RPC error, and the
    // diagnostic is published like any runtime error.
    let response = client
        .request_response("datascript/run", json!({ "textDocument": { "uri": uri } }))
        .await;
    assert!(response.get("error").is_none(), "{response}");
    let result = &response["result"];
    assert_eq!(result["failed"], true);
    let output = result["output"].as_str().unwrap();
    assert!(output.contains("error[DS9001]"), "{output}");
    assert!(output.contains("synthetic engine crash"), "{output}");
    let published = client.diagnostics(&uri).await;
    assert_eq!(published.version, Some(1));
    assert_eq!(published.codes(), ["DS9001"]);

    // The server is still up: the next run works and clears the bug report, and other
    // requests are answered.
    assert_eq!(
        client.run(&uri, None).await.unwrap(),
        "Count\nlong\n-----\n    2\n"
    );
    assert_eq!(client.diagnostics(&uri).await.codes(), Vec::<&str>::new());
    let lenses = client
        .request(
            "textDocument/codeLens",
            json!({ "textDocument": { "uri": uri } }),
        )
        .await;
    assert_eq!(lenses.as_array().map(Vec::len), Some(1), "{lenses}");
    client.shutdown().await;
}
