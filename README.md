# DataScript

DataScript is an open-source, pipe-based query language for analyzing local data files, inspired by
[Kusto Query Language (KQL)](https://learn.microsoft.com/kusto/query/). It is written in Rust and runs
on [Apache DataFusion](https://datafusion.apache.org/) by default. It works on the command line and in the browser.

```ds
let orders = 'data/orders/*.parquet';

let topCustomers = from orders
| summarize revenue = sum(amount) by customerId
| top 10 by revenue desc;

from orders
| join kind = inner topCustomers on customerId
| summarize revenue = sum(amount) by customerId, month = startofmonth(timestamp)
| render timechart
```

> **Status: pre-alpha.** Milestone M1 is done: the `ds` command line checks and runs queries
> on local CSV, Parquet and JSON files, and has a REPL. The editor extension, the web UI and the
> browser runtime are planned (see below). The language may still change. Work is tracked in
> [`docs/tasks/`](docs/tasks/README.md).

## Getting started

Build `ds` from source (see [Building](#building)) and run it from the repository root:

```bash
cargo install --path crates/datascript-cli   # installs `ds`; or use `cargo run -p datascript-cli --`
ds run examples/top_customers.ds             # the example above, on the test fixtures
```

Run a query given on the command line:

```bash
ds run -e "from 'tests/data/orders.csv' | summarize orders = count(), revenue = sum(amount) by customerId | top 3 by revenue desc"
```

```
customerId | orders | revenue
string     | long   | real
-----------+--------+--------
c-042      |      7 | 1250.5
c-007      |      6 |  980.0
c-113      |      4 |  402.25
```

Check a script without running it. Every independent error is reported, with a fix when one is
likely:

```
$ ds check typo.ds
error[DS1001]: unknown column `amout`
  --> typo.ds:1:46
  |
1 | from 'tests/data/orders.csv' | summarize sum(amout)
  |                                              ^^^^^ no column named `amout` here
  |
  = help: a column with a similar name exists: `amount`
  = note: columns available at this point: orderId, customerId, amount, timestamp
```

Start the REPL with `ds`. Statements end with `;` or a blank line, `let`s are kept for the
session, and `.help` lists the commands (`.schema 'orders.csv'`, `.tables`, `.format`,
`.max-rows`). Other commands:

| Command | What it does |
|---|---|
| `ds run script.ds [--format table\|csv\|json\|ndjson\|arrow] [--max-rows N]` | Run each query and print its result |
| `ds check script.ds [--deny-warnings]` | Report diagnostics without running |
| `ds explain script.ds` | Print each query's logical plan |
| `ds --table orders=data/orders.parquet` | Start the REPL with `orders` as a table name |

Paths in a script are relative to the script's file. The language is described in
[docs/LANGUAGE.md](docs/LANGUAGE.md).

## Core principles

- **Pipes.** A query reads a table with `from` and passes it through operators chained with `|`,
  read top to bottom.
- **Readable errors are a feature.** Every error says what went wrong, points at the exact span,
  and suggests a fix when one is likely. The CLI, the editor, and the web UI share one
  diagnostics core, so an error looks the same everywhere.
- **Visualization is in the language.** `| render timechart` is part of the query. Runtimes with a UI
  draw the chart. Runtimes without one (CLI, REPL) still show the data.
- **Pluggable engines.** The compiler emits an engine-neutral logical plan. DataFusion is the first
  backend. Others can be added, so DataScript can also serve as a front end for other query engines.
- **Local files first.** Query CSV, Parquet, and JSON straight from disk, DuckDB-style:
  `from 'logs/*.csv' | where level == "error"`.

## Ways to run it

| Surface | Command / package | Milestone |
|---|---|---|
| CLI compiler / checker | `ds check query.ds`, `ds run query.ds` | M1 (done) |
| REPL | `ds` | M1 (done) |
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
| [AGENTS.md](AGENTS.md) | Working agreement for contributors and coding agents |

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
  datascript-fixtures/      Generator for tests/data (`cargo run -p datascript-fixtures`)
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
