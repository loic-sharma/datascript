# DataScript

DataScript is an open-source, pipe-based query language for analyzing local data files, inspired by
[Kusto Query Language (KQL)](https://learn.microsoft.com/kusto/query/). It is written in Rust and runs
on [Apache DataFusion](https://datafusion.apache.org/) by default. It works on the command line and in the browser.

```ds
let orders = 'data/orders/*.parquet';

let topCustomers = orders
| summarize revenue = sum(amount) by customerId
| top 10 by revenue desc;

orders
| join kind = inner topCustomers on customerId
| summarize revenue = sum(amount) by customerId, month = startofmonth(timestamp)
| render timechart
```

> **Status: pre-alpha, design phase.** Nothing works yet. The documents below describe the plan.
> Work is tracked in [`docs/tasks/`](docs/tasks/README.md).

## Core principles

- **Pipes.** A query is a source followed by operators chained with `|`, read top to bottom.
- **Readable errors are a feature.** Every error says what went wrong, points at the exact span,
  and suggests a fix when one is likely. The CLI, the editor, and the web UI share one
  diagnostics core, so an error looks the same everywhere.
- **Visualization is in the language.** `| render timechart` is part of the query. Runtimes with a UI
  draw the chart. Runtimes without one (CLI, REPL) still show the data.
- **Pluggable engines.** The compiler emits an engine-neutral logical plan. DataFusion is the first
  backend. Others can be added, so DataScript can also serve as a front end for other query engines.
- **Local files first.** Query CSV, Parquet, and JSON straight from disk, DuckDB-style:
  `'logs/*.csv' | where level == "error"`.

## Ways to run it (planned)

| Surface | Command / package | Milestone |
|---|---|---|
| CLI compiler / checker | `ds check query.ds`, `ds run query.ds` | M1 |
| REPL | `ds` | M1 |
| Language server + VS Code extension | `ds lsp`, `editors/vscode` | M3 |
| Local web UI (like DuckDB UI) | `ds ui` → opens `http://127.0.0.1:…` | M4 |
| In-browser runtime (WASM) | `@datascript/wasm` | M5 |

## Documentation

| Document | Contents |
|---|---|
| [docs/ROADMAP.md](docs/ROADMAP.md) | Milestones and what "done" means for each |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | Crates, compiler pipeline, engine abstraction, render layer, WASM constraints |
| [docs/LANGUAGE.md](docs/LANGUAGE.md) | Draft language specification |
| [docs/DIAGNOSTICS.md](docs/DIAGNOSTICS.md) | Error-message principles, format, and error-code ranges |
| [docs/TESTING.md](docs/TESTING.md) | The `.dstest` golden-file format and test strategy |
| [docs/DECISIONS.md](docs/DECISIONS.md) | Design decisions made so far, and open questions |
| [docs/tasks/](docs/tasks/README.md) | Task breakdown: one file per unit of work |
| [CLAUDE.md](CLAUDE.md) | Working agreement for contributors and coding agents |

## Repository layout (target)

```
crates/
  datascript-diagnostics/   Diagnostic model + renderers (terminal, plain, JSON, LSP conversion)
  datascript-syntax/        Lexer, parser, lossless syntax tree, typed AST
  datascript-compiler/      Name resolution, types, function catalog, logical plan IR
  datascript-engine/        Engine + Catalog traits, result model, RenderSpec
  datascript-datafusion/    DataFusion backend
  datascript-cli/           `ds` binary: check, run, explain, fmt, REPL, lsp, ui
  datascript-lsp/           Language server (M3)
  datascript-server/        Local web UI server (M4)
  datascript-wasm/          wasm-bindgen bindings (M5)
editors/vscode/             VS Code extension (M3)
web/                        Web UI front end, shared by `ds ui` and the WASM playground (M4/M5)
tests/
  cases/                    .dstest golden files
  data/                     Shared fixture datasets
docs/
```

## Building

Requires a current stable Rust toolchain via [rustup](https://rustup.rs). `rust-toolchain.toml`
pins `stable` and installs `rustfmt`, `clippy`, and the `wasm32-unknown-unknown` target on first use.

```bash
cargo build --workspace
cargo test --workspace
cargo run -p datascript-cli -- --version
```

Before sending a change, run the same checks as CI:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo check --target wasm32-unknown-unknown -p datascript-diagnostics -p datascript-syntax -p datascript-compiler
```

The last command keeps the core crates buildable for the browser (see
[ARCHITECTURE.md](docs/ARCHITECTURE.md#wasm-constraints)).

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
