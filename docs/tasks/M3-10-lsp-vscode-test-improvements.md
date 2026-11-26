# M3-10 — LSP and VS Code test improvements

**Status:** todo
**Depends on:** M3-08, M3-09
**Crates:** `datascript-lsp`, `datascript-cli` (`tests/lsp_run.rs`), `editors/vscode`
**Diagnostic codes:** none. If a test exposes a bug whose fix needs a code, write a new task for
the fix instead of allocating one here (D-018).

## Goal
Close the integration gaps found in the November 2026 audit of the language server, the stdio
parity test and the VS Code extension: in-flight races, a watcher test that doesn't assert what it
claims, Windows coverage, and a guard-order bug in `runStatement()`.

## Scope

### Extension fix
- [ ] `Runner::runStatement()` (`editors/vscode/src/run.ts`) calls `statementAt()` before checking
      Workspace Trust and that the client is running. With the server stopped, or in Restricted
      Mode, the user sees "Put the cursor in a query statement…" instead of the real reason. Check
      trust and the client first.

### In-process LSP (`crates/datascript-lsp/tests/lsp.rs`)
- [ ] A slow engine gated on `Arc<tokio::sync::Notify>` for two deterministic in-flight tests:
      - a `didChange` during schema resolution supersedes the in-flight check, so neither its
        syntax-grace publish nor its final publish overwrites the newer version;
      - a `didClose` during a check suppresses late `publishDiagnostics` after the clear.
- [ ] Running the same query twice publishes no new runtime diagnostics (the existing test's
      comment says so but never runs it twice).
- [ ] `changed_data_files_invalidate_the_schema_cache` asserts the `.ds` watcher filter before
      sending `.csv` changes (today the second notification supersedes the first before anything
      is checked), and covers a deleted data file and invalidation across two open documents.
- [ ] Surrogate pairs (`😀`) on the same line: published diagnostic ranges and one IDE request
      (`hover`).
- [ ] `datascript/run` turns a panic into `DS9001` without taking the server down.

### Stdio parity (`crates/datascript-cli/tests/lsp_run.rs`)
- [ ] Replace the Unix-only `uri_of` with `Uri::from_file_path` and drop
      `#[cfg_attr(windows, ignore)]`, so Windows CI runs it.
- [ ] Pipe `ds lsp` stderr and assert it is empty; end with `shutdown` + `exit` and assert the
      process exits on its own.

### VS Code (`editors/vscode/test/`)
- [ ] E2E: `datascript.runStatement` with the server stopped shows the server-stopped message, and
      in the untrusted workspace `runFile`/`runStatement` show the Restricted Mode warning (extend
      `captureMessages` for `showWarningMessage`). Assert the hostile `server.extraArgs` setting
      is ignored.
- [ ] E2E: one `untitled:` document gets diagnostics.
- [ ] Remove the `fs.mkdtempSync` directories in `runTests.ts` in a `finally` block.
- [ ] Grammar: escape scopes (valid vs invalid, and verbatim `@"…"` not interpreting them) and the
      contextual words (`matches regex`, `nulls first`/`nulls last`).

### Out of scope
- Running Electron E2E on macOS/Windows CI.
- `client/registerCapability` fault injection and extension-host crash recovery.
- UTF-16 tests for every IDE request (conversion is shared; one is enough).

## Acceptance criteria
- [ ] Every scope item passes under `cargo test --workspace` (including `lsp_run.rs` on Windows)
      and `npm test` in `editors/vscode`.
- [ ] `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`,
      `npm run typecheck`, `npm run test:grammar` and `npm run test:e2e` pass.
- [ ] Any unexpected bug a new test exposes gets its own task; if not fixed inline, the test is
      checked in with `#[ignore = "<task id>"]`.

## Follow-ups
