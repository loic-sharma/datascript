# M4-01 — Local web UI server (`ds ui`)

**Status:** done — `datascript-server` (`ds ui`) with token/Origin/Host auth, JSON/Arrow/streaming export/files/schema/WebSocket endpoints, `rust-embed` + Vite dev proxy, `notify` file watcher, D-016 sandbox fixes + `DS3007` activation, Q-007/D-062 (`DS4060`), and `docs/API.md`.
**Depends on:** M1-13, M2-13
**Crates:** `datascript-server`, `datascript-cli`, `datascript-compiler`, `datascript-diagnostics`
**Diagnostic codes:** DS3007 (planned, D-016), DS3065–DS3069 (sources), DS4060–DS4064 (Q-007's
"results truncated" info diagnostic) (D-018)

## Goal
`ds ui [dir]` starts an axum server on 127.0.0.1 with a random port, prints the URL and opens the
browser. Access requires a per-launch token in the URL; check the Origin header.

## Scope
- JSON API: `POST /api/check`, `POST /api/run` (Arrow IPC or JSON rows + RenderSpec + diagnostics),
  `GET /api/files` (under the root dir only, no path traversal),
  `GET /api/schema?source=`. Streaming or cancellation over WebSocket for long queries.
- Serve the built `web/` assets embedded in the binary (`rust-embed`), with a dev mode that proxies to Vite.
  `rust-embed` fails to build if its folder is missing, and `web/` does not exist until M4-02:
  commit a placeholder `web/dist/index.html` (or embed behind a feature) so the workspace builds.
- `POST /api/complete` and `POST /api/hover` are M4-04's, not this task's.

Design-review additions:
- D-016: every path a query resolves must canonicalize under the root directory, otherwise DS3007.
  `--allow-outside-root` disables this. Symlinks that leave the root count as outside.
- Decide Q-007 (result size limits) and record it in DECISIONS.md.
- One shared `Arc<Session>` serves concurrent requests (D-011).

## Acceptance criteria
- [x] API integration tests, including path traversal attempts and missing or wrong tokens.
- [x] The API contract is documented in `docs/` (shared with the WASM runtime adapter).
- [x] `/api/run` with `from '/etc/passwd' | take 1`, `from '../x.csv'` and a symlink pointing out of
      the root all fail with DS3007. (Without `from` the query fails parsing with DS0150 and never
      reaches the sandbox, D-036.)

## Carried over
- [x] (from M2-11) The streaming API exists: `Session::start` returns a `RunStream` (`Send`, owns its engine and context) whose `next_query()` yields a `QueryStream` per query, batches arriving as they are produced, behind the panic boundary (D-055). Use it for streaming and cancellation over WebSocket (dropping it cancels the query) instead of building another.
- [x] (from M1-07) `ExecContext::row_limit` is enforced by the session and sets `QueryResult::truncated` (D-043). Decide Q-007's wording (an info diagnostic takes a code from this task's block, DS4060–DS4064) and show it. Invalidate the session's schema cache when files under the root change, since failed lookups are cached too.
- [x] (from M1-08) The sandbox hook exists: with `ExecContext::sandbox` set, the DataFusion engine returns `SourceError::OutsideSandbox` (paths compared after resolving symlinks and `..`, globs per match) and the binder reports DS3007 with draft wording. Set the sandbox to the UI root, review the wording, add golden or API tests, and mark DS3007 active in `codes.rs` and DIAGNOSTICS.md.
- [x] (from M2-29) Fix two verified sandbox escapes in `check_sandbox` / `glob_files` (`crates/datascript-datafusion/src/sources.rs`) before relying on the hook: (1) a missing file under a symlinked directory falls back to lexical `normalize()`, so an intermediate symlink can leave the root; canonicalize the deepest existing ancestor, then append and normalize the rest. (2) A glob that escapes the root but matches nothing (`../*.nope`, `/outside/*.csv`) returns DS3004 instead of DS3007, because only matches are checked; check the glob's non-wildcard prefix against the sandbox first.
- [x] (from M2-29) Cover the sandbox with a `TempDir` integration suite against `Session` + `DataFusionEngine` (`#[cfg(unix)]` for symlinks): file and directory symlinks pointing out (existing and missing targets), escaping globs with zero, all-outside and mixed matches, a sibling-prefix directory (`root2/a.csv` against `root`), absolute paths inside and outside, and host tables registered inside vs outside the root.
- [x] (from planning, 2026-11-24) Q-007's "export the full result" should stream through `datascript_engine::writers`, the same writers the `output` operator uses (M2-25…M2-27, M2-28 for Excel), so the export offers every format they can write.
- [x] (from planning, 2026-11-24) If M2-25 is done, apply D-016's sandbox to `output` paths too: writes outside the root are DS3007, with an API test. (M2-25 is still `todo`; M2-25 owns this when it lands.)
- [x] (from planning, 2026-11-24) If M7-01 is done, `ExecContext::allow_remote` exists: leave it off for `ds ui` unless started with `--allow-remote`, with an API test that a URL source is rejected without it. (M7-01 is still `todo`; M7-01 owns this when it lands.)

## Follow-ups
- Informational: Q-007 closed as D-062 (10,000-row UI default, `DS4060`, full streaming `POST /api/export`). API contract lives in `docs/API.md`. `web/dist/index.html` is a placeholder replaced by M4-02.
