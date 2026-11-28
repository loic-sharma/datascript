//! Local HTTP and WebSocket server (`ds ui`) for DataScript (M4-01, D-016, D-062).
//!
//! Serves:
//! - `/api/info` — server metadata and capabilities
//! - `/api/check` — check a program and return UTF-16 JSON diagnostics + query metadata
//! - `/api/run` — run a program (or one statement) with UI row limits, returning JSON rows or
//!   Arrow IPC + `RenderSpec` + diagnostics (`DS4060` when truncated)
//! - `/api/export` — stream the full untruncated result as CSV, JSON, NDJSON, or Arrow IPC
//! - `/api/files` — list recognized data files and subdirectories under the root directory
//! - `/api/schema` — inspect a source's columns and DataScript types
//! - `/api/ws` — WebSocket endpoint for batch streaming and mid-query cancellation
//! - `/*` — embedded `web/dist/` SPA assets (`rust-embed`), or reverse proxy to a Vite dev server
//!
//! Security:
//! - Always binds to `127.0.0.1` (random ephemeral port by default).
//! - Every `/api/*` request requires a per-launch CSPRNG session token and checks `Origin` / `Host`.
//! - Query file sources are confined to `root_dir` via `ExecContext::sandbox` (`DS3007`, D-016)
//!   unless `sandboxed` is disabled (`--allow-outside-root`).

mod api;
mod assets;
mod auth;
mod watcher;
mod ws;

use std::fmt;
use std::future::Future;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::middleware;
use axum::routing::{any, get, post};
use datascript_engine::printer::DEFAULT_UI_ROW_LIMIT;
use datascript_engine::{ExecContext, Session};
use tokio::net::TcpListener;

pub use auth::TOKEN_HEADER;
pub use watcher::RootWatcher;

/// JSON API contract version. Bumped on any breaking change; frontends check it.
pub const API_VERSION: u32 = 1;

/// Configuration for [`bind`].
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// The root directory served by `/api/files` and used as `base_dir` (and `sandbox` when
    /// `sandboxed` is true).
    pub root_dir: PathBuf,
    /// Confine query sources to `root_dir` (`DS3007`, D-016). Defaults to `true`.
    pub sandboxed: bool,
    /// Default per-query row limit for `/api/run` and `/api/ws` when a request omits `rowLimit`
    /// (`Some(10_000)` by default, D-062). `None` disables the default limit.
    pub default_row_limit: Option<usize>,
    /// Port to bind on `127.0.0.1`. `0` lets the OS pick a free ephemeral port.
    pub port: u16,
    /// Optional fixed token (for tests). When `None`, a fresh 256-bit random token is generated.
    pub token: Option<String>,
    /// Optional Vite / dev server origin URL (e.g. `http://127.0.0.1:5173`). When set, non-API
    /// requests are reverse-proxied there instead of serving embedded `web/dist/` assets.
    pub dev_proxy: Option<String>,
    /// Names of host tables registered on the session's engine, for `/api/info`, `/api/files`,
    /// and `/api/schema`.
    pub tables: Vec<String>,
    /// Whether to start a recursive filesystem watcher on `root_dir` that invalidates the
    /// session's schema cache when files change. Defaults to `true`.
    pub watch_files: bool,
}

impl ServerConfig {
    /// Default configuration serving `root_dir` on a random loopback port with sandboxing and
    /// the standard 10,000-row UI limit enabled.
    pub fn new(root_dir: impl Into<PathBuf>) -> Self {
        Self {
            root_dir: root_dir.into(),
            sandboxed: true,
            default_row_limit: Some(DEFAULT_UI_ROW_LIMIT),
            port: 0,
            token: None,
            dev_proxy: None,
            tables: Vec::new(),
            watch_files: true,
        }
    }
}

/// Shared state held behind an `Arc` across all axum handlers (D-011).
pub struct AppState {
    pub(crate) session: Arc<Session>,
    pub(crate) root_dir: PathBuf,
    pub(crate) sandboxed: bool,
    pub(crate) default_row_limit: Option<usize>,
    pub(crate) token: String,
    pub(crate) allowed_origins: Vec<String>,
    pub(crate) dev_proxy: Option<String>,
    pub(crate) tables: Vec<String>,
}

impl fmt::Debug for AppState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AppState")
            .field("root_dir", &self.root_dir)
            .field("sandboxed", &self.sandboxed)
            .field("default_row_limit", &self.default_row_limit)
            .field("allowed_origins", &self.allowed_origins)
            .field("dev_proxy", &self.dev_proxy)
            .field("tables", &self.tables)
            .finish_non_exhaustive()
    }
}

