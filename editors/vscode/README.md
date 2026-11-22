# DataScript for VS Code

Language support for [DataScript](https://github.com/loic-sharma/datascript), a pipe-based query
language for CSV, Parquet and JSON files.

- **Highlighting** for `.ds` files. It works at once, before the language server starts.
- **Diagnostics as you type** from the `ds` language server: syntax errors, unknown columns, type
  errors and warnings, each linked to its error code.
- **Snippets** for common operators (`from`, `where`, `summarize`, `join`, `render`, …).

## Requirements

The diagnostics come from `ds lsp`. Put `ds` on your `PATH`, or set `datascript.server.path` to
the executable. Without it, highlighting and snippets still work.

## Settings

| Setting | Default | |
|---|---|---|
| `datascript.server.path` | `ds` | The `ds` executable. A bare name is looked up on `PATH`; a relative path resolves against the first workspace folder. |
| `datascript.server.extraArgs` | `[]` | Extra arguments passed to `ds` before `lsp`. |
| `datascript.trace.server` | `off` | Logs the messages between VS Code and the server to the DataScript output channel. |

Run **DataScript: Restart Language Server** after replacing the `ds` binary.

## Workspace Trust

DataScript checks a query against the files it reads, so in a workspace you have not trusted it
does less:

- The server reports syntax errors only. It reads no data files, so unknown columns and type
  errors are not reported.
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

`test:e2e` downloads VS Code unless `VSCODE_EXECUTABLE_PATH` names one, and tests the `ds` in
`target/debug` unless `DS_PATH` names another. Build it first with `cargo build -p datascript-cli`.

The grammar test compares `syntaxes/datascript.tmLanguage.json` with the real lexer: the Rust test
`crates/datascript-syntax/tests/textmate_tokens.rs` writes the lexer's tokens for
`test/grammar/*.ds` to `test/grammar/tokens.json`, and fails if a token kind is missing from the
fixtures. After changing a fixture, rerun it with `DS_BLESS=1`.
