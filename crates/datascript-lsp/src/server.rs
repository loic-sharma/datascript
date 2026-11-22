//! The language server: document sync, debounced checks and diagnostics, and Run (M3-05).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use datascript_diagnostics::{Diagnostic, DiagnosticSink, Diagnostics, FileId, SourceFile};
use datascript_engine::{Session, catch_panic};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::Mutex;
use tower_lsp_server::jsonrpc::{self, Result};
use tower_lsp_server::ls_types::{
    self as lsp, CodeLens, CodeLensOptions, CodeLensParams, CompletionOptions, CompletionParams,
    CompletionResponse, DidChangeTextDocumentParams, DidChangeWatchedFilesParams,
    DidChangeWatchedFilesRegistrationOptions, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, FileSystemWatcher, GlobPattern, HoverParams,
    HoverProviderCapability, InitializeParams, InitializeResult, InitializedParams, Registration,
    ServerCapabilities, ServerInfo, SignatureHelpOptions, SignatureHelpParams,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextDocumentSyncOptions, Uri,
};
use tower_lsp_server::{Client, ClientSocket, LanguageServer, LspService, Server};

use crate::convert;
use crate::document::{apply_changes, offset_at};
use crate::ide;
use crate::run::{self, RUN_METHOD, RunParams, RunResult};

/// The JSON-RPC error code for a request the server refuses (LSP `RequestFailed`).
const REQUEST_FAILED: i64 = -32803;

/// Every document is checked as file 0: the server checks one document at a time.
const FILE: FileId = FileId(0);

/// The extension of DataScript scripts. Changes to other files may change a source's schema.
const SCRIPT_EXTENSION: &str = "ds";

/// Timing of the server's checks.
#[derive(Debug, Clone)]
pub struct Config {
    /// How long a document must stay unchanged before it is checked, so typing does not start
    /// a check per keystroke.
    pub debounce: Duration,
    /// How long a check may take before the syntax diagnostics are published on their own. With
    /// warm schemas a check finishes well within it and the editor sees one update; when schemas
    /// must be read first, syntax errors show at once and the rest follow.
    pub syntax_grace: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            debounce: Duration::from_millis(150),
            syntax_grace: Duration::from_millis(50),
        }
    }
}

/// The language server. Build it with [`service`]; tower-lsp-server calls its methods.
#[derive(Debug)]
pub struct Backend {
    client: Client,
    state: Arc<State>,
}

#[derive(Debug)]
struct State {
    session: Arc<Session>,
    config: Config,
    /// Open documents by URI. Held while publishing, so a stale check can never publish after a
    /// newer one.
    docs: Mutex<HashMap<Uri, Doc>>,
    /// The first workspace folder: the base directory of documents that are not files.
    root: std::sync::Mutex<Option<PathBuf>>,
    /// Whether the client can watch files for us (`workspace/didChangeWatchedFiles`).
    can_watch: AtomicBool,
    /// Report only lexical and syntax errors, never reading a file ([`InitOptions`]).
    syntax_only: AtomicBool,
    /// Whether the client expands snippets in completion items.
    snippets: AtomicBool,
    /// Source of [`Doc::generation`] values.
    generations: AtomicU64,
}

#[derive(Debug)]
struct Doc {
    text: String,
    version: i32,
    /// Changes whenever a check is scheduled; a check publishes only if it is still current.
    generation: u64,
    /// What the latest check published, so a run's error can be published next to it.
    checked: Vec<lsp::Diagnostic>,
    /// The runtime error of the latest run of this text, published after the check's
    /// diagnostics until the document changes.
    runtime: Vec<lsp::Diagnostic>,
}

impl Doc {
    fn new(text: String, version: i32) -> Self {
        Doc {
            text,
            version,
            generation: 0,
            checked: Vec::new(),
            runtime: Vec::new(),
        }
    }

    /// Everything to publish: the check's diagnostics, then the run's.
    fn published(&self) -> Vec<lsp::Diagnostic> {
        self.checked.iter().chain(&self.runtime).cloned().collect()
    }
}

/// The `initializationOptions` the server understands. Unknown fields, and fields of the wrong
/// type, are ignored, so a client written for a newer server still gets the full check.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct InitOptions {
    /// `"syntaxOnly": true`: publish only lexical and syntax diagnostics. The server then reads no
    /// files (no schemas) and watches none, so it is safe in a workspace the user does not trust
    /// (VS Code's Workspace Trust, ARCHITECTURE.md § Frontends).
    syntax_only: bool,
}

