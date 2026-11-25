# M7-03 — Python package

**Status:** todo
**Depends on:** M2-11, M2-13, M7-05
**Crates:** `datascript-python` (new), `python/`, `Cargo.toml` (workspace)
**Diagnostic codes:** none (diagnostics cross the boundary unchanged)

## Goal
`pip install datascript`, then:

```python
import datascript as ds
result = ds.run("from 'orders.parquet' | summarize revenue = sum(amount) by customerId")
result.df()      # pandas
result.pl()      # Polars
result.arrow()   # pyarrow.Table
ds.run("from orders | top 5 by amount desc", tables={"orders": my_dataframe})
```

## Scope
- PyO3 + maturin, binding the Rust crates directly (`datascript-engine`'s `Session`), not the C
  API (M7-04): PyO3 handles the GIL, ownership and exceptions.
- Results cross as Arrow with no copy (the Arrow PyCapsule interface, e.g. `pyo3-arrow`). `.df()`
  and `.pl()` convert lazily and import pandas or Polars only when called; neither is a required
  dependency.
- Host tables from any object exposing `__arrow_c_stream__` (pyarrow, Polars, pandas via pyarrow),
  registered through M7-05's in-memory host tables.
- A `Connection` object wrapping one `datascript_engine::Session` that keeps host tables between
  calls, plus top-level `run`/`check` helpers on a default connection. `let`s do **not** persist
  between calls, matching `Session` and the C API: only the REPL keeps them (`ReplSession` in
  `datascript-cli`, D-048). To reuse a result, register it as a table
  (`con.register("top", result.arrow())`).
- **One query or many.** `run(text)` returns one `Result` and raises if the script has more than
  one query statement, with help pointing at `run_all`. `run_all(text)` returns a list, one
  `Result` per query statement. No union return type. Streaming: iterate a result's batches
  without collecting (from `RunStream`, D-055).
- `check(text)` returns diagnostics as objects (code, severity, message, spans, help). A failed
  `run` raises `DataScriptError` carrying those diagnostics and the rendered text, so the error
  in a notebook looks like the CLI's.
- Release the GIL while a query runs. Python owns `SIGINT`, so Ctrl-C cannot be caught with
  `tokio::signal::ctrl_c()` as the REPL does: while the query runs, poll
  `Python::check_signals()` (re-acquiring the GIL briefly every ~100 ms), and on
  `KeyboardInterrupt` drop the stream to cancel the query (D-055) and re-raise.
- **`unsafe` lint.** The workspace sets `unsafe_code = "forbid"` (root `Cargo.toml`), and PyO3's
  macros expand to `unsafe` code. Give `datascript-python` its own lint settings instead of
  `[lints] workspace = true`, keeping `forbid` for every other crate, and say why in its
  `Cargo.toml`.
- Type stubs (`.pyi`), and Jupyter `_repr_html_` for results.
- **Workspace linking.** PyO3's `extension-module` feature breaks `cargo test --workspace` linking
  on macOS and Linux when it is on by default. Put it behind a cargo feature that only maturin
  enables (or keep `datascript-python` out of the workspace's default members), and say which in
  the crate's README.
- Wheels for macOS, Linux and Windows are built in CI. Publishing to PyPI is M6-02's (already in its
  "Carried over").

## Acceptance criteria
- [ ] pytest suite in CI: run, `run_all`, `run` on a script with two queries (raises), check, host
      tables from pandas, Polars and pyarrow, errors with diagnostics, and Ctrl-C cancellation.
- [ ] `maturin build` produces wheels for the three platforms in CI (dry run; nothing published).
- [ ] Docs page with the examples above.

## Follow-ups
