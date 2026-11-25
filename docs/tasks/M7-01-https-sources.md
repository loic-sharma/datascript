# M7-01 — HTTPS sources

**Status:** todo
**Depends on:** M1-13, M2-13
**Crates:** `datascript-compiler`, `datascript-datafusion`, `datascript-engine`, `datascript-cli`, `datascript-lsp`
**Diagnostic codes:** DS3100–DS3119 (D-018)

## Goal
`from 'https://example.com/data/orders.parquet'` reads a remote file directly, in the CLI, the
REPL, the language server and `ds ui`. Today "URLs are not sources" (LANGUAGE.md §3.1); in WASM
their contents are fetched by `registerFile`.

## Scope
- Use `object_store`'s HTTP store (DataFusion already depends on `object_store`). Enable its `http`
  feature for native builds only, so the `wasm-datafusion` CI check keeps passing. Parquet uses
  range requests, so `take 10` or a projection does not download the whole file.
- Format from the URL path's extension, ignoring the query string. Reader functions take URLs too.
- Globs over HTTP cannot list files: a URL whose **path** (before `?` and `#`) contains `*`, `?` or
  `[` is an error from this task's block that says so. A query string is not a glob: `is_glob` and
  `SourceRef::for_path` (`crates/datascript-compiler/src/source.rs`) must look only at the URL path.
- `https://` and plain `http://` are both allowed, as in DuckDB; no credentials are ever sent, and
  the tests use a local `http://` server.
- **Schema caching.** A remote schema is fetched once and kept:
  - Add `allow_remote` to both cache keys, the session's `CacheKey`
    (`crates/datascript-engine/src/session.rs`) and `DataFusionEngine`'s `SourceKey`
    (`crates/datascript-datafusion/src/lib.rs`), as `sandbox` already is (D-043).
  - The REPL clears every schema before each submission (D-048), which would mean an HTTP request
    per query. Instead it keeps remote schemas; a new `.reload` command clears them. Local schemas
    behave as today.
  - The LSP and `ds ui` keep remote schemas for the session; `.reload` (REPL) and restarting are
    the ways to refresh. ETag checks are not in this task.
- **`--table` with a URL.** `ds --table orders=https://…` must keep the URL as it is: the CLI
  makes table paths absolute with `std::path::absolute` (`crates/datascript-cli/src/main.rs`),
  which would turn it into `<cwd>/https:/…`. Skip that for anything with a scheme.
- Writing to a URL with `output` is refused by M2-25 (or by this task, if it lands first); this
  task does not add remote writes.
- Errors from this task's block: HTTP status (404 with the URL, 401/403 with a note that
  credentials are not supported here), DNS failure, timeout, TLS failure. Never include
  credentials or query-string secrets in a diagnostic.
- Policy, recorded in DECISIONS.md: add an `allow_remote` flag to `ExecContext`. The CLI and REPL
  set it; the language server sets it only in a trusted workspace (D-053), so schemas are fetched
  only there; `ds ui` leaves it off unless started with `--allow-remote` (D-016's sandbox covers
  the filesystem only). With the flag off, a URL source is an error from this task's block. If
  M4-01 is not done, its "Carried over" item wires the flag and the `--allow-remote` option.
- WASM is not this task's: the `object_store` HTTP store is native-only. M5-02's "Carried over"
  item covers fetching URL sources in the browser. If M5-02 is already done, add that work to
  M5-03's "Carried over" section (or a new task with an index row) instead.
- Update LANGUAGE.md §3.1.

## Acceptance criteria
- [ ] Tests against a local HTTP server started by the test (no network in CI): CSV, Parquet with
      range reads (assert fewer bytes fetched than the file size for a projection), JSON, and a
      redirect.
- [ ] Error tests for 404, 403, timeout and a URL glob.
- [ ] A test that a URL source is rejected when `allow_remote` is off (an engine test; the `ds ui`
      API test belongs to whichever of this task and M4-01 lands second).

## Follow-ups
