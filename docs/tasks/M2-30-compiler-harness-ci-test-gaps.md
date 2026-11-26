# M2-30 — Golden harness, CI and testing-docs gaps

**Status:** todo
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
- [ ] **Verify compiler fixes.** In `PipelineDriver::check`, apply `MachineApplicable`
      suggestions with `apply_suggestions`, re-run `check`, and require that the result parses
      and no longer has the diagnostic, as `PipelineDriver::syntax` already does.
- [ ] **Code coverage matrix.** A test that scans `tests/cases/**/*.dstest` and fails if a
      `CodeStatus::Active` code in `codes::REGISTRY` appears in none of them, except an explicit
      allowlist with a reason per code (`DS4002`, `DS5001`, `DS9001`).
- [ ] **Order and sorting.** Apply `Order::Any` canonicalization only to `mode: run`, so other
      outputs with dash lines are not reordered. Fix `sort_table_rows` so a blank cell in a
      1-column table, or a cell starting with `render:`, does not end the rows early.
- [ ] **Fail fast.** `discover()` propagates `read_dir` errors instead of passing with 0 trials,
      and conflicting or irrelevant headers are rejected (`schema:` with `tables:`;
      `schema:`/`tables:`/`now:` in `syntax` mode).
- [ ] **Windows paths.** `normalize_paths` turns every `\` in rewritten `<test>/…` and `<repo>/…`
      paths into `/`, not just the first.

### CI and docs
- [ ] CI runs `cargo test -p datascript-datafusion --no-default-features`, so the build without
      `compression` is tested natively.
- [ ] `docs/TESTING.md`: add the missing layers to the "Test layers" table (CLI, REPL, stdio LSP,
      lexer and parser proptests, diagnostic snapshots), and describe `show: schema` as
      `query_schemas` prints it (`query N columns:` headers, `(error)` types).

### Out of scope
- The multi-engine `bless.rs` shadowing bug and its tests ([M6-03](M6-03-sql-backend.md)).
- A cross-OS `clippy` matrix or MSRV job ([M6-02](M6-02-packaging.md)).
- Tests for unreachable internal `plan.rs` `DS9001` guards.

## Acceptance criteria
- [ ] `PipelineDriver::check` verifies `MachineApplicable` fixes on every golden `check` case, and
      the coverage matrix passes.
- [ ] Each harness fix has a unit test in `datascript-tests`.
- [ ] `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`,
      `cargo test --workspace` and `cargo test -p datascript-datafusion --no-default-features`
      pass.

## Follow-ups
