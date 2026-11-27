# M2-30 — Golden harness, CI and testing-docs gaps

**Status:** done — Verified `MachineApplicable` fixes in `PipelineDriver::check`, added an active diagnostic code coverage matrix test, scoped `Order::Any` to `mode: run` and fixed `sort_table_rows`, propagated `discover()` errors and rejected conflicting/irrelevant headers, normalized all backslashes in rewritten paths, added the `--no-default-features` `datascript-datafusion` CI step, and updated `docs/TESTING.md`.
**Depends on:** M2-32
**Crates:** `datascript-tests` (plus `.github/workflows/ci.yml` and `docs/TESTING.md`)
**Diagnostic codes:** none (D-018).

## Goal
The November 2026 audit found blind spots in the golden harness: compiler fixes are never applied
and re-checked, nothing enforces that active codes have golden coverage, and a few harness paths
silently pass or corrupt output. Close those, add one CI build, and bring
[TESTING.md](../TESTING.md) in line with the test layers that exist.

The compiler and IDE bugs found by the same audit are [M2-32](M2-32-compiler-ide-bug-fixes.md).
This task depends on it because verifying fixes in `check` would fail on its unquoted did-you-mean
edits.

## Scope

### Golden harness (`crates/datascript-tests`)
- [x] **Verify compiler fixes.** In `PipelineDriver::check`, apply `MachineApplicable`
      suggestions with `apply_suggestions` and re-run `check` on the result. Fail the case
      (`DriverError::Failed`) if the fixed query has parse errors or still has a diagnostic with
      the same code at the fixed location. Other diagnostics may remain (recovery cases have
      several errors). This is an assertion only: unlike `PipelineDriver::syntax`, it adds nothing
      to the expected output, so existing `mode: check` files do not change.
- [x] **Code coverage matrix.** A test that scans `tests/cases/**/*.dstest` and fails if a
      `CodeStatus::Active` code in `codes::REGISTRY` appears in none of them, except an explicit
      allowlist with a reason per code (`DS4002`, `DS5001`, `DS9001`).
- [x] **Order and sorting.** Apply `Order::Any` canonicalization only to `mode: run`, so other
      outputs with dash lines are not reordered. Fix `sort_table_rows` so a blank cell in a
      1-column table, or a cell starting with `render:`, does not end the rows early.
- [x] **Fail fast.** `discover()` propagates `read_dir` errors instead of passing with 0 trials,
      and conflicting or irrelevant headers are rejected (`schema:` with `tables:`;
      `schema:`/`tables:`/`now:` in `syntax` mode).
- [x] **Windows paths.** `normalize_paths` turns every `\` in rewritten `<test>/…` and `<repo>/…`
      paths into `/`, not just the first.

### CI and docs
- [x] CI runs `cargo test -p datascript-datafusion --no-default-features`, so the build without
      `compression` is tested natively.
- [x] `docs/TESTING.md`: add the missing layers to the "Test layers" table (CLI, REPL, stdio LSP,
      lexer and parser proptests, diagnostic snapshots), and describe `show: schema` as
      `query_schemas` prints it (`query N columns:` headers, `(error)` types).

### Out of scope
- The multi-engine `bless.rs` shadowing bug and its tests ([M6-03](M6-03-sql-backend.md)).
- A cross-OS `clippy` matrix or MSRV job ([M6-02](M6-02-packaging.md)).
- Tests for unreachable internal `plan.rs` `DS9001` guards.

## Acceptance criteria
- [x] `PipelineDriver::check` verifies `MachineApplicable` fixes on every golden `check` case, and
      the coverage matrix passes.
- [x] Each harness fix has a unit test in `datascript-tests`.
- [x] `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`,
      `cargo test --workspace` and `cargo test -p datascript-datafusion --no-default-features`
      pass.

## Follow-ups
