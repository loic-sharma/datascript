# M3-04 — VS Code extension

**Status:** done — `editors/vscode`: `.ds` language, a TextMate grammar checked token by token against the lexer, snippets, and a client for `ds lsp` that starts it syntax-only (`initializationOptions.syntaxOnly`, new in `datascript-lsp`) in untrusted workspaces; extension tests in VS Code, trusted and untrusted; CI builds the .vsix (D-053).
**Depends on:** M3-01
**Crates:** `editors/vscode`

## Goal
A TypeScript extension: `.ds` language registration, a TextMate grammar (for highlighting before
the server starts), a language client that launches `ds lsp` (configurable path, or a bundled
binary later), and snippets.

## Acceptance criteria
- [x] `vsce package` builds a .vsix. An extension test opens a file and receives diagnostics.
      (`npm run package`, also in CI with the .vsix uploaded; `test/e2e/suite/trusted.test.ts`.)
- [x] Grammar covers every token kind of the lexer (tested against fixture files). Hyphenated
      operator names are matched as adjacent `ident-ident` after `|`, the same rule as the parser (D-026).
      (`datascript-syntax/tests/textmate_tokens.rs` fails if a kind is missing from
      `test/grammar/*.ds`; `test/grammar/grammar.test.ts` checks every token's scope and the
      operator-name rule, which like the parser joins only the known hyphenated names.)
- [x] Honors Workspace Trust: in an untrusted workspace, the server runs syntax-only (no schema reads, no Run). Tested.
      (`syntax_only_mode_reads_no_files` with an engine that panics on any read;
      `test/e2e/suite/untrusted.test.ts` in Restricted Mode. There is no Run yet; M3-05 carries it.)

## Carried over
- [x] (from M3-01) The server has no syntax-only mode yet. Add one to `datascript-lsp` (for example an `initializationOptions` flag that skips `Session::check` and publishes only `parse` diagnostics) and have the client set it in an untrusted workspace.
- [x] (from M3-01) The server clears its schema cache only when the client reports changed files through a dynamically registered `workspace/didChangeWatchedFiles` watcher (D-052). Make sure the language client advertises `dynamicRegistration` for it (vscode-languageclient does by default).

## Follow-ups
- Syntax-only is a startup flag; the client restarts the server when trust is granted. VS Code has
  no API to grant trust, so that restart is not tested end to end (D-053).
- The watcher registration is verified end to end: the trusted suite edits a CSV on disk and sees
  the diagnostics change.
- Workspace values of `datascript.server.path` / `extraArgs` are ignored when untrusted
  (`restrictedConfigurations`); the untrusted suite opens a folder that sets them.
- Handed over: Run in untrusted workspaces (M3-05), completion and hover in syntax-only mode
  (M3-02), semantic tokens and the TextMate scopes (M3-03), bundling `ds` in the .vsix (M6-02).
