# Spike: DataFusion on wasm32 (M0-05)

**Date:** 2026-11-20 · **Verdict:** DataFusion works on `wasm32-unknown-unknown`. The engine
module is **5.8 MB gzipped / 3.8 MB brotli**, well under the ~15 MB limit. The plan does not change.

Code: [`spikes/datafusion-wasm/`](../../spikes/datafusion-wasm/). It is throwaway, not a workspace
member, and not maintained after M0-05.

## What was tested

| Check | Native | wasm32 (Node 26) | wasm32 (Chromium 156) |
|---|---|---|---|
| `count(*)`, `sum(amount)` over `tests/data/orders.csv` from an in-memory `ObjectStore` | ✅ | ✅ | ✅ |
| Same over `tests/data/orders.parquet` | ✅ | ✅ | ✅ |
| Parquet → CSV join, group by, sort desc | ✅ | ✅ | ✅ |
| Parquet re-encoded with snappy, gzip, lz4, zstd, brotli, then read back | ✅ | ✅ | — |

- Queries use the DataFrame API, not SQL, because DataScript lowers to DataFusion plans directly.
- Files are `put` into `object_store::memory::InMemory`, registered at `memory://`, and read as
  `memory:///orders.csv`.
- Session: `SessionConfig::new().with_target_partitions(1)`.
- Node: `wasm-pack test --node [--release] -- --test queries` (`wasm-bindgen-test`).
- Browser: the `--target web` build loaded by `spikes/datafusion-wasm/www/index.html` in the Claude
  desktop app's built-in Chromium. Results matched native. Init took 49 ms, the three queries 78 ms.
  Headless Chrome through `wasm-pack test --headless --chrome` was **not** run, because there was
  no chromedriver on the machine. M5-01 should set that up in CI.
- None of these queries needed a tokio runtime. `collect()` is driven by `wasm-bindgen-futures`
  and nothing spawned a task with one partition. Operators that spawn tasks when there is more than
  one partition (repartition, parallel Parquet writing) were not exercised. Keep
  `target_partitions = 1` on wasm.

## Versions and toolchain

- DataFusion **55.3.0** (arrow / parquet **59.3**, object_store **0.13.2**, not 0.14). DataFusion
  55's MSRV is **Rust 1.94**, but the workspace declares `rust-version = "1.85"`. The task that
  adds DataFusion to the workspace (M1-15) must raise it.
- rustc 1.100.0 · wasm-pack 0.15.1 · wasm-bindgen 0.2.130 · binaryen (wasm-opt) 133 · Homebrew
  LLVM/clang 23.

## Feature set for M1-08

```toml
# datascript-datafusion/Cargo.toml
datafusion = { version = "55.3", default-features = false, features = ["parquet"] }
# Native-only, optional: compressed CSV/JSON (.gz/.bz2/.xz/.zst). It also builds and links for
# wasm32 (+0.1 MB gzipped), but decoding was not exercised at runtime there.
#   features = ["parquet", "compression"]
# M2 (make_list, split, mv-expand): add "nested_expressions" (+0.2 MB gzipped).

[target.'cfg(target_arch = "wasm32")'.dependencies]
getrandom_03 = { package = "getrandom", version = "0.3", features = ["wasm_js"] }
chrono = { version = "0.4", default-features = false, features = ["clock", "wasmbind"] }
```

- **The function features do nothing for size.** `datafusion` depends on `datafusion-functions`
  with its default features, so string, unicode, datetime, regex, math and encoding functions are
  always compiled in, whatever `datafusion`'s `*_expressions` features say. Only
  `nested_expressions` (a separate crate), `crypto_expressions` and `sql` change the binary.
- **`sql` is not needed.** DataScript builds plans itself, and SQL adds 0.3 MB gzipped.
- **All Parquet codecs are on.** `parquet` comes in with its default features through
  `datafusion-common`, so snappy, gzip, lz4, zstd and brotli all read on wasm32.
- The `wasm` cargo feature that M1-08 adds can be thin. The wasm-only crates above are
  target-gated dependencies, so the feature mostly turns native-only options off (such as
  `compression` if we decide to keep it off on wasm). The CI command in M1-08's task file stays as
  written.

## Workarounds needed on wasm32

