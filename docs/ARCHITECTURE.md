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
- `compiler` may depend on `arrow-schema` (type definitions only, for the DataScript ↔ Arrow
  mapping and source schemas). It must not depend on `arrow-array` or any compute crate.
- `engine` may depend on `arrow-schema` / `arrow-array` (Arrow is the interchange format between
  engines and frontends), but not on DataFusion.
- Only `datascript-datafusion` depends on DataFusion. From M1-08 on, CI also runs
  `cargo check -p datascript-datafusion --no-default-features --features wasm --target wasm32-unknown-unknown`
  so the browser build cannot rot before M5.
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
- Keywords are contextual (LANGUAGE.md §1.1). The lexer emits identifiers; the parser checks the
  text where the grammar expects a keyword. The lexer is **context-free** (D-026): it never looks at
  the previous token. Hyphenated operator names (`project-away`) are joined by the parser from
  adjacent `ident` `-` `ident` tokens right after `|`. A context-free lexer keeps incremental
  relexing simple and matches what the TextMate grammar can do. The one lexer-level special case is
  the literal body of `datetime(...)` and `timespan(...)` (LANGUAGE.md §1).
- Positional rules that need no names or types are syntax errors, reported by the parser:
  `render` not last (`DS0120`), missing `;` between statements (`DS0106`).

### Semantics (`datascript-compiler`)

- **Types**: DataScript's own scalar types (see [LANGUAGE.md](LANGUAGE.md#5-types)) with a fixed
  mapping to Arrow types. The compiler reasons in DataScript types and backends map them.
- **Function catalog**: one registry describes every scalar and aggregate function: name,
  signatures, return-type rule, docs, examples. The binder, LSP completion/hover, and the generated
  docs all read it. Backends must say how they implement each function. A missing implementation
  is a compile-time diagnostic (`DS4001`), not a runtime panic.
- **Capabilities**: `Capabilities` (supported functions, join kinds, source formats, and
  `deviations`: semantic rules from LANGUAGE.md the backend knowingly does not follow, such as
  checked integer overflow, D-014) is a plain data type defined **in the compiler**. Engines return one; the compiler's capability pass checks the
  bound program against it and reports `DS4001` at the call site. Because the check is in the
  compiler, `check` in the LSP and WASM reports it too, without executing anything.
- **Schema discovery**: the compiler does not read files. It asks a `SchemaProvider` (implemented
  by the engine layer) for a source's schema, synchronously:
  `schema_of(source: &SourceRef) -> SchemaLookup`, where `SchemaLookup` is `Ready(Schema)`,
  `Failed(SourceError)`, or `Pending`. A file path, glob, `read_csv(...)` call, or host table name
  is a `SourceRef`. `Schema` is an `arrow_schema::Schema` already widened to DataScript types
  (LANGUAGE.md §5.1). `SourceError` is structured (`NotFound { nearby }`, `UnknownFormat`,
  `NoMatches`, `Conflict { files, column, types }`, `Unreadable`, `OutsideRoot`, …) and carries no
  span; the binder turns it into the DS3xxx diagnostic at **every** place the source appears.
- **Source resolution** (D-022): schema lookups are async (they read files), but binding is sync.
  The binder never reads files: it asks the provider, which is a cache. On a miss the provider
  answers `Pending`; the binder gives that source the error type silently (no diagnostic, no
  cascade), keeps going, and lists it in `Analysis::pending_sources`. The session fetches every
  pending schema, fills the cache, and binds again. Because sources may only depend on constants
  (a non-constant path or reader option is `DS3006`), the first pass finds every source, so the
  loop normally ends after the second pass. A third pass that still has pending sources is a bug
  (`DS9001`). Name resolution, `let` handling and constant folding live in one place, the binder;
  there is no separate pre-pass that could disagree with it.
- **Binder output**: a typed HIR where every column reference is resolved and every expression
  has a type. The binder computes the output schema of each pipeline stage, which also drives
  column completion in the LSP.
- **Completion**: `complete(&Analysis, offset) -> Vec<Completion>` lives in the compiler, so the
  LSP, the `ds ui` server and WASM return the same items for the same text.
- **Logical plan IR**: a small, engine-neutral relational algebra:
  `Scan, Filter, Project, Aggregate, Sort, Limit, TopN, Join, Union, Distinct, Values`, with an
  expression IR (`Column, Literal, Call(fn_id, args), Cast, Case, …`). `ds explain` pretty-prints it.
  The IR is deliberately smaller than KQL. Sugar such as `top`, `count`, and `extend` is
  desugared during planning.

Public facade (sketch):

```rust
pub fn parse(text: &str) -> Parse;                          // syntax only (LSP fast path)
pub fn check(src: &SourceFile, schemas: &dyn SchemaProvider, caps: &Capabilities) -> Analysis;
    // Analysis { hir, diagnostics, stage_schemas, pending_sources }
pub fn compile(src: &SourceFile, schemas: &dyn SchemaProvider, caps: &Capabilities)
    -> Result<Program, Vec<Diagnostic>>;
pub fn complete(analysis: &Analysis, offset: TextSize) -> Vec<Completion>;
```

A `Program` contains one `QueryPlan` per output statement, with an optional `RenderSpec` each.

## Engine abstraction (`datascript-engine`)

```rust
// Send on native targets, ?Send on wasm32 (D-011). Declared once, e.g. with `trait-variant`
// or a pair of cfg_attr'd #[async_trait] attributes.
pub trait Engine {
    fn name(&self) -> &str;
    fn capabilities(&self) -> &Capabilities;  // compiler type: functions, join kinds, formats
    async fn schema_of(&self, source: &SourceRef) -> Result<Schema, SourceError>;  // no span: D-023
    async fn execute(&self, plan: &QueryPlan, ctx: &ExecContext) -> Result<ResultStream, Diagnostic>;
}
```

