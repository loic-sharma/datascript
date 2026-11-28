# Tasks

One file per unit of work. Each task should fit in a single focused agent session.

**How to use this** (also in [AGENTS.md](../../AGENTS.md)):

1. Pick a `todo` task whose dependencies are all `done`.
2. Set it to `in-progress` here *and* in the task file. Work on branch `task/<id>-<slug>`.
3. When finished, set it to `done` with a one-line summary in the task file. Copy every follow-up
   that another task should do into that task's "Carried over" section (`- [ ] (from <id>) …`).
   The "Carried over" items in your own task file are part of your scope.
4. Found extra work? Add a new task file with the next free id in that milestone and a row below.
   Don't grow the current task.

Statuses: `todo` · `in-progress` · `done` · `blocked` (say why in the task file).

Tasks within a milestone that share no dependency edge can run in parallel sessions. They must not
edit the same files. The "Parallel with" hints below list safe pairs.

M1 starts with a walking skeleton (M1-15): one query runs through every layer before any layer is
built out, so interface mistakes show up early. Later M1 tasks extend that code.
M1-16 and M1-17 both edit the binder; they run in parallel only if each keeps to its own modules
(operators and joins vs. sources and capabilities).
M1-18…M1-23 came out of a review of M1-01…M1-04. M1-18, M1-19, M1-21 and M1-22 all edit the
parser in `datascript-syntax`. M1-19 and M1-21 run in parallel only if each keeps to its own code
(grammar and recovery vs. fixes in `targeted.rs`).
M1-24 (require `from`) changes the grammar the binder reads, so M1-05 waits for it. M1-25 (the
diagnostics and statement-start rule that follow from `from`) touches only the parser, so it runs in
parallel with M1-05 and later binder tasks.

Two shared files are touched by many tasks:
- **This index.** Status-only conflicts in the table are expected when parallel branches merge.
  Resolve them by keeping every row's newest status.
