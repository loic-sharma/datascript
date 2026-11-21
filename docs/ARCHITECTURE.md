# Architecture

This document describes the target architecture. Tasks in [`tasks/`](tasks/README.md) build it
incrementally. When the code and this document disagree, fix one of them in the same change.

## Goals that shape the design

1. **One compiler, many surfaces.** CLI, REPL, LSP, local web UI, and WASM all call the same
   compiler API and get the same diagnostics.
2. **Editor-grade front end.** The parser must survive broken input (recovery), keep every byte
   (lossless tree, for formatting and LSP), and be fast enough to run on every keystroke.
3. **Engine independence.** Nothing above the engine boundary knows about DataFusion.
4. **Runs in the browser.** Core crates compile to `wasm32-unknown-unknown` with no threads,
   no filesystem, and no tokio.

## Crate graph

```
                    ┌──────────────────────────┐
                    │  datascript-diagnostics  │  Diagnostic, Span, SourceMap, renderers
                    └────────────▲─────────────┘
                                 │
                    ┌────────────┴─────────────┐
                    │    datascript-syntax     │  lexer, parser, CST (rowan), typed AST
                    └────────────▲─────────────┘
                                 │
                    ┌────────────┴─────────────┐
                    │   datascript-compiler    │  binder, types, functions, logical plan IR
                    └────────────▲─────────────┘
                                 │
                    ┌────────────┴─────────────┐
                    │    datascript-engine     │  Engine + Catalog traits, QueryResult,
                    └────────────▲─────────────┘  RenderSpec, session facade
                                 │
                    ┌────────────┴─────────────┐
                    │  datascript-datafusion   │  lowering IR → DataFusion LogicalPlan
                    └────────────▲─────────────┘
           ┌─────────────────┬───┴─────────────┬───────────────────┐
   datascript-cli     datascript-lsp    datascript-server    datascript-wasm
   (ds binary, REPL)  (tower-lsp)       (axum, ds ui)        (wasm-bindgen)
                                                 │                   │
                                                 └──── web/ ─────────┘
                                                (shared TS front end)
```

Rules:

- `diagnostics`, `syntax`, `compiler` are **pure**. No I/O, no async, no DataFusion, no Arrow
  compute. They must `cargo check` for `wasm32-unknown-unknown`. CI enforces this.
- `engine` may depend on `arrow-schema` / `arrow-array` (Arrow is the interchange format between
  engines and frontends), but not on DataFusion.
- Only `datascript-datafusion` depends on DataFusion.
- Frontends depend on `engine` and pick a backend at startup.

## Compilation pipeline

```
source text
  │  lex            (syntax)      tokens + lexical diagnostics
  ▼
  │  parse          (syntax)      lossless CST + syntax diagnostics (with recovery)
  ▼
  │  lower to AST   (syntax)      typed AST view over the CST (no allocation copy)
  ▼
  │  bind           (compiler)    resolve lets, sources, columns, functions; infer schemas;
  │                               type-check expressions → HIR + semantic diagnostics
  ▼
  │  plan           (compiler)    engine-neutral logical plan (relational algebra) + RenderSpec
  ▼
  │  lower          (backend)     IR → engine plan (DataFusion LogicalPlan)
  ▼
  │  execute        (backend)     stream of Arrow RecordBatches
  ▼
QueryResult { schema, batches, render: Option<RenderSpec>, warnings }
```

Each stage keeps going after errors where it can, so one run reports as many independent problems
as possible. A stage only runs if earlier stages produced a usable tree; binder errors on one
statement do not stop other statements from being checked.

### Syntax (`datascript-syntax`)

- Hand-written lexer and **recursive-descent parser**. Hand-written beats a generator here because
  error messages and recovery are the top priority.
- Lossless concrete syntax tree using [`rowan`](https://crates.io/crates/rowan) (as in
  rust-analyzer). Whitespace and comments are trivia tokens in the tree. That gives us the formatter,
  LSP ranges, and error recovery for free.
- A typed AST layer (`ast::WhereOp`, `ast::BinaryExpr`, …) wraps CST nodes.
- Parser output is always a tree, even for garbage input. Error nodes wrap what could not be
  parsed. Recovery points are `|`, `;`, and operator keywords at the start of a line.

### Semantics (`datascript-compiler`)

- **Types**: DataScript's own scalar types (see [LANGUAGE.md](LANGUAGE.md#types)) with a fixed
  mapping to Arrow types. The compiler reasons in DataScript types and backends map them.
- **Function catalog**: one registry describes every scalar and aggregate function: name,
  signatures, return-type rule, docs, examples. The binder, LSP completion/hover, and the generated
  docs all read it. Backends must say how they implement each function. A missing implementation
  is a compile-time diagnostic (`DS4xxx`), not a runtime panic.
- **Schema discovery**: the compiler does not read files. It asks a `SchemaProvider` (implemented
  by the engine layer) for a source's schema: `schema_of(source: &SourceRef) -> Result<Schema, SourceError>`.
  A file path, glob, or `read_csv(...)` call is a `SourceRef`. For the LSP and the browser, the
  provider caches results.
- **Binder output**: a typed HIR where every column reference is resolved and every expression
  has a type. The binder computes the output schema of each pipeline stage, which also drives
  column completion in the LSP.
- **Logical plan IR**: a small, engine-neutral relational algebra:
  `Scan, Filter, Project, Aggregate, Sort, Limit, TopN, Join, Union, Distinct, Values`, with an
  expression IR (`Column, Literal, Call(fn_id, args), Cast, Case, …`). `ds explain` pretty-prints it.
  The IR is deliberately smaller than KQL. Sugar such as `top`, `count`, and `extend` is
  desugared during planning.