impl InitOptions {
    fn from_json(options: Option<&serde_json::Value>) -> Self {
        let flag = |name: &str| {
            options
                .and_then(|o| o.get(name))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
        };
        InitOptions {
            syntax_only: flag("syntaxOnly"),
        }
    }
}

/// The LSP service over `session`, for [`Server`] or an in-process client.
pub fn service(session: Arc<Session>, config: Config) -> (LspService<Backend>, ClientSocket) {
    LspService::build(move |client| Backend {
        client,
        state: Arc::new(State {
            session,
            config,
            docs: Mutex::new(HashMap::new()),
            root: std::sync::Mutex::new(None),
            can_watch: AtomicBool::new(false),
            syntax_only: AtomicBool::new(false),
            snippets: AtomicBool::new(false),
            generations: AtomicU64::new(0),
        }),
    })
    .custom_method(RUN_METHOD, Backend::run)
    .finish()
}

/// Serves the LSP over `input` and `output` until the client exits.
pub async fn serve(
    input: impl AsyncRead + Unpin,
    output: impl AsyncWrite,
    session: Arc<Session>,
    config: Config,
) {
    let (service, socket) = service(session, config);
    Server::new(input, output, socket).serve(service).await;
}

/// Serves the LSP over stdin and stdout (`ds lsp`).
pub async fn serve_stdio(session: Arc<Session>) {
    serve(
        tokio::io::stdin(),
        tokio::io::stdout(),
        session,
        Config::default(),
    )
    .await;
}

impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        #[allow(deprecated)] // `root_uri` is the fallback for clients without workspace folders
        let root = params
            .workspace_folders
            .as_ref()
            .and_then(|folders| folders.first())
            .map(|folder| &folder.uri)
            .or(params.root_uri.as_ref())
            .and_then(file_path);
        *self.state.root.lock().unwrap_or_else(|e| e.into_inner()) = root;
        let can_watch = params
            .capabilities
            .workspace
            .as_ref()
            .and_then(|w| w.did_change_watched_files.as_ref())
            .and_then(|w| w.dynamic_registration)
            .unwrap_or(false);
        self.state.can_watch.store(can_watch, Ordering::Relaxed);
        let snippets = params
            .capabilities
            .text_document
            .as_ref()
            .and_then(|t| t.completion.as_ref())
            .and_then(|c| c.completion_item.as_ref())
            .and_then(|i| i.snippet_support)
            .unwrap_or(false);
        self.state.snippets.store(snippets, Ordering::Relaxed);
        let options = InitOptions::from_json(params.initialization_options.as_ref());
        self.state
            .syntax_only
            .store(options.syntax_only, Ordering::Relaxed);
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Options(
                    TextDocumentSyncOptions {
                        open_close: Some(true),
                        change: Some(TextDocumentSyncKind::INCREMENTAL),
                        ..TextDocumentSyncOptions::default()
                    },
                )),
                code_lens_provider: Some(CodeLensOptions {
                    resolve_provider: Some(false),
                }),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(
                        ["|", "(", ",", "=", "'", "\"", "/"]
                            .map(String::from)
                            .to_vec(),
                    ),
                    ..CompletionOptions::default()
                }),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                signature_help_provider: Some(SignatureHelpOptions {
                    trigger_characters: Some(vec!["(".into(), ",".into()]),
                    retrigger_characters: None,
                    work_done_progress_options: Default::default(),
                }),
                ..ServerCapabilities::default()
            },
            server_info: Some(ServerInfo {
                name: "ds".into(),
                version: Some(env!("CARGO_PKG_VERSION").into()),
            }),
            offset_encoding: None,
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        // A syntax-only server reads no data files, so their changes do not matter to it.
        if !self.state.can_watch.load(Ordering::Relaxed)
            || self.state.syntax_only.load(Ordering::Relaxed)
        {
            return;
        }
        // Any file may be a source (`read_csv('data.txt')`, globs), so watch them all and skip
        // scripts when a change comes in.
        let options = DidChangeWatchedFilesRegistrationOptions {
            watchers: vec![FileSystemWatcher {
                glob_pattern: GlobPattern::String("**/*".into()),
                kind: None,
            }],
        };
        let registration = Registration {
            id: "datascript-data-files".into(),
            method: "workspace/didChangeWatchedFiles".into(),
            register_options: serde_json::to_value(options).ok(),
        };
        if let Err(e) = self.client.register_capability(vec![registration]).await {
            self.client
                .log_message(
                    tower_lsp_server::ls_types::MessageType::WARNING,
                    format!(
                        "cannot watch data files, so changed files are noticed only after a restart: {e}"
                    ),
                )
                .await;
        }
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let doc = params.text_document;
        self.state
            .docs
            .lock()
            .await
            .insert(doc.uri.clone(), Doc::new(doc.text, doc.version));
        // Opening is not typing: check at once.
        self.schedule(doc.uri, Duration::ZERO).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        {
            let mut docs = self.state.docs.lock().await;
            let Some(doc) = docs.get_mut(&uri) else {
                return;
            };
            apply_changes(&mut doc.text, params.content_changes);
            doc.version = params.text_document.version;
            // A run's error was about the old text; the next check publishes without it.
            doc.runtime.clear();
        }
        self.schedule(uri, self.state.config.debounce).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        let mut docs = self.state.docs.lock().await;
        if docs.remove(&uri).is_some() {
            self.client.publish_diagnostics(uri, Vec::new(), None).await;
        }
    }

    async fn did_change_watched_files(&self, params: DidChangeWatchedFilesParams) {
        if self.state.syntax_only.load(Ordering::Relaxed) {
            return;
        }
        let data_changed = params.changes.iter().any(|change| {
            file_path(&change.uri).is_none_or(|path| {
                path.extension()
                    .is_none_or(|ext| !ext.eq_ignore_ascii_case(SCRIPT_EXTENSION))
            })
        });
        if !data_changed {
            return;
        }
        // A changed file can change any cached schema (a glob that now matches it, a file that
        // now exists), so forget them all; the next check reads what it needs again (D-043).
        self.state.session.invalidate_schemas();
        let uris: Vec<Uri> = self.state.docs.lock().await.keys().cloned().collect();
        for uri in uris {
            self.schedule(uri, self.state.config.debounce).await;
        }
    }

    async fn code_lens(&self, params: CodeLensParams) -> Result<Option<Vec<CodeLens>>> {
        // Nothing runs in a workspace the user does not trust (D-053).
        if self.state.syntax_only.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let uri = params.text_document.uri;
        let Some(text) = self.state.text_of(&uri).await else {
            return Ok(None);
        };
        let file = SourceFile::new(self.state.display_name(&uri), text);
        Ok(Some(run::code_lenses(&file, FILE, &uri)))
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let at = params.text_document_position;
        let Some(cursor) = self.state.analyze(&at.text_document.uri, at.position).await else {
            return Ok(None);
        };
        let completions = match catch_panic(Some(FILE), "completing", || {
            datascript_compiler::ide::complete(&cursor.file, &cursor.analysis, cursor.offset)
        }) {
            Ok(completions) => completions,
            Err(crash) => {
                self.log_crash(&crash).await;
                return Ok(None);
            }
        };
        // A syntax-only server lists no files (D-053).
        let base_dir =
            (!self.state.syntax_only.load(Ordering::Relaxed)).then_some(&cursor.base_dir);
        let items = ide::completion_items(
            &cursor.file,
            FILE,
            completions,
            self.state.snippets.load(Ordering::Relaxed),
            base_dir.map(PathBuf::as_path),
        );
        Ok(Some(CompletionResponse::Array(items)))
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<lsp::Hover>> {
        let at = params.text_document_position_params;
        let Some(cursor) = self.state.analyze(&at.text_document.uri, at.position).await else {
            return Ok(None);
        };
        match catch_panic(Some(FILE), "finding what to show on hover", || {
            datascript_compiler::ide::hover(&cursor.file, &cursor.analysis, cursor.offset)
        }) {
            Ok(found) => Ok(found.map(|h| ide::hover(&cursor.file, FILE, h))),
            Err(crash) => {
                self.log_crash(&crash).await;
                Ok(None)
            }
        }
    }

    async fn signature_help(
        &self,
        params: SignatureHelpParams,
    ) -> Result<Option<lsp::SignatureHelp>> {
        let at = params.text_document_position_params;
        let Some(cursor) = self.state.analyze(&at.text_document.uri, at.position).await else {
            return Ok(None);
        };
        match catch_panic(Some(FILE), "finding the signature", || {
            datascript_compiler::ide::signature_help(&cursor.file, &cursor.analysis, cursor.offset)
        }) {
            Ok(found) => Ok(found.map(ide::signature_help)),
            Err(crash) => {
                self.log_crash(&crash).await;
                Ok(None)
            }
        }
    }
}