1. **zstd-sys builds C.** `arrow` is pulled in with its default features (DataFusion depends on it
   that way), which enables `arrow-ipc/zstd`, and `parquet/zstd` needs it too. Apple's clang has
   no wasm32 backend, so the build fails with *"No available targets are compatible with triple
   wasm32-unknown-unknown"*. Fix: point `cc` at a clang that has one. zstd-sys ships its own
   wasm libc shim, so nothing else is needed:
   ```toml
   # .cargo/config.toml
   [env]
   CC_wasm32_unknown_unknown = "/opt/homebrew/opt/llvm/bin/clang"   # brew install llvm
   AR_wasm32_unknown_unknown = "/opt/homebrew/opt/llvm/bin/llvm-ar"
   ```
   On Ubuntu CI runners, the stock `clang` (with `llvm-ar`) works. Set the same variables, or use
   `CC_wasm32_unknown_unknown=clang`. The final module imports nothing except wasm-bindgen's JS
   glue (no `env` imports).
2. **getrandom.** `ahash` (via arrow) pulls in getrandom 0.3, which needs the `wasm_js` feature
   *and* the cfg below. getrandom 0.4 (via tempfile) is not built for wasm32.
   ```toml
   # .cargo/config.toml
   [target.wasm32-unknown-unknown]
   rustflags = ['--cfg', 'getrandom_backend="wasm_js"']
   ```
   Because this is a rustflag, the repo's `.cargo/config.toml` (or CI env) needs it. Rustflags
   only apply to the wasm32 target, so they do not affect the core crates' native builds.
3. **`SessionContext::new*` panics** with *"time not implemented on this platform"*. It calls
   `chrono::Utc::now()`, which is `SystemTime::now()` unless chrono's `wasmbind` feature is on
   (that feature reads JS `Date`). DataFusion's own `now()` also comes from this clock
   (`query_execution_start_time`). D-015 needs an injected clock anyway, so M1-09 should lower
   `now()`/`ago()` to a literal from `ExecContext`'s clock and never use DataFusion's `now()`.

No patches to DataFusion were needed.

**tokio is still pulled in** on wasm32 (features `rt`, `sync`, `time`, `macros`, `io-util`, `fs`,
`bytes`). It compiles and causes no trouble while nothing spawns. `tempfile` and the disk manager
are compiled in too. M5-01 should turn spilling off on wasm (`DiskManagerMode::Disabled`) so a
memory limit shows a clean error instead of a filesystem error.

## Size

`wasm-pack build --release --target web` with `opt-level = "z"`, `lto = true`,
`codegen-units = 1`, `panic = "abort"`, `strip = true`; then `wasm-opt -Oz`, `gzip -9`,
`brotli -q 11`. Measured with `spikes/datafusion-wasm/scripts/size.sh`. MB = 10⁶ bytes.

| Variant | Raw | `wasm-opt -Oz` | gzip -9 | brotli |
|---|---|---|---|---|
| CSV only (no `parquet`) | 20.4 MB | 17.5 MB | 4.73 MB | 2.99 MB |
| **`parquet`** (recommended) | 23.9 MB | 20.6 MB | **5.83 MB** | **3.77 MB** |
| `parquet` + `compression` | 24.1 MB | 20.8 MB | 5.92 MB | 3.83 MB |
| `parquet` + `nested_expressions` | 24.9 MB | 21.4 MB | 6.02 MB | 3.88 MB |
| `parquet` + `nested_expressions` + `sql` | 25.7 MB | 22.1 MB | 6.30 MB | 4.08 MB |

- `wasm-opt -Oz` saves only about 14% on top of `opt-level = "z"` + LTO.
- Most of the size is DataFusion's core. Parquet adds about 1.1 MB gzipped. That fits the
  two-module design (ARCHITECTURE.md § WASM constraints): the compiler module loads first, and
  this one loads lazily.
- Serve with brotli where the host allows it. The DataScript engine adds to these numbers.
  M5-01 tracks the real size.

## timespan as `Duration(µs)` (for M1-09)

Probed natively with `cargo run --example probes`. Columns are `Timestamp(µs, "UTC")`,
`Duration(µs)` and `Int64`.

