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

## Acceptance criteria
- [ ] API integration tests, including path traversal attempts and missing or wrong tokens.
- [ ] The API contract is documented in `docs/` (shared with the WASM runtime adapter).

## Follow-ups
