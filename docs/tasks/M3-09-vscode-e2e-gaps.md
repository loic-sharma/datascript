# M3-09 — VS Code extension E2E test gaps

**Status:** todo
**Depends on:** M3-05
**Crates:** none (`editors/vscode/test/e2e/`)
**Diagnostic codes:** none (tests only)

## Goal
A review of the integration tests (November 2026) found extension behaviour that the E2E suites
(`editors/vscode/test/e2e/suite/trusted.test.ts`, `untrusted.test.ts`) never exercise. Add E2E
tests for it.

## Scope

### Run
- [ ] The `datascript.run.maxRows` setting (read in `editors/vscode/src/run.ts`) cuts the output
  in the terminal.
- [ ] `datascript.runStatement` with the cursor on a `let` or a blank line runs nothing and shows
  the info message; with the cursor on line 2 or later of a multi-line query, it runs that query.
- [ ] Closing the `DataScript` terminal while a query runs cancels it (only Ctrl+C and running
  again are tested).
- [ ] A compile error (`DS1001`) and chart info (`DS5001`) show in the terminal output.

### Server
- [ ] Changing a `datascript.server` setting restarts the server (`extension.ts`,
  `affectsConfiguration('datascript.server')`).
- [ ] An invalid `datascript.server.path` goes through `reportStartFailure` and highlighting still
  works.
- [ ] Optional: granting Workspace Trust restarts the server from syntax-only to full check
  (`onDidGrantWorkspaceTrust`). If this cannot be automated, write the manual check under
  "Follow-ups".

## Acceptance criteria
- [ ] Every item above has a test, or a note under "Follow-ups" saying why it cannot.
- [ ] The new tests run in CI (`.github/workflows/ci.yml`) with the existing suites.
- [ ] Any wrong behaviour a new test exposes gets its own task file and index row.

## Follow-ups
