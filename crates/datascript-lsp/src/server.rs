//! The language server: document sync, debounced checks and diagnostics.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use datascript_diagnostics::{Diagnostic, DiagnosticSink, Diagnostics, FileId, SourceFile};
use datascript_engine::{Session, catch_panic};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::Mutex;
use tower_lsp_server::jsonrpc::Result;
use tower_lsp_server::ls_types::{
    DidChangeTextDocumentParams, DidChangeWatchedFilesParams,
    DidChangeWatchedFilesRegistrationOptions, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, FileSystemWatcher, GlobPattern, InitializeParams, InitializeResult,
    InitializedParams, Registration, ServerCapabilities, ServerInfo, TextDocumentSyncCapability,
    TextDocumentSyncKind, TextDocumentSyncOptions, Uri,
};
use tower_lsp_server::{Client, ClientSocket, LanguageServer, LspService, Server};

use crate::convert;
use crate::document::apply_changes;

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
    /// Source of [`Doc::generation`] values.
    generations: AtomicU64,
}

#[derive(Debug)]
struct Doc {
    text: String,
    version: i32,
    /// Changes whenever a check is scheduled; a check publishes only if it is still current.
    generation: u64,
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
    LspService::new(move |client| Backend {
        client,
        state: Arc::new(State {
            session,
            config,
            docs: Mutex::new(HashMap::new()),
            root: std::sync::Mutex::new(None),
            can_watch: AtomicBool::new(false),
            syntax_only: AtomicBool::new(false),
            generations: AtomicU64::new(0),
        }),
    })
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
        self.state.docs.lock().await.insert(
            doc.uri.clone(),
            Doc {
                text: doc.text,
                version: doc.version,
                generation: 0,
            },
        );
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
}

impl Backend {
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
        let docs = self.docs.lock().await;
        if docs.get(uri).is_none_or(|doc| doc.generation != generation) {
            return;
        }
        let diagnostics = convert::to_lsp_all(diagnostics, file, uri);
        client
            .publish_diagnostics(uri.clone(), diagnostics, Some(version))
            .await;
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