| LANGUAGE.md §4.1 row | DataFusion 55 result | M1-09 action |
|---|---|---|
| `datetime - datetime` | ✅ `Duration(µs)` | none |
| `datetime + timespan`, `timespan + datetime`, `datetime - timespan` | ✅ `Timestamp(µs, "UTC")` | none |
| `timespan + timespan`, `timespan - timespan` | ✅ `Duration(µs)` | none |
| `-timespan` | ❌ planning error: *"Negation only supports numeric, interval and timestamp types"* | lower as `0µs - a`, or a UDF |
| `timespan * num`, `num * timespan`, `timespan / num` | ❌ *"Cannot coerce arithmetic expression Duration(µs) * Int64"* (also Float64) | UDF, or cast to Int64 µs → arithmetic → cast back to `Duration(µs)` (Int64 → Duration cast works and keeps the value as µs). For `real` factors, round to µs. |
| `timespan / timespan` → `real` | ❌ coerced to `Interval(MonthDayNano)`, then *"Invalid interval arithmetic operation"* | cast both to Int64, then `Float64` division |
| `<` `<=` `>` `>=` `==` on timespan | ✅ `Boolean` | none |
| `sum`, `avg`, `min`, `max` over a duration column | ✅ all return `Duration(µs)` (avg truncates to µs) | none. A golden case for each |
| `bin(datetime, timespan)` | `date_bin` takes no `Duration`, only `Interval`; with an Interval it returns **`Timestamp(ns, "UTC")`** | UDF, or cast the size to `Interval(MonthDayNano)` and cast the result back to `Timestamp(µs, "UTC")` |

No probed operation returned `Interval` to the user. `Interval` only shows up inside coercion, as
in the `timespan / timespan` error. The ns-precision result of `date_bin` is the one silent type
change to watch.

## Checked integer arithmetic (for M1-09, D-014)

| Expression (`Int64`) | DataFusion 55 default | Meets D-014? |
|---|---|---|
| `a / 0`, `a % 0` (column or literal; constant-folded too) | error *"Arrow error: Divide by zero error"* | ✅ (map to DS4003) |
| `i64::MIN / -1` | error *"Arithmetic overflow: Overflow happened on: -9223372036854775808 / -1"* | ✅ |
| `-7 / 2`, `-7 % 2` | `-3`, `-1` (truncates toward zero) | ✅ |
| `abs(i64::MIN)` | error *"Int64Array overflow on abs(...)"* | ✅ |
| `i64::MAX + 1`, `i64::MIN - 1`, `i64::MAX * 3` | **wraps silently** | ❌ |
| `-i64::MIN` | **wraps** (returns `i64::MIN`) | ❌ |
| `i64::MAX + 1` with both literals (constant-folded) | **wraps** | ❌ |
| `sum(long)` overflow | **wraps** | ❌ |

- There is **no session option** for this. `BinaryExpr::with_fail_on_overflow(true)` exists in
  `datafusion-physical-expr` and switches `+ - *` to arrow's checked kernels, but the default
  physical planner always builds `fail_on_overflow: false`. `ExecutionPlan` has no generic way to
  rewrite expressions after planning. `datafusion.execution.enable_ansi_mode` only affects the
  Spark function crate.
- **Recommendation:** lower integer `+`, `-`, `*` and unary `-` (when both sides are integer
  typed, which the binder knows) to Immutable scalar UDFs such as `ds_checked_add`. They call
  `arrow::compute::kernels::numeric::{add, sub, mul, neg}`, the checked variants that return
  `ArrowError::ArithmeticOverflow`. The spike's `ds_checked_add` errors on overflow for both
  column and literal inputs. Constant folding still runs it, so the error happens at plan time for
  literals. M1-09 must map that to DS4003 too, and it gets no row span. Real arithmetic stays on
  DataFusion's operators.
- `sum(long)` needs a checked UDAF (`ds_sum`), or a decision recorded in Q-008 to accept wrapping
  there.
- Plain `/` and `%` can stay as DataFusion operators. Their errors are already the right kind.

## Other notes for M1

- `col("customerId")` parses its argument as a SQL identifier and **lowercases** it. Lower column
  references with `Expr::Column(Column::new(qualifier, name))` (or `ident(name)`), never `col()`.
- Every reader names its table `?table?`, so joining two unaliased DataFrames on a same-named key
  fails with *"FieldNotFound"*. Alias each side (`DataFrame::alias`) and join on qualified columns.
- An `InMemory` store registered at `memory://` is enough for the browser. M5 can map uploaded
  files to `memory:///<name>`.
