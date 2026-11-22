# DataScript for VS Code

Language support for [DataScript](https://github.com/loic-sharma/datascript), a pipe-based query
language for CSV, Parquet and JSON files.

- **Highlighting** for `.ds` files. It works at once, before the language server starts.
- **Diagnostics as you type** from the `ds` language server: syntax errors, unknown columns, type
  errors and warnings, each linked to its error code.
- **Snippets** for common operators (`from`, `where`, `summarize`, `join`, `render`, …).
- **Run** a query from the "▶ Run" CodeLens above it (or "▶ Run file"), or with
  **DataScript: Run Statement at Cursor** and **DataScript: Run File**. The result prints in a
  terminal named "DataScript" exactly as `ds run` prints it, unsaved edits included. Runtime
  errors also show in the editor until the next edit. Press Ctrl+C in the terminal to stop a run.

  Relative paths resolve against the script's folder. An untitled script resolves them against
  the workspace folder, or your home directory when no folder is open. Charts print as tables for
  now, as on the command line.

## Requirements

The diagnostics come from `ds lsp`. Put `ds` on your `PATH`, or set `datascript.server.path` to
the executable. Without it, highlighting and snippets still work.

## Settings

| Setting | Default | |
|---|---|---|
| `datascript.server.path` | `ds` | The `ds` executable. A bare name is looked up on `PATH`; a relative path resolves against the first workspace folder. |
| `datascript.server.extraArgs` | `[]` | Extra arguments passed to `ds` before `lsp`. |
| `datascript.run.maxRows` | `100` | Rows shown per table when running, like `ds run --max-rows`. 0 shows every row. |
| `datascript.trace.server` | `off` | Logs the messages between VS Code and the server to the DataScript output channel. |

Run **DataScript: Restart Language Server** after replacing the `ds` binary.

## Workspace Trust

DataScript checks a query against the files it reads, so in a workspace you have not trusted it
does less:

- The server reports syntax errors only. It reads no data files, so unknown columns and type
  errors are not reported.
- Nothing runs: there is no Run CodeLens, and the Run commands are off.
- `datascript.server.path` and `datascript.server.extraArgs` are read from your user settings
  only. An untrusted folder cannot choose the program VS Code runs.

When you trust the workspace, the server restarts with the full check.

## Development

```bash
npm ci
npm run build          # bundle src/ into dist/extension.js
npm run test:grammar   # the grammar against the lexer's tokens (no VS Code needed)
npm run test:e2e       # the extension in VS Code, trusted and untrusted (needs target/debug/ds)
npm run package        # build datascript-<version>.vsix
```

### Trying it locally

Build the server and the extension, then open a VS Code window that loads the extension from this
folder:

```bash
cargo build -p datascript-cli          # from the repository root
npm ci && npm run build                # from editors/vscode
code --extensionDevelopmentPath="$PWD" ../../examples
```

In that window, set `datascript.server.path` to the absolute path of `target/debug/ds` and open a
`.ds` file. After a change:

- **Extension code:** `npm run build` (or keep `npm run watch` running), then **Developer: Reload
  Window**.
- **Grammar, snippets or `package.json`:** **Developer: Reload Window**.
- **Server:** `cargo build -p datascript-cli`, then **DataScript: Restart Language Server**.

Set `datascript.trace.server` to `verbose` to see the LSP messages in the DataScript output
channel. To try Restricted Mode, run **Workspaces: Manage Workspace Trust** and don't trust the
folder. To try the packaged extension instead, run `npm run package` and
`code --install-extension datascript-0.0.1.vsix`.

### Tests

`test:e2e` downloads VS Code unless `VSCODE_EXECUTABLE_PATH` names one, and tests the `ds` in
`target/debug` unless `DS_PATH` names another. Build it first with `cargo build -p datascript-cli`.

The grammar test compares `syntaxes/datascript.tmLanguage.json` with the real lexer: the Rust test
`crates/datascript-syntax/tests/textmate_tokens.rs` writes the lexer's tokens for
`test/grammar/*.ds` to `test/grammar/tokens.json`, and fails if a token kind is missing from the
fixtures. After changing a fixture, rerun it with `DS_BLESS=1`.