- Why the `Send` split matters: `axum` handlers and the LSP's multi-threaded tokio runtime need
  `Send` futures, while DataFusion on wasm32 is single-threaded. On native, `Session` is
  `Send + Sync` and can be shared behind an `Arc`.
- `ExecContext` carries the clock value for this run (D-015), the base directory for relative
  paths, an optional path sandbox (D-016), and an optional row limit for UI runtimes (Q-007).
- Results are **Arrow** `RecordBatch` streams using the canonical Arrow types (LANGUAGE.md §5).
  Every frontend (terminal table, web grid, JSON/CSV output, WASM → JS through Arrow IPC) consumes
  Arrow. Dropping the stream cancels the query; that is how Ctrl-C, LSP `$/cancelRequest` and the
  web UI's Stop button work.
- Engine errors are mapped to `Diagnostic`s by the backend. When the backend can trace an error
  back to an IR node, it attaches that node's source span. The IR carries spans for this reason.
- A `Session` type in this crate wires everything together for frontends: it holds the engine, a
  schema cache, and the clock, and offers `check`, `explain`, and `run` over a `SourceFile`. It does
  not hold one document's text, so the LSP and the web server use one `Session` for many documents
  and concurrent requests. `check` and `run` run the source-resolution loop: bind against the
  schema cache, fetch `pending_sources` through `Engine::schema_of`, bind again (D-022).
- **Panics**: frontends call the compiler and engine through a boundary that turns a panic into
  `DS9001` (`catch_unwind` on native, a panic hook on wasm32). The LSP and `ds ui` must survive a
  compiler bug.

Future backends (not planned in detail): a SQL emitter (IR → SQL text for DuckDB, SQLite, or
Postgres), which would prove the abstraction works, and remote engines.

## Diagnostics (`datascript-diagnostics`)

See [DIAGNOSTICS.md](DIAGNOSTICS.md) for the message style guide. Structure:

```rust
pub struct Diagnostic {
    pub code: Code,                 // DS0101
    pub severity: Severity,         // Error | Warning | Info
    pub message: String,            // one line, lowercase start, no trailing period
    pub primary: Option<Label>,     // span + short label text; None only for diagnostics that
                                    // have no location (DS0004, untraceable engine errors)
    pub secondary: Vec<Label>,
    pub file: Option<FileId>,       // names the file when `primary` is None (D-027)
    pub notes: Vec<String>,         // "note: ..."
    pub help: Vec<Help>,            // "help: ...", optionally with a machine-applicable Suggestion
}
pub struct Suggestion { pub edits: Vec<TextEdit>, pub applicability: Applicability }
pub struct TextEdit { pub span: Span, pub replacement: String }   // non-overlapping
```

The JSON form has a top-level `"version": 1` field. Bump it on any breaking change; the web UI and
`@datascript/wasm` check it. Every JSON consumer is JavaScript (CodeMirror, the WASM API, VS Code
webviews), so positions in JSON are **UTF-16 code units** (D-025): each span has `start`/`end`
offsets and 1-based `line`/`column`, all counted in UTF-16. Byte offsets are not part of the JSON
form. The terminal and plain renderers count columns in characters, and print help messages but
not suggestion edits (D-027).

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
  last operator of a query (enforced by the parser, `DS0120`). It does not change the data. It attaches a
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
| VS Code | `editors/vscode` | TS client, TextMate grammar, "▶ Run" CodeLens above each query statement, results webview reusing `web/` components. Honors Workspace Trust: in an untrusted workspace the server runs syntax-only (no schema reads, no Run) |
| Local web UI | `datascript-server` + `web/` | `ds ui` starts axum on 127.0.0.1 with a random port and a session token, serves the embedded SPA, exposes a JSON/WebSocket API. Query sources are confined to the root directory (D-016) |
| WASM | `datascript-wasm` + `web/` | Same SPA with a client-side runtime adapter; files come from drag-drop, File System Access API, or URLs |

The `web/` front end talks to a `Runtime` interface (TypeScript) with two implementations,
`ServerRuntime` (HTTP to `ds ui`) and `WasmRuntime` (in-process), so the UI code is written once.

## WASM constraints

- DataFusion builds for `wasm32-unknown-unknown` with default features off plus `parquet`, and
  runs with `target_partitions = 1` and no tokio runtime. File bytes are put into an in-memory
  `ObjectStore` before queries run. Spike M0-05 confirmed this: CSV and Parquet queries ran in Node
  and Chromium, and the engine module is about 5.8 MB gzipped. Its findings, including the
  workarounds (a wasm-capable clang for zstd-sys, getrandom `wasm_js`, chrono `wasmbind`), are in
  [spikes/datafusion-wasm.md](spikes/datafusion-wasm.md). CI keeps checking the build from M1-08.
- Core crates must avoid `std::fs`, `std::time::SystemTime::now()` (use an injected clock, because
  `now()` and `ago()` need one), and threads.
- The browser build ships as **two wasm modules** behind one JS API (`@datascript/wasm`): a small
  compiler module (syntax, compiler, diagnostics: `check`, `complete`, `format`) that loads at
  once, and the large engine module (DataFusion) that loads lazily on the first `run` or the first
  schema lookup. Diagnostics and completion for syntax and names then work before the engine has
  downloaded. Both run in a Web Worker; cancelling a run terminates or interrupts that worker,
  because a CPU-bound query on a single thread cannot see a dropped stream.
- `@datascript/wasm` exposes: `registerFile(name, bytes)`, `check(text) -> Diagnostic[]`,
  `run(text) -> { arrowIpc, render, diagnostics }`, `complete(text, offset)`.
- Binary size matters. Track it in CI once M5 starts.