Public facade (sketch):

```rust
pub fn parse(text: &str) -> Parse;                          // syntax only (LSP fast path)
pub fn check(src: &SourceFile, schemas: &dyn SchemaProvider) -> Analysis;  // + semantics
pub fn compile(src: &SourceFile, schemas: &dyn SchemaProvider) -> Result<Program, Vec<Diagnostic>>;
```

A `Program` contains one `QueryPlan` per output statement, with an optional `RenderSpec` each.

## Engine abstraction (`datascript-engine`)

```rust
#[async_trait(?Send)]               // ?Send so it works on wasm32
pub trait Engine {
    fn name(&self) -> &str;
    fn capabilities(&self) -> &Capabilities;  // supported functions, join kinds, source formats
    async fn schema_of(&self, source: &SourceRef) -> Result<Schema, Diagnostic>;
    async fn execute(&self, plan: &QueryPlan) -> Result<ResultStream, Diagnostic>;
}
```

- `Capabilities` lets the compiler report "`percentile()` is not supported by the X engine" at
  compile time, pointing at the call site.
- Results are **Arrow** `RecordBatch` streams. Every frontend (terminal table, web grid,
  JSON/CSV output, WASM → JS through Arrow IPC) consumes Arrow.
- Engine errors are mapped to `Diagnostic`s by the backend. When the backend can trace an error
  back to an IR node, it attaches that node's source span. The IR carries spans for this reason.
- A `Session` type in this crate wires everything together for frontends: it holds the source
  text, the engine, and the schema cache, and offers `check`, `explain`, and `run`.

Future backends (not planned in detail): a SQL emitter (IR → SQL text for DuckDB, SQLite, or
Postgres), which would prove the abstraction works, and remote engines.

## Diagnostics (`datascript-diagnostics`)

See [DIAGNOSTICS.md](DIAGNOSTICS.md) for the message style guide. Structure:

```rust
pub struct Diagnostic {
    pub code: Code,                 // DS0101
    pub severity: Severity,         // Error | Warning | Info
    pub message: String,            // one line, lowercase start, no trailing period
    pub primary: Label,             // span + short label text
    pub secondary: Vec<Label>,
    pub notes: Vec<String>,         // "note: ..."
    pub help: Vec<Help>,            // "help: ...", optionally with a machine-applicable Suggestion
}
pub struct Suggestion { pub span: Span, pub replacement: String, pub applicability: Applicability }
```

Renderers, all fed from the same struct:

| Renderer | Used by |
|---|---|
| Terminal (color, Unicode, rustc-style snippets) | CLI, REPL |
| Plain (no color, ASCII-stable) | golden tests, `--color never` |
| JSON (serde) | web UI, WASM, `ds check --format json` |
| LSP conversion (`lsp_types::Diagnostic` + code actions from suggestions) | LSP |

The web UI renders JSON diagnostics in the editor and can show the same snippet view as the terminal.

## Render layer

- `render <kind> [with (title = "...", xcolumn = ..., ycolumns = ..., series = ...)]` must be the
  last operator of a query (enforced by the binder). It does not change the data. It attaches a
  `RenderSpec { kind, x, y: Vec<_>, series, title, … }` to the result.
- The compiler validates the spec against the output schema, e.g. "`timechart` needs a datetime
  column for the x-axis, but the result has none".
- Frontends decide what to do with it:
  - Web UI and VS Code results panel: draw the chart.
  - CLI and REPL: print the table and an info line saying the chart was not drawn here. Later:
    `--render-out chart.html`.
- The mapping from RenderSpec to a chart library lives in `web/` (TypeScript). The Rust side only
  produces the spec.

## Frontends

| Frontend | Crate | Notes |
|---|---|---|
| CLI | `datascript-cli` | `ds check`, `ds run`, `ds explain`, `ds fmt`; output as table, CSV, JSON, or Arrow IPC; exit codes 0/1/2 |
| REPL | `datascript-cli` | `ds` with no args; reedline; multi-line input until `;` or a blank line; `.schema <source>`, `.help` |
| LSP | `datascript-lsp` | tower-lsp-server; uses `parse`/`check` with a caching SchemaProvider; custom `datascript/run` request for the editor's Run button |
| VS Code | `editors/vscode` | TS client, TextMate grammar, "▶ Run" CodeLens above each query statement, results webview reusing `web/` components |
| Local web UI | `datascript-server` + `web/` | `ds ui` starts axum on 127.0.0.1 with a random port and a session token, serves the embedded SPA, exposes a JSON/WebSocket API |
| WASM | `datascript-wasm` + `web/` | Same SPA with a client-side runtime adapter; files come from drag-drop, File System Access API, or URLs |

The `web/` front end talks to a `Runtime` interface (TypeScript) with two implementations,
`ServerRuntime` (HTTP to `ds ui`) and `WasmRuntime` (in-process), so the UI code is written once.

## WASM constraints

- DataFusion supports `wasm32-unknown-unknown` with default features off: no `parquet` async
  readers that need tokio fs, single-threaded execution. File bytes are registered into an
  in-memory `ObjectStore` before queries run.
- Core crates must avoid `std::fs`, `std::time::SystemTime::now()` (use an injected clock, because
  `now()` and `ago()` need one), and threads.
- `datascript-wasm` exposes: `registerFile(name, bytes)`, `check(text) -> Diagnostic[]`,
  `run(text) -> { arrowIpc, render, diagnostics }`, `complete(text, offset)`.
- Binary size matters. Track it in CI once M5 starts.
