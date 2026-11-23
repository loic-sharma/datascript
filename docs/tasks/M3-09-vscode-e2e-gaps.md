# M3-09 — VS Code extension E2E test gaps

**Status:** done — Added 6 VS Code E2E tests in `editors/vscode/test/e2e/suite/trusted.test.ts` covering `datascript.run.maxRows`, `datascript.runStatement` cursor placement, terminal-close cancellation, `DS1001`/`DS5001` terminal output, server restart on `datascript.server` setting changes, and `reportStartFailure` with an invalid server path; documented the manual Workspace Trust grant check.
**Depends on:** M3-05
**Crates:** none (`editors/vscode/test/e2e/`)
**Diagnostic codes:** none (tests only)

## Goal
A review of the integration tests (November 2026) found extension behaviour that the E2E suites
(`editors/vscode/test/e2e/suite/trusted.test.ts`, `untrusted.test.ts`) never exercise. Add E2E
tests for it.

## Scope

### Run
- [x] The `datascript.run.maxRows` setting (read in `editors/vscode/src/run.ts`) cuts the output
  in the terminal.
- [x] `datascript.runStatement` with the cursor on a `let` or a blank line runs nothing and shows
  the info message; with the cursor on line 2 or later of a multi-line query, it runs that query.
- [x] Closing the `DataScript` terminal while a query runs cancels it (only Ctrl+C and running
  again are tested).
- [x] A compile error (`DS1001`) and chart info (`DS5001`) show in the terminal output.

### Server
- [x] Changing a `datascript.server` setting restarts the server (`extension.ts`,
  `affectsConfiguration('datascript.server')`).
- [x] An invalid `datascript.server.path` goes through `reportStartFailure` and highlighting still
  works.
- [x] Optional: granting Workspace Trust restarts the server from syntax-only to full check
  (`onDidGrantWorkspaceTrust`). If this cannot be automated, write the manual check under
  "Follow-ups". *(Manual check written under "Follow-ups".)*

## Acceptance criteria
- [x] Every item above has a test, or a note under "Follow-ups" saying why it cannot.
- [x] The new tests run in CI (`.github/workflows/ci.yml`) with the existing suites.
- [x] Any wrong behaviour a new test exposes gets its own task file and index row.

## Follow-ups
- **Granting Workspace Trust (`onDidGrantWorkspaceTrust`) cannot be automated:** VS Code exposes
  no API or non-interactive command to grant Workspace Trust (by design, only the user can grant
  trust; `workbench.trust.manage` and `workbench.trust.configure` only open the interactive Trust
  editor, as also noted in M3-04 / D-053). Manual check:
  1. Open a folder containing `orders.csv` and a `.ds` file with a schema error (e.g.
     `from 'orders.csv' | project amout`) in Restricted Mode (untrusted).
  2. Confirm no `DS1001` diagnostic appears and no `▶ Run` CodeLens is shown (syntax-only mode).
  3. Run **Workspaces: Manage Workspace Trust** and click **Trust**.
  4. Confirm the `DataScript` output channel logs
     `The workspace is now trusted: restarting with the full check.`, `DS1001` appears on `amout`,
     and the `▶ Run` CodeLens appears above the query.
- None of the new tests exposed wrong extension behaviour, so no bug tasks were created.
