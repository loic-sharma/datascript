# Decisions

A lightweight decision log. Add an entry when you make a choice someone might later question.
Status is one of: **accepted**, **proposed** (open for the maintainer to confirm), or **superseded by D-xxx**.

## Accepted

### D-001 — Hand-written recursive-descent parser over a lossless rowan CST
Error messages and recovery are the top priority, and the LSP and formatter need a lossless tree.
Parser generators (lalrpop, pest) make targeted messages and recovery hard. Prior art: rust-analyzer, Biome.

### D-002 — Engine-neutral logical plan IR in the compiler
The compiler lowers to its own small relational IR instead of building DataFusion plans or SQL
directly. This keeps DataFusion out of the core crates (WASM, pluggable backends), lets the
compiler check engine capabilities before running, and gives `ds explain` something stable to print.

### D-003 — Arrow is the result interchange format
Every engine returns Arrow RecordBatches, and every frontend consumes Arrow (Arrow IPC across the
WASM boundary). Most candidate engines already speak Arrow.

### D-004 — `join` defaults to `kind = inner`; no `innerunique`
KQL's default `innerunique` deduplicates the left side silently, which surprises users and would
need a dedicated diagnostic to explain. DataScript uses SQL-like `inner`. Using `innerunique` gives
an error that explains the divergence and how to get the same effect (`distinct` on the left first).

### D-005 — `sort by` defaults to ascending
KQL defaults `sort` to descending, which surprises nearly everyone. `top` stays descending by
default, because top-N is about the largest values.
*(Divergence from KQL; confirm before M1 ships — see Q-001.)*

### D-006 — DuckDB-style file sources
A string literal at the head of a pipe is a file path or glob (`'data/*.parquet' | ...`), with
`read_csv`/`read_parquet`/`read_json` for explicit options. Relative paths resolve against the
script's directory. Chosen by the maintainer.

### D-007 — `=` is never comparison; `let` cannot shadow
Both choices exist to make errors clearer: `=` in a predicate is always a mistake we can fix
automatically, and shadowing a `let` is a common source of confusion in long scripts.

### D-008 — Binary name `ds`, crates `datascript-*`
Short to type. If `ds` turns out to clash with common tools, packaging can also install `datascript`.

### D-009 — Dual MIT / Apache-2.0 license
Rust ecosystem convention, compatible with DataFusion (Apache-2.0).

## Proposed (open questions)

### Q-001 — Confirm the `sort` default (D-005)
Ascending is friendlier, but people porting KQL will get the reverse order with no error. An
alternative is to require an explicit direction (`sort by x` → warning "add asc or desc").

### Q-002 — Web front-end stack
Proposal: TypeScript + Vite + **CodeMirror 6** (smaller than Monaco, easier to embed in WASM
builds) + a light UI layer (Svelte or Preact) + **Vega-Lite** or **ECharts** for charts. Decide at
the start of M4 (task M4-02). The VS Code results panel should reuse the same components.

### Q-003 — `dynamic` representation
JSON-encoded strings (simple, works everywhere) versus Arrow structs/unions or the emerging Arrow
Variant type (faster, more complex). Decide in M2-03.

### Q-004 — Multiple result statements in the web UI
Show tabs per result, or stack them? Decide in M4-02.

### Q-005 — Quoted identifiers vs path strings
`['order id']` (KQL bracket syntax) is a column name, while `'orders.csv'` at a pipe head is a
path. This is unambiguous to the parser, but check with users that it is not confusing.