- **The diagnostic code registry.** Each task allocates codes only from the block named in its task
  file (D-018, [DIAGNOSTICS.md](../DIAGNOSTICS.md#error-code-ranges)).

## M0 — Foundation

| ID | Task | Depends on | Status | Parallel with |
|---|---|---|---|---|
| [M0-01](M0-01-workspace-scaffold.md) | Workspace scaffold and CI | — | done | |
| [M0-02](M0-02-diagnostics-core.md) | Diagnostics core | M0-01 | done | M0-04 |
| [M0-03](M0-03-golden-test-harness.md) | Golden test harness | M0-01, M0-02 | done | M0-04 |
| [M0-04](M0-04-test-fixtures.md) | Test fixture datasets | M0-01 | done | M0-02, M0-03 |
| [M0-05](M0-05-datafusion-wasm-spike.md) | Spike: DataFusion on wasm32 | M0-01 | done | M0-02, M0-03, M0-04 |

## M1 — Core language + CLI

| ID | Task | Depends on | Status | Parallel with |
|---|---|---|---|---|
| [M1-01](M1-01-lexer.md) | Lexer | M0-02, M0-03 | done | M1-04 |
| [M1-02](M1-02-parser.md) | Parser and syntax tree | M1-01 | done | M1-04 |
| [M1-03](M1-03-syntax-diagnostics.md) | Syntax diagnostics quality pass | M1-02 | done | M1-05 and later |
| [M1-04](M1-04-types-function-catalog.md) | Type system and function catalog | M0-02 | done | M1-01, M1-02 |
| [M1-15](M1-15-walking-skeleton.md) | Walking skeleton: one query through every layer | M1-02, M1-04, M0-04, M0-05 | done | M1-03 |
| [M1-18](M1-18-parser-long-input.md) | Parser: long and deep input | M1-03 | done | M1-20, M1-23, M1-05 and later |
| [M1-19](M1-19-parser-grammar-recovery.md) | Parser: grammar fixes and recovery | M1-18 | done | M1-20, M1-23, M1-05 and later |
| [M1-20](M1-20-lexer-literal-fixes.md) | Lexer and literal fixes | M1-03 | done | M1-18, M1-19, M1-21, M1-23 |
| [M1-21](M1-21-syntax-fix-correctness.md) | Syntax suggestions that produce valid code | M1-18 | done | M1-19, M1-20, M1-23 |
| [M1-22](M1-22-syntax-diagnostics-coverage.md) | Syntax diagnostics: more habits and style fixes | M1-21, M1-19 | done | M1-23, M1-05 and later |
| [M1-23](M1-23-type-system-fixes.md) | Type system and catalog fixes | M1-04 | done | M1-18…M1-22 |
| [M1-24](M1-24-from-keyword.md) | Require `from` at the start of a query | M1-22 | done | M1-23 |
| [M1-25](M1-25-from-diagnostics.md) | `from` mistakes and statement starts | M1-24 | done | M1-23, M1-05 and later |
| [M1-05](M1-05-binder.md) | Binder core and expression checking | M1-15, M1-24 | done | M1-03 |
| [M1-16](M1-16-operator-schemas-joins.md) | Binder: operator schemas, column naming, joins | M1-05 | done | M1-03, M1-17 |
| [M1-17](M1-17-source-resolution-capabilities.md) | Binder: source resolution and capability pass | M1-05 | done | M1-03, M1-16 |
| [M1-06](M1-06-logical-plan-ir.md) | Logical plan IR and `explain` | M1-16, M1-17 | done | M1-03 |
| [M1-07](M1-07-engine-api.md) | Engine API and Session | M1-06 | done | M1-03, M1-10 |
| [M1-08](M1-08-datafusion-sources.md) | DataFusion: sources and schemas | M1-07, M0-04, M0-05 | done | M1-10 |
| [M1-09](M1-09-datafusion-lowering.md) | DataFusion: lowering and execution | M1-08 | done | M1-10 |
| [M1-10](M1-10-render-operator.md) | `render` operator semantics | M1-16, M1-06 | done | M1-14 |
| [M1-11](M1-11-cli.md) | CLI: check, run, explain | M1-09 | done | |
| [M1-12](M1-12-repl.md) | REPL | M1-11 | done | |
| [M1-13](M1-13-m1-acceptance.md) | M1 acceptance and polish | M1-03, M1-10, M1-11, M1-12, M1-14, M1-18…M1-25 | done | |
| [M1-14](M1-14-lints.md) | Warning lints | M1-16 | done | M1-06…M1-12 |

## M2 — Language breadth

| ID | Task | Depends on | Status |
|---|---|---|---|
| [M2-01](M2-01-tabular-operators.md) | Remaining tabular operators | M1-13, M2-13 | todo |
| [M2-02](M2-02-parse-operator.md) | `parse` operator | M1-13, M2-13 | todo |
| [M2-03](M2-03-dynamic-type.md) | `dynamic` type, JSON, `mv-expand` | M1-13, M2-13 | todo |
| [M2-04](M2-04-make-series.md) | `make-series` | M2-03, M2-13 | todo |
| [M2-05](M2-05-user-defined-functions.md) | User-defined functions | M1-13, M2-13 | todo |
| [M2-06](M2-06-function-library.md) | Complete the function library | M1-13, M2-13 | todo |
| [M2-07](M2-07-formatter.md) | Formatter (`ds fmt`) | M1-13 | todo |
| [M2-08](M2-08-error-explanations.md) | Long-form error explanations | M1-13, M2-13 | todo |
| [M2-09](M2-09-schema-overrides.md) | Reader schema overrides | M1-13, M2-13 | todo |
| [M2-10](M2-10-decimal-guid.md) | `decimal` and `guid` types | M1-13, M2-13 | todo |
| [M2-11](M2-11-streaming-run.md) | Stream results in `ds run` | M1-13 | done |
| [M2-12](M2-12-runtime-source-errors.md) | Name the file in runtime read errors | M1-13, M2-13 | todo |
| [M2-13](M2-13-task-contract-fixes.md) | Fix task contracts and diagnostic code blocks | M1-13 | done |
| [M2-14](M2-14-spec-code-mismatches.md) | Spec/code mismatches from the docs review | M1-13 | done |
| [M2-15](M2-15-golden-run-gaps.md) | Golden `run` coverage gaps | M1-13 | done |
| [M2-16](M2-16-cli-repl-test-gaps.md) | CLI and REPL integration test gaps | M2-11 | done |
| [M2-17](M2-17-sumif-overflow-message.md) | `sumif` overflow names `sum` instead of `sumif` | M2-15 | done |
| [M2-18](M2-18-calendar-year-range-min.md) | `calendar::year_range` panics on `i64::MIN` year | M2-15 | done |
| [M2-19](M2-19-trailing-commas.md) | Trailing commas in lists | M1-13 | done |
| [M2-20](M2-20-conversion-diagnostics.md) | Conversions that can never succeed | M1-13 | todo |
| [M2-21](M2-21-division-by-zero-guard-help.md) | `DS4003` help: guard the divisor per row | M1-13 | todo |
| [M2-22](M2-22-csv-sniffing.md) | CSV sniffing | M1-13, M2-13 | todo |
| [M2-23](M2-23-describe-operator.md) | `describe` operator | M1-13, M2-13 | todo |
| [M2-24](M2-24-excel-reader.md) | Excel reader | M1-13, M2-09, M2-13 | todo |
| [M2-25](M2-25-output-operator.md) | `output` operator | M1-13, M2-13 | todo |
| [M2-26](M2-26-writer-functions.md) | `output`: writer functions, options and compression | M2-25, M2-27 | todo |
| [M2-27](M2-27-output-parquet-safety.md) | `output`: Parquet, self-overwrite check and read-back check | M2-25 | todo |
| [M2-28](M2-28-excel-writer.md) | Excel writer | M2-24, M2-26 | todo |
| [M2-29](M2-29-engine-datafusion-integration-tests.md) | Engine and DataFusion integration tests | M2-15 | done |
| [M2-30](M2-30-compiler-harness-ci-test-gaps.md) | Golden harness, CI and testing-docs gaps | M2-32 | done |
| [M2-31](M2-31-cli-repl-test-improvements.md) | CLI and REPL bug fixes and test hygiene | M2-16, M3-07 | done |
| [M2-32](M2-32-compiler-ide-bug-fixes.md) | Compiler, IDE and diagnostics bug fixes from the test audit | — | done |

## M3 — Editor experience

| ID | Task | Depends on | Status |
|---|---|---|---|
| [M3-01](M3-01-lsp-diagnostics.md) | LSP core and diagnostics | M1-13 | done |
| [M3-02](M3-02-lsp-completion-hover.md) | LSP completion, hover, signature help | M3-01 | done |
| [M3-03](M3-03-lsp-navigation-fixes.md) | LSP quick fixes, navigation, semantic tokens | M3-01, M2-13 | done |
| [M3-04](M3-04-vscode-extension.md) | VS Code extension | M3-01 | done |
| [M3-05](M3-05-vscode-run.md) | VS Code Run CodeLens with CLI-style output | M3-04 | done |
| [M3-06](M3-06-vscode-results-panel.md) | VS Code results panel (grid and charts) | M3-05, M4-02, M4-03, M2-13 | todo |
| [M3-07](M3-07-repl-column-completion.md) | REPL column completion | M3-02 | done |
| [M3-08](M3-08-lsp-test-gaps.md) | LSP integration test gaps | M3-05 | done |
| [M3-09](M3-09-vscode-e2e-gaps.md) | VS Code extension E2E test gaps | M3-05 | done |
| [M3-10](M3-10-lsp-vscode-test-improvements.md) | LSP and VS Code test improvements | M3-08, M3-09 | todo |

## M4 — Local web UI

| ID | Task | Depends on | Status |
|---|---|---|---|
| [M4-01](M4-01-server-api.md) | Local server (`ds ui`) | M1-13, M2-13 | todo |
| [M4-02](M4-02-web-frontend.md) | Web UI front end | M4-01, M2-13 | todo |
| [M4-03](M4-03-web-charts.md) | Charts from RenderSpec | M4-02, M1-10 | todo |
| [M4-04](M4-04-web-schema-completion.md) | Schema browser and completion | M4-02, M3-02, M2-13 | todo |

## M5 — In-browser runtime (WASM)

| ID | Task | Depends on | Status |
|---|---|---|---|
| [M5-01](M5-01-wasm-build.md) | Build the engine for wasm32 | M1-13, M2-13 | todo |
| [M5-02](M5-02-wasm-js-api.md) | JS/TS API (`@datascript/wasm`) | M5-01, M3-02, M2-13 | todo |
| [M5-03](M5-03-wasm-playground.md) | WASM runtime adapter and playground | M5-02, M4-02, M4-03, M4-04, M2-13 | todo |
| [M5-04](M5-04-parity-tests.md) | CLI ↔ WASM parity tests | M5-02 | todo |

## M6 — Release readiness

| ID | Task | Depends on | Status |
|---|---|---|---|
| [M6-01](M6-01-reference-docs.md) | Language reference and docs site | M2-08, M5-03, M2-13, M6-04 | todo |
| [M6-02](M6-02-packaging.md) | Packaging and releases | M1-13, M2-13, M3-04, M4-02, M5-02 | todo |
| [M6-03](M6-03-sql-backend.md) | Second backend: SQL emitter | M1-13 | todo |
| [M6-04](M6-04-core-docs-refresh.md) | Bring the core docs in line with the code | M2-13, M2-14 | todo |

## M7 — Data access and embedding

| ID | Task | Depends on | Status |
|---|---|---|---|
| [M7-01](M7-01-https-sources.md) | HTTPS sources | M1-13, M2-13 | todo |
| [M7-02](M7-02-cloud-object-stores.md) | S3, GCS and Azure sources | M7-01 | todo |
| [M7-03](M7-03-python-package.md) | Python package | M2-11, M2-13, M7-05 | todo |
| [M7-04](M7-04-c-api.md) | C API | M2-11, M2-13, M7-05 | blocked |
| [M7-05](M7-05-in-memory-host-tables.md) | In-memory host tables on a live session | M2-13 | todo |

The editor (M3) and the local web UI (M4) are being done before language breadth (M2). Where an M2
task adds something the editor or web UI must show, its "Carried over" section says so.

After M1, milestones M2–M7 are largely independent tracks (language, editor, web, wasm) that can
proceed in parallel. Keep wasm32 compatibility (checked in CI) intact on every track so M5 doesn't
turn into a big-bang port.

M2-22, M2-24…M2-28, M7-01 and M7-02 all touch source resolution or the reader and writer catalogs
(`source::READERS`, `is_glob`, `SourceRef::for_path` and the DataFusion source code); run them one
at a time, or keep each to its own modules. M2-23 (`describe`), M2-25 (`output`) and M2-26 (writer
option syntax) edit `datascript-syntax`, so none of them should run in parallel with M2-01, M2-03,
M2-04, M2-05 or each other. M7-03 and M7-04 both need M7-05 (in-memory host tables) first; once
M7-04 is unblocked, the two can run in parallel.

Within M2, M2-19 edits `list()` in `datascript-syntax` (`grammar.rs`) and should not run in parallel
with M2-01, M2-03, M2-04 or M2-05 (prefer landing M2-19 first).

M2-29, M2-31, M2-32 and M3-10 can run in parallel: they touch `datascript-datafusion`/
`datascript-engine` tests; `datascript-cli`; `datascript-compiler`/`datascript-diagnostics` (and
one test in `datascript-syntax`); and `datascript-lsp`/`lsp_run.rs`/`editors/vscode`. M2-30 waits
for M2-32, because verifying compiler fixes in the golden harness fails on M2-32's unquoted
did-you-mean edits. M2-31 and M3-10 both edit `crates/datascript-cli` (`main.rs` and `Cargo.toml`
respectively); coordinate if both run at once.
