# Roadmap

Milestones are ordered by dependency. Within a milestone, tasks can often run in parallel. See the
dependency column in [tasks/README.md](tasks/README.md).

## M0 — Foundation

Workspace, CI, the diagnostics core, and the golden-test harness. After M0, every later task can
write failing `.dstest` cases first. A spike (M0-05) checks early that DataFusion runs on wasm32, because
M5 depends on it.

**Done when:** `cargo test --workspace` runs an (initially trivial) `.dstest` suite in CI; core
crates are checked for wasm32 in CI; diagnostics render in terminal, plain, and JSON form.

## M1 — Core language + CLI

Parse, check, and run real queries on local CSV/Parquet/JSON files from the command line.
It starts with a walking skeleton (M1-15): one simple query runs through every layer first,
then each layer is built out.

**Done when** (all checked by M1-13, 2026-11-22):
- [x] The README example runs end to end with `ds run` against fixtures
  (`tests/cases/examples/readme.dstest`, `examples/top_customers.ds`).
- [x] All ● operators and the M1 function set in [LANGUAGE.md](LANGUAGE.md) work, each with
  happy-path and error golden tests (`tests/cases/operators/`, `tests/cases/functions/*/`).
- [x] `ds check` reports multiple independent errors per file with did-you-mean suggestions.
- [x] `ds` (REPL) supports multi-line queries, history, and `.schema`.
- [x] `render` is parsed, validated, and shown as an info note in the CLI.

## M2 — Language breadth

○ operators, `dynamic`, user-defined functions, the formatter (`ds fmt`), the rest of the function
library, and long-form error explanations.

**Done when:** a realistic log-analysis tutorial (JSON logs, parse, mv-expand, make-series) runs end
to end, and `ds fmt` is idempotent over the whole test corpus.

## M3 — Editor experience

Language server plus VS Code extension: live diagnostics, completion (operators, columns from
inferred schemas, functions, file paths), hover docs, go to definition for `let`, quick fixes from
suggestions, and a "▶ Run" CodeLens that shows results (and charts, after M4) in a panel.

**Done when:** typing in VS Code shows the same diagnostics as `ds check` within ~100 ms on typical
scripts, and Run shows a result grid.

## M4 — Local web UI

`ds ui` starts a local server and opens a notebook-first web workbench ([WEB_UI.md](WEB_UI.md)):
a working-directory file tree, `.dsnb` Markdown/DataScript notebooks (and `.ds` scripts) with
inline diagnostics, completion, virtualized Arrow grids, ECharts visualizations for every render
kind, and an on-demand right-hand Result Profiler & Source Schema inspector. Power-user workspace
polish (`M4-05`), CLI/LSP `.dsnb` support (`M4-06`), and optional local output caching (`M4-07`)
build cleanly on the v1 core (`M4-08`, `M4-01`, `M4-10`, `M4-02`, `M4-09`, `M4-03`, `M4-04`).

**Done when (M4 v1):** a new user can `ds ui` in a folder of CSVs/Parquet files, preview a file in
one click from the Scratchpad, inspect schemas from the file tree, author and reorder a multi-cell
`.dsnb` notebook with shared `let`s, inline quick fixes, completion, and auto-save, profile
results in the inspector, and render a timechart.

## M5 — In-browser runtime (WASM)

The same web UI running fully client-side: files come from drag-drop or URLs, and execution uses
DataFusion compiled to WASM. Published as `@datascript/wasm` plus a static playground site.

**Done when:** the playground runs the README example on dropped files with results identical to
the CLI (checked by a parity test over the golden corpus).

## M6 — Release readiness

Language reference generated from the function catalog, prebuilt binaries and installers, VS Code
Marketplace and npm publishing, and a second backend (SQL emitter) that proves the engine
abstraction works.

## M7 — Data access and embedding

Read remote data (HTTPS, then S3, GCS and Azure) and embed DataScript in other languages: a
Python package that returns Arrow, pandas and Polars results. A C API for other languages is
written up (M7-04) but waits for a consumer.

**Done when:** `from 's3://bucket/logs/*.parquet'` runs from the CLI, and
`datascript.run(...).df()` works in a notebook with the same diagnostics as the CLI.

## Explicitly later / unscheduled

Hive-partitioned directories, reading and writing (Q-006), query parameters, Tauri desktop app
packaging (reusing `web/` over a native `TauriRuntime`), **materialized / cached `let` results**
(now higher priority once `.dsnb` notebooks land, since downstream cells currently re-scan upstream
sources on each run), additional engines beyond the SQL emitter, and localization of error
messages.
