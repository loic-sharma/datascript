# M4-01 — Local web UI server (`ds ui`)

**Status:** todo
**Depends on:** M4-08
**Crates:** `datascript-server`, `datascript-cli`, `datascript-engine`, `datascript-diagnostics`
**Diagnostic codes:** DS3007 (planned, D-016), DS3065–DS3069 (sources), DS4060–DS4064 (`DS4060`
results truncated per D-064; `DS4061` upstream cell has errors per D-062) (D-018)

## Goal
`ds ui [dir]` starts an axum server on 127.0.0.1 with a random port, prints the URL and opens the
browser. Access requires a per-launch token in the URL and `Origin` checks on API/WS requests.
Implements the M4 v1 server endpoints specified in
[WEB_UI.md §6](../WEB_UI.md#server-api-contract-datascript-server-m4-01-m4-04--m4-05) (file/folder
create-dir, rename, and delete land in M4-05).

## Scope
- **Streaming row-limit helper** (`datascript-engine`, D-064):
  - `ExecContext::row_limit` is currently enforced only inside `ResultStream::collect` /
    `QueryStream::collect`. Add a reusable streaming-limit helper in `datascript-engine` that counts
    rows as batches arrive, slices the final batch at `row_limit`, drops the `QueryStream` on the
    next row to cancel execution, and reports `truncated = true` (shared by `datascript-server`,
    M3-06, and M5).
