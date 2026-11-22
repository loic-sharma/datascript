# M4-01 — Local web UI server (`ds ui`)

**Status:** todo
**Depends on:** M1-13
**Crates:** `datascript-server`, `datascript-cli`

## Goal
`ds ui [dir]` starts an axum server on 127.0.0.1 with a random port, prints the URL and opens the
browser. Access requires a per-launch token in the URL; check the Origin header.

## Scope
- JSON API: `POST /api/check`, `POST /api/run` (Arrow IPC or JSON rows + RenderSpec + diagnostics),
  `POST /api/complete`, `GET /api/files` (under the root dir only, no path traversal),
  `GET /api/schema?source=`. Streaming or cancellation over WebSocket for long queries.
- Serve the built `web/` assets embedded in the binary (`rust-embed`), with a dev mode that proxies to Vite.

Design-review additions:
- D-016: every path a query resolves must canonicalize under the root directory, otherwise DS3007.
  `--allow-outside-root` disables this. Symlinks that leave the root count as outside.
- Decide Q-007 (result size limits) and record it in DECISIONS.md.
- One shared `Arc<Session>` serves concurrent requests (D-011).

## Acceptance criteria
- [ ] API integration tests, including path traversal attempts and missing or wrong tokens.
- [ ] The API contract is documented in `docs/` (shared with the WASM runtime adapter).
- [ ] `/api/run` with `'/etc/passwd' | take 1`, `'../x.csv'` and a symlink pointing out of the root all fail with DS3007.

## Carried over
- [ ] (from planning, 2026-11-22) M2-11 also needs a streaming variant of `Session::run`. Whichever task comes first builds it in `datascript-engine` for both; the other one reuses it.
- [ ] (from M1-07) `ExecContext::row_limit` is enforced by the session and sets `QueryResult::truncated` (D-043). Decide Q-007's wording (an info diagnostic needs a code from this task's block) and show it. Invalidate the session's schema cache when files under the root change, since failed lookups are cached too.
- [ ] (from M1-08) The sandbox hook exists: with `ExecContext::sandbox` set, the DataFusion engine returns `SourceError::OutsideSandbox` (paths compared after resolving symlinks and `..`, globs per match) and the binder reports DS3007 with draft wording. Set the sandbox to the UI root, review the wording, add golden or API tests, and mark DS3007 active in `codes.rs` and DIAGNOSTICS.md.

## Follow-ups