/// A document checked for a request at a position.
struct AtCursor {
    file: SourceFile,
    analysis: datascript_compiler::Analysis,
    /// The byte offset of the position.
    offset: usize,
    base_dir: PathBuf,
}

impl Backend {
    /// Tells the client about a compiler bug a request ran into; the request answers nothing.
    async fn log_crash(&self, crash: &Diagnostic) {
        self.client
            .log_message(lsp::MessageType::ERROR, crash.message.clone())
            .await;
    }

    /// `datascript/run`: runs the server's text of an open document as `ds run` would, and
    /// publishes the runtime error, if any, next to the check's diagnostics.
    ///
    /// Cancelling the request (`$/cancelRequest`) drops this future, and with it the running
    /// query, which stops it.
    pub async fn run(&self, params: RunParams) -> Result<RunResult> {
        if self.state.syntax_only.load(Ordering::Relaxed) {
            return Err(refused(
                "running queries is off in a workspace you do not trust; trust the workspace to \
                 run it",
            ));
        }
        let uri = params.text_document.uri.clone();
        let (text, version) = {
            let docs = self.state.docs.lock().await;
            match docs.get(&uri) {
                Some(doc) => (doc.text.clone(), doc.version),
                None => {
                    return Err(jsonrpc::Error::invalid_params(format!(
                        "`{}` is not open; open it in the editor before running it",
                        uri.as_str()
                    )));
                }
            }
        };
        let file = SourceFile::new(self.state.display_name(&uri), text);
        let path = file_path(&uri);
        let ctx = self
            .state
            .session
            .exec_context(&self.state.base_dir(path.as_deref()));
        // The query runs on its own task: the server reads and answers messages on one task, so
        // a query polled there would hold up every message, `$/cancelRequest` included, while it
        // computes. Cancelling drops this future, and the guard aborts the task, which drops the
        // running query at its next yield.
        let session = Arc::clone(&self.state.session);
        let task_file = file.clone();
        let mut task = AbortOnDrop(tokio::spawn(async move {
            run::run(&session, &task_file, FILE, &ctx, &params).await
        }));
        let ran = match (&mut task.0).await {
            Ok(ran) => ran.map_err(|refusal| refused(refusal.message()))?,
            Err(e) => {
                return Err(jsonrpc::Error {
                    code: jsonrpc::ErrorCode::InternalError,
                    message: format!("the run stopped unexpectedly: {e}").into(),
                    data: None,
                });
            }
        };

        let runtime: Vec<lsp::Diagnostic> = ran
            .runtime_error
            .iter()
            .map(|d| convert::to_lsp(d, &file, &uri))
            .collect();
        let mut docs = self.state.docs.lock().await;
        // Publish only about the text that ran; an edit since then makes the error stale.
        if let Some(doc) = docs.get_mut(&uri).filter(|doc| doc.version == version)
            && doc.runtime != runtime
        {
            doc.runtime = runtime;
            self.client
                .publish_diagnostics(uri, doc.published(), Some(version))
                .await;
        }
        Ok(ran.result)
    }

    /// Checks `uri` after `delay`, unless it changes again first, and publishes its diagnostics.
    async fn schedule(&self, uri: Uri, delay: Duration) {
        let generation = self.state.generations.fetch_add(1, Ordering::Relaxed) + 1;
        {
            let mut docs = self.state.docs.lock().await;
            let Some(doc) = docs.get_mut(&uri) else {
                return;
            };
            doc.generation = generation;
        }
        let state = Arc::clone(&self.state);
        let client = self.client.clone();
        tokio::spawn(async move {
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            state.check(&client, uri, generation).await;
        });
    }
}

impl State {
    /// Checks the server's text of `uri` for a request at `position`. A syntax-only server
    /// checks without schemas, so it reads no files and knows no columns (D-053).
    async fn analyze(&self, uri: &Uri, position: lsp::Position) -> Option<AtCursor> {
        let text = self.text_of(uri).await?;
        let offset = offset_at(&text, position);
        let file = SourceFile::new(self.display_name(uri), text);
        let path = file_path(uri);
        let base_dir = self.base_dir(path.as_deref());
        let analysis = if self.syntax_only.load(Ordering::Relaxed) {
            let caps = self.session.engine().capabilities();
            catch_panic(Some(FILE), "checking the program", || {
                datascript_compiler::check(&file, FILE, &datascript_compiler::NoSchemas, caps)
            })
            .ok()?
        } else {
            let ctx = self.session.exec_context(&base_dir);
            self.session.check(&file, FILE, &ctx).await
        };
        Some(AtCursor {
            file,
            analysis,
            offset,
            base_dir,
        })
    }