- **M4 v1 workspace files API** (all confined to root per D-016; safety rules in
  [WEB_UI.md §4.1](../WEB_UI.md#41-left-pane--working-directory-tree--attached-tables)):
  - `GET /api/info`, `GET /api/files?dir=&includeHidden=`, `GET /api/schema?source=`.
  - `GET /api/file`, `POST /api/file` (`O_CREAT|O_EXCL`), `PUT /api/file` (`expectedVersion` `409`
    conflict check, diffing against on-disk `.dsnb` via M4-08's serializer to preserve untouched
    cells/separators). Refuse non-`.dsnb`/`.ds` paths and any path with a symlink component under
    the root (`symlink_metadata`/`O_NOFOLLOW`).
- **Whole-notebook check, streaming run, `DS4061` & ticketed export**:
  - `POST /api/check`: checks the entire notebook or `.ds` script in one pass via M4-08's
    `StitchedProgram`, returning diagnostics grouped by `cellIndex` (UTF-16, D-025) and per-cell
    in-scope `let` summaries.
  - `WS /api/ws`: streaming query execution (`Session::start` + streaming row-limit helper) and
    workspace file-watch invalidations. Emits `started` with `nowMicros` (accepting an optional
    shared `nowMicros` for Run all), `rowLimit`, and `sourceCount` (count of distinct `SourceRef`s
    computed by walking `Plan::Scan` nodes in `QueryPlan::root`), `DS4060` on truncation, and
    `DS4061` per the exact trigger in [WEB_UI.md §3](../WEB_UI.md#upstream-error-handling-ds4061).
  - Two-step zero-buffering export (`POST /api/export` → one-shot `GET /api/export/<ticket>`,
    [WEB_UI.md §3](../WEB_UI.md#result-size-limits--streaming-export-d-064-closes-q-007),
    [§6](../WEB_UI.md#server-api-contract-datascript-server-m4-01-m4-04--m4-05)).
- Embed built `web/` assets (`rust-embed`, with placeholder `web/dist/index.html` until M4-02) plus
  a dev proxy to Vite. One shared `Arc<Session>` serves concurrent requests (D-011).
- `POST /api/complete`, `/api/hover`, and `/api/signature-help` are M4-04's.

## Acceptance criteria
- [ ] API integration tests covering:
  - auth (missing/wrong token, bad `Origin` on API/WS, one-shot ticket replay/expiry and
    `Sec-Fetch-Site` on `GET /api/export/<ticket>`);
  - path traversal and workspace safety: `.csv`/`.parquet` write rejection, in-root symlink
    rejection (`notes.ds -> data/orders.csv`), atomic `O_CREAT|O_EXCL` create refusal on existing
    files, `409` conflicts, and lossless `PUT` diffing;
  - whole-notebook `POST /api/check`, single-cell `WS` run + cancel + `sourceCount`/`rowLimit` +
    `DS4060` truncation (default & custom `row_limit`), and `DS4061` trigger rules (fires on
    upstream syntax error or prepended `let` error; does *not* fire on upstream query error or
    warning);
  - two-step streaming export reusing the run-time cell snapshot, `nowMicros` (`now()`/`ago()`),
    and cell-relative `statementIndex` on a multi-statement cell with local & upstream `let`s.
- [ ] Wire contract documented alongside [WEB_UI.md](../WEB_UI.md) (shared with M5 WASM).
- [ ] Running `from '/etc/passwd' | take 1`, `from '../x.csv'` and a symlink pointing out of the
      root all fail with DS3007 (D-016, D-036).

## Carried over
- [ ] (from M2-11) The streaming API exists: `Session::start` returns a `RunStream` (`Send`, owns its engine and context) whose `next_query()` yields a `QueryStream` per query, batches arriving as they are produced, behind the panic boundary (D-055). Use it for streaming and cancellation over WebSocket (dropping it cancels the query) instead of building another.
- [ ] (from M1-07) `ExecContext::row_limit` is enforced by `collect` and sets `QueryResult::truncated` (D-043). Extend it with the streaming-limit helper above, implement D-064's `DS4060` truncation info diagnostic and D-062's `DS4061` upstream-cell error (from this task's block, DS4060–DS4064), and show them. Invalidate the session's schema cache when files under the root change, since failed lookups are cached too.
- [ ] (from M1-08) The sandbox hook exists: with `ExecContext::sandbox` set, the DataFusion engine returns `SourceError::OutsideSandbox` (paths compared after resolving symlinks and `..`, globs per match) and the binder reports DS3007 with draft wording. Set the sandbox to the UI root, review the wording, add golden or API tests, and mark DS3007 active in `codes.rs` and DIAGNOSTICS.md.
- [ ] (from M2-29) Fix two verified sandbox escapes in `check_sandbox` / `glob_files` (`crates/datascript-datafusion/src/sources.rs`) before relying on the hook: (1) a missing file under a symlinked directory falls back to lexical `normalize()`, so an intermediate symlink can leave the root; canonicalize the deepest existing ancestor, then append and normalize the rest. (2) A glob that escapes the root but matches nothing (`../*.nope`, `/outside/*.csv`) returns DS3004 instead of DS3007, because only matches are checked; check the glob's non-wildcard prefix against the sandbox first.
- [ ] (from M2-29) Cover the sandbox with a `TempDir` integration suite against `Session` + `DataFusionEngine` (`#[cfg(unix)]` for symlinks): file and directory symlinks pointing out (existing and missing targets), escaping globs with zero, all-outside and mixed matches, a sibling-prefix directory (`root2/a.csv` against `root`), absolute paths inside and outside, and host tables registered inside vs outside the root.
- [ ] (from planning, 2026-11-24) D-064's "export the full result" should stream through `datascript_engine::writers`, the same writers the `output` operator uses (M2-25…M2-27, M2-28 for Excel), so the export offers every format they can write.
- [ ] (from planning, 2026-11-24) If M2-25 is done, apply D-016's sandbox to `output` paths too: writes outside the root are DS3007, with an API test. (If M2-25 lands after this task, M2-25 does it.)
- [ ] (from planning, 2026-11-24) If M7-01 is done, `ExecContext::allow_remote` exists: leave it off for `ds ui` unless started with `--allow-remote`, with an API test that a URL source is rejected without it. (If M7-01 lands after this task, M7-01 does it.)

## Follow-ups
