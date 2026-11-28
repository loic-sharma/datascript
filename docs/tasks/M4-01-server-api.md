# M4-01 — Local web UI server (`ds ui`)

**Status:** todo
**Depends on:** M1-13, M2-13
**Crates:** `datascript-server`, `datascript-cli`, `datascript-compiler`, `datascript-diagnostics`
**Diagnostic codes:** DS3007 (planned, D-016), DS3065–DS3069 (sources), DS4060–DS4064 (D-063's
"results truncated" info diagnostic, using DS4060) (D-018)

## Goal
`ds ui [dir]` starts an axum server on 127.0.0.1 with a random port, prints the URL and opens the
browser. Access requires a per-launch token in the URL; check the `Origin` header. Implements the
server API specified in [WEB_UI.md §6](../WEB_UI.md#server-api-contract-datascript-server-m4-01--m4-04).

## Scope
- **Server info & workspace files API** (all confined to root per D-016):
  - `GET /api/info`: root folder name, capabilities, default `row_limit` (`10_000`, D-063),
    registered `--table` host tables.
  - `GET /api/files?dir=&includeHidden=`: full working-directory tree (hiding `.git`,
    `node_modules`, `target`, dotfiles unless `includeHidden=true`; no path traversal).
  - `GET /api/file?path=`, `PUT /api/file`, `POST /api/file`, `PATCH /api/file`, `DELETE /api/file`:
    read/write/create/rename/delete `.dsnb` notebooks, `.ds` scripts, and folders. Writes carry an
    optional `expectedVersion` (mtime + hash) and return `409 Conflict` if changed on disk (D-061).
    Refuse mutating or deleting data files (any file other than `.dsnb`, `.ds`, or folders).
  - `GET /api/schema?source=`: inspect a file or host table schema via `Session`.
- **Notebook-aware check, streaming run, cancellation & export**:
  - Combined-program cell stitcher (modeled after `ReplProgram` in `datascript-cli`) that prepends
    upstream `let` statements (`0..targetCellIndex`) ahead of the target cell and maps UTF-16 spans
    (D-025) back to `{ cellIndex, start, end, line, column }` (D-061). Relative paths resolve
    against the `.dsnb`/`.ds` file's parent directory (or root for Scratchpads), inside the D-016
    sandbox.
  - `POST /api/check`: checks a script or notebook cell with upstream `let`s in scope; returns
    per-cell UTF-16 JSON diagnostics and active `let` scope summaries.
  - `WS /api/ws`: multiplexed WebSocket for (1) streaming query runs (`run` / `cancel` using
    `Session::start` → Arrow IPC batches + `RenderSpec` + diagnostics + timing, with default
    `row_limit = 10_000` and `DS4060` truncation info per D-063; supports running a whole cell or a
    single statement index) and (2) filesystem watch events (updating the tree and invalidating
    `Session`'s schema cache).
  - `POST /api/export`: streams the full uncapped result via `datascript_engine::writers` (`csv`,
    `json`, `ndjson`, `parquet`, `arrow`).
- Serve the built `web/` assets embedded in the binary (`rust-embed`), with a dev mode that proxies
  to Vite. `rust-embed` fails to build if its folder is missing, and `web/` does not exist until
  M4-02: commit a placeholder `web/dist/index.html` (or embed behind a feature) so the workspace
  builds.
- `POST /api/complete`, `POST /api/hover`, and `POST /api/signature-help` are M4-04's, not this
  task's.

Design-review additions:
- D-016: every path a query resolves must canonicalize under the root directory, otherwise DS3007.
  `--allow-outside-root` disables this. Symlinks that leave the root count as outside.
- D-063 (closed Q-007): default UI `row_limit` is `10_000`; emit `DS4060` info diagnostic when
  truncated and stream full results on `/api/export`.
- One shared `Arc<Session>` serves concurrent requests (D-011).

## Acceptance criteria
- [ ] API integration tests covering auth (missing/wrong token, bad `Origin`), path traversal,
      workspace CRUD (including read-only rejection on `.csv`/`.parquet` writes/deletes and `409`
      version conflicts), cross-cell `let` checking/execution with mapped spans, WebSocket
      streaming + cancellation, `DS4060` truncation, and streaming `/api/export`.
- [ ] The exact JSON/WebSocket wire contract is documented alongside [WEB_UI.md](../WEB_UI.md)
      (shared with the M5 WASM runtime adapter).
- [ ] Running `from '/etc/passwd' | take 1`, `from '../x.csv'` and a symlink pointing out of the
      root all fail with DS3007. (Without `from` the query fails parsing with DS0150 and never
      reaches the sandbox, D-036.)

## Carried over
- [ ] (from M2-11) The streaming API exists: `Session::start` returns a `RunStream` (`Send`, owns its engine and context) whose `next_query()` yields a `QueryStream` per query, batches arriving as they are produced, behind the panic boundary (D-055). Use it for streaming and cancellation over WebSocket (dropping it cancels the query) instead of building another.
- [ ] (from M1-07) `ExecContext::row_limit` is enforced by the session and sets `QueryResult::truncated` (D-043). Implement D-063's `DS4060` truncation info diagnostic (from this task's block, DS4060–DS4064) and show it. Invalidate the session's schema cache when files under the root change, since failed lookups are cached too.
- [ ] (from M1-08) The sandbox hook exists: with `ExecContext::sandbox` set, the DataFusion engine returns `SourceError::OutsideSandbox` (paths compared after resolving symlinks and `..`, globs per match) and the binder reports DS3007 with draft wording. Set the sandbox to the UI root, review the wording, add golden or API tests, and mark DS3007 active in `codes.rs` and DIAGNOSTICS.md.
- [ ] (from M2-29) Fix two verified sandbox escapes in `check_sandbox` / `glob_files` (`crates/datascript-datafusion/src/sources.rs`) before relying on the hook: (1) a missing file under a symlinked directory falls back to lexical `normalize()`, so an intermediate symlink can leave the root; canonicalize the deepest existing ancestor, then append and normalize the rest. (2) A glob that escapes the root but matches nothing (`../*.nope`, `/outside/*.csv`) returns DS3004 instead of DS3007, because only matches are checked; check the glob's non-wildcard prefix against the sandbox first.
- [ ] (from M2-29) Cover the sandbox with a `TempDir` integration suite against `Session` + `DataFusionEngine` (`#[cfg(unix)]` for symlinks): file and directory symlinks pointing out (existing and missing targets), escaping globs with zero, all-outside and mixed matches, a sibling-prefix directory (`root2/a.csv` against `root`), absolute paths inside and outside, and host tables registered inside vs outside the root.
- [ ] (from planning, 2026-11-24) D-063's "export the full result" should stream through `datascript_engine::writers`, the same writers the `output` operator uses (M2-25…M2-27, M2-28 for Excel), so the export offers every format they can write.
- [ ] (from planning, 2026-11-24) If M2-25 is done, apply D-016's sandbox to `output` paths too: writes outside the root are DS3007, with an API test. (If M2-25 lands after this task, M2-25 does it.)
- [ ] (from planning, 2026-11-24) If M7-01 is done, `ExecContext::allow_remote` exists: leave it off for `ds ui` unless started with `--allow-remote`, with an API test that a URL source is rejected without it. (If M7-01 lands after this task, M7-01 does it.)

## Follow-ups
