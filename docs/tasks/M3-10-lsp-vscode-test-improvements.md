# M3-10 — LSP and VS Code test improvements

**Status:** todo
**Depends on:** M3-08, M3-09
**Crates:** `datascript-lsp`, `datascript-cli` (`tests/lsp_run.rs`), `editors/vscode`
**Diagnostic codes:** none new expected. If a test exposes a bug whose fix needs a code, write a
new task for the fix instead of allocating one here (D-018).

## Goal
Close the remaining integration and E2E test gaps found in the November 2026 audit of the language
server (`crates/datascript-lsp`), real-stdio CLI parity (`crates/datascript-cli/tests/lsp_run.rs`),
and VS Code extension (`editors/vscode`), and fix the verified `runStatement()` guard ordering bug
in `editors/vscode/src/run.ts`.

## Scope

### Verified extension UX fix
- [ ] Fix `Runner::runStatement()` in `editors/vscode/src/run.ts:121-138` so it checks Workspace
      Trust (`vscode.workspace.isTrusted`) and that the language client is running before calling
      `statementAt()` (`run.ts:130`). Today `statementAt()` returns `undefined` when the server is
      stopped (`run.ts:172-174`) or when invoked programmatically in Restricted Mode, showing
      `"Put the cursor in a query statement to run it..."` instead of the server-stopped or
      Restricted Mode message (`run.ts:195-207`).

### In-process LSP tests (`crates/datascript-lsp/tests/lsp.rs`)
- [ ] Add a minimal `Arc<tokio::sync::Notify>`-gated slow engine for two deterministic in-flight
      tests:
      - `didChange` arriving while a check is awaiting schema resolution supersedes the in-flight
        check so neither its syntax-grace publish (`crates/datascript-lsp/src/server.rs:580`) nor
        its final publish (`server.rs:585`) overwrites the newer version.
      - `didClose` arriving while a check is in flight suppresses late `publishDiagnostics` after
        the empty clear (`server.rs:315`).
- [ ] Verify repeat-run diagnostic deduplication (`server.rs:493`, `doc.runtime != runtime`):
      `runtime_errors_are_published_until_the_next_edit` (`lsp.rs:940`) states in a comment that
      running again publishes nothing new, but never runs the query a second time.
- [ ] Fix `changed_data_files_invalidate_the_schema_cache` (`lsp.rs:580-611`) so the `.ds` watcher
      filter (`server.rs:323-331`) is genuinely asserted before sending `.csv` changes (currently
      line 598 immediately supersedes any check scheduled by line 591), and cover deleted data
      files (`type: 3`) and multi-document invalidation (`server.rs:335-338`).
- [ ] Add same-line UTF-16 / surrogate-pair (`😀`) JSON-RPC tests for published diagnostics
      (`crates/datascript-lsp/src/convert.rs:83-96`), `textDocument/completion`,
      `textDocument/hover`, `textDocument/signatureHelp` (`crates/datascript-lsp/src/ide.rs:45-196`),
      and `textDocument/codeLens` (`crates/datascript-lsp/src/run.rs:171-207`).
- [ ] Cover `snippetSupport: false` capability negotiation (`server.rs:200-208`; `Client::start_full`
      at `lsp.rs:113` currently hardcodes `true`), unknown/closed URIs and out-of-bounds positions
      for `completion`, `hover`, and `signatureHelp` (`server.rs:354-414`), multi-document
      `base_dir` isolation across subdirectories (`server.rs:638-646`), and `datascript/run`
      panic-to-`DS9001` isolation (`server.rs:471-483`).

### Real-stdio LSP parity (`crates/datascript-cli/tests/lsp_run.rs`)
- [ ] Replace the hand-rolled Unix-only `uri_of` helper (`lsp_run.rs:112-126`) with
      `tower_lsp_server::ls_types::Uri::from_file_path` (matching `lsp.rs:353-358`, adding
      `tower-lsp-server.workspace = true` to `[dev-dependencies]` in
      `crates/datascript-cli/Cargo.toml` or exposing a helper from `datascript-lsp`) and remove
      `#[cfg_attr(windows, ignore)]` (`lsp_run.rs:153`) so Windows CI runs the stdio parity test.
- [ ] Pipe `ds lsp` standard error (instead of `Stdio::null()` at `lsp_run.rs:35`) and assert it is
      empty, verify clean `shutdown` + `exit` termination (instead of only killing the child in
      `Drop` at `lsp_run.rs:105-110`), and add a compact stdio smoke test covering `$/cancelRequest`
      and IDE requests (`completion`, `hover`, `signatureHelp`, `codeLens`).

### VS Code E2E and grammar tests (`editors/vscode/test/`)
- [ ] Add E2E tests in `editors/vscode/test/e2e/suite/trusted.test.ts` covering:
      - `vscode.executeCompletionItemProvider`, `vscode.executeHoverProvider`, and
        `vscode.executeSignatureHelpProvider`.
      - `untitled:` documents (`editors/vscode/src/extension.ts:111`).
      - `datascript.runStatement` when the server is stopped (verifying the fix in `run.ts:130`).
- [ ] Update `editors/vscode/test/e2e/suite/untrusted.test.ts` to assert that the hostile workspace
      `datascript.server.extraArgs` setting injected by `editors/vscode/test/e2e/runTests.ts:43` is
      ignored, and that `datascript.runFile` / `datascript.runStatement` show the Restricted Mode
      warning (`run.ts:196`; extend `captureMessages` in `editors/vscode/test/e2e/suite/helpers.ts`
      to support `showWarningMessage`).
- [ ] Clean up the temporary directories created by `fs.mkdtempSync` in
      `editors/vscode/test/e2e/runTests.ts:34-67` after each run (in a `finally` block).
- [ ] Tighten `editors/vscode/test/grammar/grammar.test.ts` with targeted scope assertions for:
      - `#escapes` (`editors/vscode/syntaxes/datascript.tmLanguage.json:184-195`: valid
        `constant.character.escape` vs invalid `invalid.illegal.escape`, and verbatim
        `@"..."`/`@'...'` at `tmLanguage.json:112-141` not interpreting escapes).
      - Contextual `#words` (`tmLanguage.json:372-409`: `matches regex`, `nulls first`/`nulls last`,
        and `entity.name.function`).

### Out of scope
- Running Electron E2E or standalone `npm run test:grammar` on macOS/Windows CI (`.github/workflows/ci.yml:51-77` keeps the `vscode` job on `ubuntu-latest`).
- Scripted `client/registerCapability` fault-injection tests (`server.rs:269-278`).
- Extension-host crash-recovery E2E tests.

## Acceptance criteria
- [ ] Every item in Scope is implemented and passing under `cargo test --workspace` (including
      `lsp_run.rs` on Windows) and `npm test` in `editors/vscode`.
- [ ] `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`,
      `npm run typecheck`, `npm run test:grammar`, and `npm run test:e2e` pass.
- [ ] Any unexpected bug exposed by a new test gets its own task file and index row; if not fixed
      inline, the test is checked in with `#[ignore = "<task id>"]`.

## Follow-ups
