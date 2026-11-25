# M4-01 — Local web UI server (`ds ui`)

**Status:** todo
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
- [ ] API integration tests, including path traversal attempts and missing or wrong tokens.
- [ ] The API contract is documented in `docs/` (shared with the WASM runtime adapter).
- [ ] `/api/run` with `from '/etc/passwd' | take 1`, `from '../x.csv'` and a symlink pointing out of
      the root all fail with DS3007. (Without `from` the query fails parsing with DS0150 and never
      reaches the sandbox, D-036.)

## Carried over
- [ ] (from M2-11) The streaming API exists: `Session::start` returns a `RunStream` (`Send`, owns its engine and context) whose `next_query()` yields a `QueryStream` per query, batches arriving as they are produced, behind the panic boundary (D-055). Use it for streaming and cancellation over WebSocket (dropping it cancels the query) instead of building another.
- [ ] (from M1-07) `ExecContext::row_limit` is enforced by the session and sets `QueryResult::truncated` (D-043). Decide Q-007's wording (an info diagnostic takes a code from this task's block, DS4060–DS4064) and show it. Invalidate the session's schema cache when files under the root change, since failed lookups are cached too.
- [ ] (from M1-08) The sandbox hook exists: with `ExecContext::sandbox` set, the DataFusion engine returns `SourceError::OutsideSandbox` (paths compared after resolving symlinks and `..`, globs per match) and the binder reports DS3007 with draft wording. Set the sandbox to the UI root, review the wording, add golden or API tests, and mark DS3007 active in `codes.rs` and DIAGNOSTICS.md.
- [ ] (from planning, 2026-11-24) Q-007's "export the full result" should stream through `datascript_engine::writers`, the same writers the `output` operator uses (M2-26, M2-25 for Excel), so the export offers every format they can write.
- [ ] (from planning, 2026-11-24) If M2-26 is done, apply D-016's sandbox to `output` paths too: writes outside the root are DS3007, with an API test. (If M2-26 lands after this task, M2-26 does it.)
- [ ] (from planning, 2026-11-24) If M7-01 is done, `ExecContext::allow_remote` exists: leave it off for `ds ui` unless started with `--allow-remote`, with an API test that a URL source is rejected without it. (If M7-01 lands after this task, M7-01 does it.)

## Follow-ups