    /// Checks the document if `generation` is still its latest, publishing syntax diagnostics
    /// first when the full check is slow.
    async fn check(&self, client: &Client, uri: Uri, generation: u64) {
        let (text, version) = {
            let docs = self.docs.lock().await;
            match docs.get(&uri) {
                Some(doc) if doc.generation == generation => (doc.text.clone(), doc.version),
                _ => return,
            }
        };
        let path = file_path(&uri);
        let name = path.as_deref().and_then(Path::file_name).map_or_else(
            || uri.as_str().to_owned(),
            |n| n.to_string_lossy().into_owned(),
        );
        let file = SourceFile::new(name, text);
        if self.syntax_only.load(Ordering::Relaxed) {
            let syntax = syntax_diagnostics(&file);
            self.publish(client, &uri, generation, version, &file, &syntax)
                .await;
            return;
        }
        let ctx = self.session.exec_context(&self.base_dir(path.as_deref()));
        let check = self.session.check(&file, FILE, &ctx);
        tokio::pin!(check);
        let analysis = match tokio::time::timeout(self.config.syntax_grace, &mut check).await {
            Ok(analysis) => analysis,
            Err(_) => {
                let syntax = syntax_diagnostics(&file);
                self.publish(client, &uri, generation, version, &file, &syntax)
                    .await;
                check.await
            }
        };
        self.publish(
            client,
            &uri,
            generation,
            version,
            &file,
            &analysis.diagnostics,
        )
        .await;
    }

    /// Publishes `diagnostics` if `generation` is still the document's latest.
    async fn publish(
        &self,
        client: &Client,
        uri: &Uri,
        generation: u64,
        version: i32,
        file: &SourceFile,
        diagnostics: &[Diagnostic],
    ) {
        let mut docs = self.docs.lock().await;
        let Some(doc) = docs.get_mut(uri).filter(|doc| doc.generation == generation) else {
            return;
        };
        doc.checked = convert::to_lsp_all(diagnostics, file, uri);
        client
            .publish_diagnostics(uri.clone(), doc.published(), Some(version))
            .await;
    }

    /// The server's text of an open document.
    async fn text_of(&self, uri: &Uri) -> Option<String> {
        self.docs.lock().await.get(uri).map(|doc| doc.text.clone())
    }

    /// The name a document's diagnostics show (`--> name:line:col`): its path relative to the
    /// workspace folder when it is inside it, as `ds run` run from that folder shows it; else
    /// its full path, or its URI when it is not a file.
    fn display_name(&self, uri: &Uri) -> String {
        let Some(path) = file_path(uri) else {
            return uri.as_str().to_owned();
        };
        let root = self.root.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let shown = root
            .as_deref()
            .and_then(|root| path.strip_prefix(root).ok())
            .unwrap_or(&path);
        shown.display().to_string()
    }

    /// Relative paths in a document resolve against its directory; for a document that is not
    /// a file, against the workspace folder or else the server's working directory.
    fn base_dir(&self, path: Option<&Path>) -> PathBuf {
        if let Some(dir) = path.and_then(Path::parent) {
            return dir.to_path_buf();
        }
        if let Some(root) = self.root.lock().unwrap_or_else(|e| e.into_inner()).clone() {
            return root;
        }
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    }
}

/// Aborts a task when dropped, so cancelling a request stops the work it started.
struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// A refused request: the reason is for the user.
fn refused(message: impl Into<String>) -> jsonrpc::Error {
    jsonrpc::Error {
        code: jsonrpc::ErrorCode::ServerError(REQUEST_FAILED),
        message: message.into().into(),
        data: None,
    }
}

/// The path of a `file:` URI. Other schemes (`untitled:`) have none.
fn file_path(uri: &Uri) -> Option<PathBuf> {
    if !uri.scheme().as_str().eq_ignore_ascii_case("file") {
        return None;
    }
    uri.to_file_path().map(|path| path.into_owned())
}

/// The lexical and syntax diagnostics of `file`, sorted and capped like a full check's.
fn syntax_diagnostics(file: &SourceFile) -> Vec<Diagnostic> {
    let errors = catch_panic(Some(FILE), "parsing the program", || {
        datascript_syntax::parse(file.text(), FILE).errors
    })
    .unwrap_or_else(|crash| vec![crash]);
    let mut diagnostics = Diagnostics::new();
    for d in errors {
        diagnostics.emit(d);
    }
    diagnostics.finish()
}
