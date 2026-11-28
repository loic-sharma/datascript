# M4-01 — Local web UI server (`ds ui`): auth, files API & check

**Status:** todo
**Depends on:** M4-08, M4-12, M4-13
**Crates:** `datascript-server`, `datascript-cli`
**Diagnostic codes:** none (sandbox errors are M4-12's `DS3007`)

## Goal
`ds ui [dir]` starts an axum server on 127.0.0.1 with a random port, prints the launch URL with a
per-launch token, and opens the browser. Implements cookie session auth, strict CSP, server-side
preferences, atomic workspace file API, whole-notebook `POST /api/check`, the
D-016 sandbox set to the root (hardened in M4-12), and embedded SPA serving ([WEB_UI.md §6](../WEB_UI.md#server-auth-csp--api-contract-datascript-server-m4-01-m4-10-m4-14-m4-04--m4-16)).
WebSocket streaming runs and row-limit capping land in M4-10; export in M4-14.

## Scope
- **Cookie auth, `Origin` checks & CSP**:
  - `GET /?token=<token>` validates the launch token, mints a separate random session id, sets
    `Set-Cookie: ds_session_<port>=<session id>; HttpOnly; SameSite=Strict; Path=/`, and `302`
    redirects to `/`. The port is in the name because cookies are shared by every server on
    `127.0.0.1` regardless of port, so a fixed name would let a second `ds ui` sign out the first
    ([WEB_UI.md §6](../WEB_UI.md#server-auth-csp--api-contract-datascript-server-m4-01-m4-10-m4-14-m4-04--m4-16)).
  - `GET /` with neither a valid cookie nor a valid token serves a small static page telling the
    user to open the URL `ds ui` printed.
  - Require the session cookie on all API routes, plus `Origin` check on mutating routes (`POST`,
    `PUT`, `PATCH`, `DELETE`).
  - Serve the SPA HTML with the strict Content-Security-Policy header in
    [WEB_UI.md §6](../WEB_UI.md#server-auth-csp--api-contract-datascript-server-m4-01-m4-10-m4-14-m4-04--m4-16).
- **Info, preferences & M4 v1 workspace files API** (all confined to root per D-016; safety rules
  in [WEB_UI.md §4.1](../WEB_UI.md#41-left-pane--working-directory-tree--attached-tables)):
  - `GET /api/info` & `PATCH /api/prefs`: persist per-root UI preferences (`row_limit`, theme) in
    the user's OS config dir (`dirs::config_dir()`) so settings survive `ds ui`'s random port.
  - `GET /api/files?dir=&includeHidden=`, `GET /api/schema?source=`.
  - `GET /api/file` (returns parsed cells `{ kind, text }` via M4-13 for `.dsnb`, or raw text for
    `.ds`, plus `version`), `POST /api/file` (`OpenOptions::create_new(true)`), and `PUT /api/file`
    (per-path `tokio::sync::Mutex`, `expectedVersion` `409` check, lossless content diff against
    the on-disk file via M4-13, atomic temp-file + `sync_all` + rename preserving Unix
    permissions). `PUT` requires `expectedVersion` (`428 Precondition Required` without it),
    returns `WriteResult { version }`, answers `200` with the current `version` (not `409`) when the
    version is stale but the file on disk already equals the request (normalized cells for `.dsnb`,
    LF text for `.ds`), and records the `version` it wrote per path so M4-10's
    watcher can ignore the server's own writes.
  - Refuse non-`.dsnb`/`.ds` paths and any path with a symlink component under the root
    (`symlink_metadata` + final `O_NOFOLLOW`).
- **Whole-notebook check & embedded SPA**:
  - `POST /api/check`: checks a `.ds` script or whole `.dsnb` notebook in one pass via M4-08's
    `StitchedProgram`, returning diagnostics grouped by `cellIndex` (UTF-16, D-025) and per-cell
    in-scope `let` summaries.
  - Embed built `web/` assets (`rust-embed`, placeholder `web/dist/index.html` until M4-02) plus a
    dev proxy to Vite. One shared `Arc<Session>` serves concurrent requests (D-011), with
    `ExecContext::sandbox` set to the workspace root (D-016, M4-12).

## Acceptance criteria
- [ ] API integration tests covering:
  - cookie auth (`GET /?token=` → `Set-Cookie` + `302`, cookie value ≠ launch token, subsequent
    `GET`/`POST` with cookie, missing/wrong cookie, the static page on `GET /` without either,
    bad `Origin` on mutating requests, strict CSP header on `/`);
  - **two servers at once** on different ports in one cookie jar: both stay authenticated after
    each one signs in;
  - server-persisted preferences (`PATCH /api/prefs` → `GET /api/info` across server restarts);
  - path traversal and workspace safety: `.csv`/`.parquet` write rejection, in-root symlink
    rejection (`notes.ds -> data/orders.csv`), atomic `O_CREAT|O_EXCL` create refusal on existing
    files, atomic save + `409` conflicts, and lossless content-diff `PUT`s including **two
    consecutive saves** (edit cell A, save, use the returned `version`, edit cell B, save: cell A
    and untouched cells stay byte-identical), a moved untouched cell keeping its bytes, the same `PUT`
    sent twice (second one gets `200` with the current version, not `409`), and a `PUT` without
    `expectedVersion` getting `428`;
  - whole-notebook `POST /api/check` with cross-cell `let`s and UTF-16 mapped spans.
- [ ] Through the API, `from '/etc/passwd' | take 1` and `from '../x.csv'` fail with `DS3007`
      (the sandbox is wired to the root; M4-12 covers the engine cases).

## Carried over
- [ ] (from planning, 2026-11-24) If M7-01 is done, `ExecContext::allow_remote` exists: leave it off for `ds ui` unless started with `--allow-remote`, with an API test that a URL source is rejected without it. (If M7-01 lands after this task, M7-01 does it.)

## Follow-ups
