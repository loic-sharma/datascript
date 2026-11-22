# M3-05 — VS Code: Run CodeLens with CLI-style output

**Status:** todo
**Depends on:** M3-04
**Crates:** `datascript-lsp`, `editors/vscode`

## Goal
A grey "▶ Run" CodeLens above each query statement (like Flutter's run button). Clicking it runs
the statement, with the `let`s it needs, as it is in the editor (unsaved edits included), and
prints the result in a "DataScript" terminal exactly as `ds run` would. A results panel with a
grid and charts comes later (M3-06) and reuses the same request (D-054).

## Design
- **The server runs what the editor shows.** A custom request `datascript/run` with params
  `{ textDocument: { uri }, statement?: number }` runs the server's current text of an open
  document, so unsaved edits need no extra work. `statement` is 1-based, like
  `ds run --statement`; without it the whole file runs. The result is
  `{ output: string }`: the text `ds run --format table` prints for that run, made by the same
  code (`datascript_engine::table`, the CLI's diagnostic renderer), including the info line for a
  `render` the terminal cannot draw. Pass the row limit (`maxRows`, default 100 like the CLI) and
  whether to color the output.
- **CodeLens comes from the server** (`textDocument/codeLens`), since it already parses the
  document: one "▶ Run" above each query statement, plus "▶ Run file" above the first statement
  when there are several. `let`-only statements get none.
- **Runtime errors** (DS3xxx, DS4xxx) are published with `publishDiagnostics` next to the check
  diagnostics of that document, and printed in the terminal as the CLI prints them. The next
  check after an edit clears them.
- **Output** goes to one reused terminal named "DataScript", backed by a `Pseudoterminal`: a
  header line naming the file and statement, the output, and the time taken.
- **Cancellation**: the client cancels the request (`$/cancelRequest`) when the user presses
  Ctrl+C in that terminal, closes it, or runs the same document again. The server must stop the
  query, not just drop the reply.
- **Untitled documents**: relative paths resolve against the first workspace folder. Decide and
  test what happens with no folder open.
- **No new diagnostic codes are expected.** If one is needed, reserve a block first (D-018).

## Acceptance criteria
- [ ] Run a single statement (with the `let`s it depends on) or the whole file, from the CodeLens
      and from a command (`DataScript: Run Statement at Cursor`, `DataScript: Run File`).
- [ ] Unsaved edits are run: an edit that is not saved changes the output.
- [ ] For the same file, the output equals what `ds run` prints (test in
      `crates/datascript-lsp/tests/lsp.rs` against the CLI's renderer).
- [ ] Runtime errors appear as diagnostics in the editor and in the terminal.
- [ ] Cancellation works, and a cancelled query stops running.
- [ ] Extension tests in `editors/vscode/test/e2e/suite/` cover the CodeLens, a run with unsaved
      edits, and a runtime error. Expose the last output through `DataScriptApi` so tests can
      read it.

## Carried over
- [ ] (from M2-11) `ds run` now prints by streaming: `stream_results` in `crates/datascript-cli/src/output.rs` reads a `RunStream` (`Session::start`) and writes to a `Sink`, which can wrap any `Write` (a `Vec<u8>` for the LSP). Cut tables say `… at least N more rows` when the rest was not counted (D-055). To return the same text from `datascript/run`, move that printer to a crate the LSP can depend on rather than copying it.
- [ ] (from M3-04) No Run in an untrusted workspace (ARCHITECTURE.md § Frontends, D-053): show no Run CodeLens when `vscode.workspace.isTrusted` is false, and have the server reject `datascript/run` when it was started with `initializationOptions.syntaxOnly`. Add both to `editors/vscode/test/e2e/suite/untrusted.test.ts` and `crates/datascript-lsp/tests/lsp.rs`. A syntax-only server returns no CodeLens either.

## Follow-ups
