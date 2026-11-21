# M4-02 — Web UI front end

**Status:** todo
**Depends on:** M4-01
**Crates:** `web/`

## Goal
The SPA shared by `ds ui` and the WASM playground. Decide Q-002 (stack) and Q-004 (multiple
results) first and record them in DECISIONS.md.

## Scope
- A `Runtime` TS interface with a `ServerRuntime` implementation (`WasmRuntime` comes in M5-03).
- Editor with DataScript highlighting, inline diagnostics (same text as the CLI, rendered from
  JSON), Run (Ctrl/Cmd+Enter runs the statement at the cursor), and a virtualized results grid.
- File sidebar listing data files in the root. Clicking a file inserts it as a source.

## Acceptance criteria
- [ ] Vitest unit tests and a Playwright smoke test against a real `ds ui`.
- [ ] Dark/light themes. Keyboard accessible.

## Follow-ups