impl AppState {
    /// Builds `AppState` for a bound port (or port `0` in direct `Router` unit tests).
    pub fn new(session: Arc<Session>, config: &ServerConfig, port: u16) -> Self {
        let token = config.token.clone().unwrap_or_else(generate_token);
        let mut allowed_origins = Vec::new();
        if port > 0 {
            allowed_origins.push(format!("http://127.0.0.1:{port}"));
            allowed_origins.push(format!("http://localhost:{port}"));
            allowed_origins.push(format!("http://[::1]:{port}"));
        }
        if let Some(proxy) = &config.dev_proxy
            && let Some(origin) = origin_of_url(proxy)
        {
            allowed_origins.push(origin.clone());
            // Also allow the localhost <-> 127.0.0.1 twin of the dev server.
            if let Some(rest) = origin.strip_prefix("http://127.0.0.1") {
                allowed_origins.push(format!("http://localhost{rest}"));
            } else if let Some(rest) = origin.strip_prefix("http://localhost") {
                allowed_origins.push(format!("http://127.0.0.1{rest}"));
            }
        }
        let mut tables = config.tables.clone();
        tables.sort();
        tables.dedup();
        Self {
            session,
            root_dir: config.root_dir.clone(),
            sandboxed: config.sandboxed,
            default_row_limit: config.default_row_limit,
            token,
            allowed_origins,
            dev_proxy: config.dev_proxy.clone(),
            tables,
        }
    }

    /// Returns the session token.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Whether `origin` is permitted to call `/api/*`.
    pub(crate) fn is_allowed_origin(&self, origin: &str) -> bool {
        self.allowed_origins
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(origin))
    }

    /// Builds an [`ExecContext`] for one request, applying the root directory, sandbox (D-016),
    /// and row limit (D-062).
    pub(crate) fn exec_context(&self, row_limit: Option<usize>) -> ExecContext {
        let mut ctx = self.session.exec_context(&self.root_dir);
        if self.sandboxed {
            ctx.sandbox = Some(self.root_dir.clone());
        }
        ctx.row_limit = row_limit;
        ctx
    }

    /// Resolves the effective row limit for a request: omitted uses `default_row_limit`, `Some(0)`
    /// means unlimited (`None`), and `Some(n)` means `Some(n)`.
    pub(crate) fn effective_row_limit(&self, requested: Option<usize>) -> Option<usize> {
        match requested {
            None => self.default_row_limit,
            Some(0) => None,
            Some(n) => Some(n),
        }
    }
}

/// Builds the full axum [`Router`] for `state`.
pub fn router(state: Arc<AppState>) -> Router {
    let api_routes = Router::new()
        .route("/info", get(api::info))
        .route("/check", post(api::check))
        .route("/run", post(api::run))
        .route("/export", post(api::export))
        .route("/files", get(api::files))
        .route("/schema", get(api::schema))
        .route("/ws", get(ws::ws_handler))
        .fallback(any(api::not_found))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_auth,
        ));

    Router::new()
        .nest("/api", api_routes)
        .fallback(assets::serve_frontend)
        .with_state(state)
}

/// A bound server ready to accept connections.
pub struct RunningServer {
    listener: TcpListener,
    addr: SocketAddr,
    state: Arc<AppState>,
    _watcher: Option<RootWatcher>,
}

impl fmt::Debug for RunningServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RunningServer")
            .field("addr", &self.addr)
            .field("root_dir", &self.state.root_dir)
            .finish_non_exhaustive()
    }
}

impl RunningServer {
    /// The loopback address the server is bound to.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The per-launch session token.
    pub fn token(&self) -> &str {
        &self.state.token
    }

    /// The launch URL containing the session token (`http://127.0.0.1:<port>/?token=<token>`).
    pub fn url(&self) -> String {
        format!("http://{}/?token={}", self.addr, self.state.token)
    }

    /// The shared server state.
    pub fn state(&self) -> &Arc<AppState> {
        &self.state
    }

    /// Serves requests until `shutdown` resolves.
    pub async fn serve_with_graceful_shutdown<F>(self, shutdown: F) -> io::Result<()>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let RunningServer {
            listener,
            state,
            _watcher,
            ..
        } = self;
        let app = router(state);
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown)
            .await
    }
}

/// Binds the server to `127.0.0.1:<config.port>` and optionally starts the root file watcher.
pub async fn bind(session: Arc<Session>, config: ServerConfig) -> io::Result<RunningServer> {
    let root_dir =
        std::fs::canonicalize(&config.root_dir).unwrap_or_else(|_| config.root_dir.clone());
    let config = ServerConfig { root_dir, ..config };

    let bind_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), config.port);
    let listener = TcpListener::bind(bind_addr).await?;
    let addr = listener.local_addr()?;

    let watcher = if config.watch_files {
        match RootWatcher::watch(&config.root_dir, session.clone()) {
            Ok(w) => Some(w),
            Err(err) => {
                eprintln!(
                    "warning: could not watch `{}` for file changes: {err}",
                    config.root_dir.display()
                );
                None
            }
        }
    } else {
        None
    };

    let state = Arc::new(AppState::new(session, &config, addr.port()));
    Ok(RunningServer {
        listener,
        addr,
        state,
        _watcher: watcher,
    })
}

/// Generates a 256-bit cryptographically random lowercase hex session token.
pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("OS CSPRNG should provide random bytes");
    let mut out = String::with_capacity(64);
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Extracts `scheme://authority` from a URL string (e.g. `http://localhost:5173/foo` ->
/// `http://localhost:5173`).
fn origin_of_url(url: &str) -> Option<String> {
    let uri: axum::http::Uri = url.parse().ok()?;
    let scheme = uri.scheme_str()?;
    let authority = uri.authority()?.as_str();
    Some(format!("{scheme}://{authority}"))
}
