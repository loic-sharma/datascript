# M4-01 — Local web UI server (`ds ui`): auth, files API, check & sandbox

**Status:** todo
**Depends on:** M4-08
**Crates:** `datascript-server`, `datascript-cli`, `datascript-datafusion`, `datascript-diagnostics`
**Diagnostic codes:** DS3007 (planned, D-016), DS3065–DS3069 (sources) (D-018)

## Goal
`ds ui [dir]` starts an axum server on 127.0.0.1 with a random port, prints the launch URL with a
per-launch token, and opens the browser. Implements cookie session auth, strict CSP, server-side
preferences, atomic workspace file API, whole-notebook `POST /api/check`, M2-29 sandbox hardening,
and embedded SPA serving ([WEB_UI.md §6](../WEB_UI.md#server-auth-csp--api-contract-datascript-server-m4-01-m4-10-m4-04--m4-05)).
WebSocket streaming runs, row-limit capping, and export land in M4-10.

## Scope
- **Cookie auth, `Origin` checks & CSP**:
  - `GET /?token=<token>` validates the launch token, sets
    `Set-Cookie: ds_session=<token>; HttpOnly; SameSite=Strict; Path=/`, and `302` redirects to `/`.
  - Require `ds_session` cookie on all API routes, plus `Origin` check on mutating routes (`POST`,
    `PUT`, `PATCH`, `DELETE`).
  - Serve the SPA HTML with the strict Content-Security-Policy header in
    [WEB_UI.md §6](../WEB_UI.md#server-auth-csp--api-contract-datascript-server-m4-01-m4-10-m4-04--m4-05).
- **Info, preferences & M4 v1 workspace files API** (all confined to root per D-016; safety rules
  in [WEB_UI.md §4.1](../WEB_UI.md#41-left-pane--working-directory-tree--attached-tables)):
  - `GET /api/info` & `PATCH /api/prefs`: persist per-root UI preferences (`row_limit`, theme) in
    the user's OS config dir (`dirs::config_dir()`) so settings survive `ds ui`'s random port.
  - `GET /api/files?dir=&includeHidden=`, `GET /api/schema?source=`.
  - `GET /api/file` (returns parsed cells with `originId` via M4-08 for `.dsnb`, or raw text for
    `.ds`, plus `version`), `POST /api/file` (`OpenOptions::create_new(true)`), and `PUT /api/file`
    (per-path `tokio::sync::Mutex`, `expectedVersion` `409` check, `originId` lossless diff via
    M4-08, atomic temp-file + `sync_all` + rename preserving Unix permissions).
  - Refuse non-`.dsnb`/`.ds` paths and any path with a symlink component under the root
    (`symlink_metadata` + final `O_NOFOLLOW`).
- **Whole-notebook check & embedded SPA**:
  - `POST /api/check`: checks a `.ds` script or whole `.dsnb` notebook in one pass via M4-08's
    `StitchedProgram`, returning diagnostics grouped by `cellIndex` (UTF-16, D-025) and per-cell
    in-scope `let` summaries.
  - Embed built `web/` assets (`rust-embed`, placeholder `web/dist/index.html` until M4-02) plus a
    dev proxy to Vite. One shared `Arc<Session>` serves concurrent requests (D-011).

## Acceptance criteria
- [ ] API integration tests covering:
  - cookie auth (`GET /?token=` → `Set-Cookie` + `302`, subsequent `GET`/`POST` with cookie,
    missing/wrong cookie, bad `Origin` on mutating requests, strict CSP header on `/`);
  - server-persisted preferences (`PATCH /api/prefs` → `GET /api/info` across server restarts);
  - path traversal and workspace safety: `.csv`/`.parquet` write rejection, in-root symlink
    rejection (`notes.ds -> data/orders.csv`), atomic `O_CREAT|O_EXCL` create refusal on existing
    files, atomic save + `409` conflicts, and `originId` lossless `PUT` diffing;
  - whole-notebook `POST /api/check` with cross-cell `let`s and UTF-16 mapped spans.
- [ ] Running `from '/etc/passwd' | take 1`, `from '../x.csv'` and a symlink pointing out of the
      root all fail with DS3007 (D-016, D-036).

## Carried over
- [ ] (from M1-08) The sandbox hook exists: with `ExecContext::sandbox` set, the DataFusion engine returns `SourceError::OutsideSandbox` (paths compared after resolving symlinks and `..`, globs per match) and the binder reports DS3007 with draft wording. Set the sandbox to the UI root, review the wording, add golden or API tests, and mark DS3007 active in `codes.rs` and DIAGNOSTICS.md.
- [ ] (from M2-29) Fix two verified sandbox escapes in `check_sandbox` / `glob_files` (`crates/datascript-datafusion/src/sources.rs`) before relying on the hook: (1) a missing file under a symlinked directory falls back to lexical `normalize()`, so an intermediate symlink can leave the root; canonicalize the deepest existing ancestor, then append and normalize the rest. (2) A glob that escapes the root but matches nothing (`../*.nope`, `/outside/*.csv`) returns DS3004 instead of DS3007, because only matches are checked; check the glob's non-wildcard prefix against the sandbox first.
- [ ] (from M2-29) Cover the sandbox with a `TempDir` integration suite against `Session` + `DataFusionEngine` (`#[cfg(unix)]` for symlinks): file and directory symlinks pointing out (existing and missing targets), escaping globs with zero, all-outside and mixed matches, a sibling-prefix directory (`root2/a.csv` against `root`), absolute paths inside and outside, and host tables registered inside vs outside the root.
- [ ] (from planning, 2026-11-24) If M2-25 is done, apply D-016's sandbox to `output` paths too: writes outside the root are DS3007, with an API test. (If M2-25 lands after this task, M2-25 does it.)
- [ ] (from planning, 2026-11-24) If M7-01 is done, `ExecContext::allow_remote` exists: leave it off for `ds ui` unless started with `--allow-remote`, with an API test that a URL source is rejected without it. (If M7-01 lands after this task, M7-01 does it.)

## Follow-ups
