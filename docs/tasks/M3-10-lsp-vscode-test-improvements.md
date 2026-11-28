# M3-10 — LSP and VS Code test improvements

**Status:** done — `runStatement()` checks trust and the server first; in-flight race, watcher, UTF-16 and Run-panic LSP tests; portable stdio test with stderr and clean-exit checks; VS Code E2E for stopped server, Restricted Mode, hostile `extraArgs` and `untitled:`; temp-dir cleanup; escape and contextual-word grammar tests.
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
- [x] `Runner::runStatement()` (`editors/vscode/src/run.ts`) calls `statementAt()` before checking
      Workspace Trust and that the client is running. With the server stopped, or in Restricted
      Mode, the user sees "Put the cursor in a query statement…" instead of the real reason. Check
      trust and the client first.

### In-process LSP (`crates/datascript-lsp/tests/lsp.rs`)
- [x] A slow engine gated on `Arc<tokio::sync::Notify>` for two deterministic in-flight tests:
      - a `didChange` during schema resolution supersedes the in-flight check, so neither its
        syntax-grace publish nor its final publish overwrites the newer version;
      - a `didClose` during a check suppresses late `publishDiagnostics` after the clear.
- [x] Running the same query twice publishes no new runtime diagnostics (the existing test's
      comment says so but never runs it twice).
- [x] `changed_data_files_invalidate_the_schema_cache` asserts the `.ds` watcher filter before
      sending `.csv` changes (today the second notification supersedes the first before anything
      is checked), and covers a deleted data file and invalidation across two open documents.
- [x] Surrogate pairs (`😀`) on the same line: published diagnostic ranges and one IDE request
      (`hover`).
- [x] `datascript/run` turns a panic into `DS9001` without taking the server down.

### Stdio parity (`crates/datascript-cli/tests/lsp_run.rs`)
- [x] Replace the Unix-only `uri_of` with `tower_lsp_server::ls_types::Uri::from_file_path` and
      drop `#[cfg_attr(windows, ignore)]`, so Windows CI runs it. `datascript-cli` does not depend
      on `tower-lsp-server` and `datascript-lsp` does not re-export `Uri`: add
      `tower-lsp-server.workspace = true` to `[dev-dependencies]` in
      `crates/datascript-cli/Cargo.toml`.
- [x] Pipe `ds lsp` stderr and assert it is empty; end with `shutdown` + `exit` and assert the
      process exits on its own.

### VS Code (`editors/vscode/test/`)
- [x] E2E: `datascript.runStatement` with the server stopped shows the server-stopped message, and
      in the untrusted workspace `runFile`/`runStatement` show the Restricted Mode warning (extend
      `captureMessages` for `showWarningMessage`). Assert the hostile `server.extraArgs` setting
      is ignored.
- [x] E2E: one `untitled:` document gets diagnostics.
- [x] Remove the `fs.mkdtempSync` directories in `runTests.ts` in a `finally` block.
- [x] Grammar: escape scopes (valid vs invalid, and verbatim `@"…"` not interpreting them) and the
      contextual words (`matches regex`, `nulls first`/`nulls last`).

### Out of scope
- Running Electron E2E on macOS/Windows CI.
- `client/registerCapability` fault injection and extension-host crash recovery.
- UTF-16 tests for every IDE request (conversion is shared; one is enough).

## Acceptance criteria
- [x] Every scope item passes under `cargo test --workspace` (including `lsp_run.rs` on Windows)
      and `npm test` in `editors/vscode`.
- [x] `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`,
      `npm run typecheck`, `npm run test:grammar` and `npm run test:e2e` pass.
- [x] Any unexpected bug a new test exposes gets its own task; if not fixed inline, the test is
      checked in with `#[ignore = "<task id>"]`.

## Follow-ups

- In-flight tests hold the first schema read on a `Notify` gate, so the old check is pending for
  exactly as long as the test wants. Whether its syntax-grace timer fires before or after the
  change/close arrives depends on the scheduler; the assertions accept a syntax-only publish
  only *before* the newer version (or the clear), so they never flake, and on a normal machine
  they exercise the suppression path (verified by removing the generation filter in
  `State::publish`: the change test fails).
- The watcher test was verified the same way: dropping the `.ds` filter makes it fail.
- `ds lsp` already exits on `exit` with standard input still open and writes nothing to
  standard error; no bug found.
- The new E2E cases were run against the old `run.ts` and fail there (they show the
  "Put the cursor…" info message instead of the real reason).
