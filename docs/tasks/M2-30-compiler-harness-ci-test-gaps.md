# M2-30 — Compiler, diagnostics, golden harness, and CI test gaps

**Status:** todo
**Depends on:** M2-15
**Crates:** `datascript-compiler`, `datascript-diagnostics`, `datascript-syntax`, `datascript-tests` (plus `.github/workflows/ci.yml` and `docs/TESTING.md`)
**Diagnostic codes:** none new expected (tests and fixes for existing codes only; D-018).

## Goal
An audit of the compiler front-end, diagnostics layer, golden test harness, and CI (November 2026) uncovered six latent bugs hidden by missing integration tests and several structural blind spots in `datascript-tests`, `datascript-compiler`, `datascript-syntax`, `datascript-diagnostics`, and `.github/workflows/ci.yml`. Fix those bugs, harden the golden harness (including automated diagnostic-code and compiler-fix verification), and close the targeted test and CI gaps.

## Scope

### Verified bugs to fix
- [ ] Wrap did-you-mean `Suggestion::replace` edits with `datascript_syntax::display_name(best)` so suggestions for spaced or reserved names produce valid syntax (matching `binder/render.rs:686`):
  - `crates/datascript-compiler/src/binder.rs:1247` (`Binder::unknown_column`) and `binder.rs:1276` (`Binder::unknown_name`)
  - `crates/datascript-compiler/src/binder/join.rs:269` (`Binder::missing_key`)
  - `crates/datascript-compiler/src/binder/ops.rs:664` (`Binder::missing_column`)
  - `crates/datascript-compiler/src/binder/sources.rs:760` (`Binder::unknown_table`)
- [ ] Preserve `Column::arrow_only` in `binder::schema_of` (`crates/datascript-compiler/src/binder.rs:1473-1480`), matching `plan::schema_of` (`crates/datascript-compiler/src/plan.rs:766-776`) and `Doc::let_kind` (`crates/datascript-compiler/src/ide/mod.rs:218-221`), so `Analysis::stage_schemas` keeps the real Arrow `DataType` instead of `DataType::Null`.
- [ ] Fix `ide::signature_help` parameter matching (`crates/datascript-compiler/src/ide/signature.rs:108-113`): looking up `active_parameter` by parameter name gets stuck at `Some(0)` for `coalesce` (`crates/datascript-compiler/src/builtins.rs:226-231`), which repeats parameter name `"value"`. Match by parameter index/position rather than first matching name.
- [ ] Fix `$right.` completion inside `join ... on` (`crates/datascript-compiler/src/ide/complete.rs:680-694`): completing after `$right.` currently offers left-side columns and scalar functions instead of the right table's columns (and `$left.` should offer left-side columns).
- [ ] Fix wide-character window slicing alignment in `crates/datascript-diagnostics/src/render.rs:525-538` (`slice_columns` at line 532 drops a 2-column character straddling `window.start`, shifting subsequent characters left by 1 column relative to `Window::column` and misaligning carets).
- [ ] Fix `DS_BLESS=0` rewriting `editors/vscode/test/grammar/tokens.json` in `crates/datascript-syntax/tests/textmate_tokens.rs:94`: parse `DS_BLESS` the same way as `crates/datascript-tests/src/runner.rs:84` (treating `""` and `"0"` as off instead of `var_os("DS_BLESS").is_some()`).

### Golden harness hardening (`crates/datascript-tests`)
- [ ] Verify `MachineApplicable` suggestions in `PipelineDriver::check` (`crates/datascript-tests/src/pipeline.rs:138-160`), mirroring `PipelineDriver::syntax` (`pipeline.rs:116-134`): apply `MachineApplicable` compiler/lint suggestions with `apply_suggestions` and re-run `check`, verifying the fixed query parses cleanly and eliminates the target diagnostic (covering all 18 `MachineApplicable` sites across `datascript-compiler`).
- [ ] Add an automated diagnostic-code coverage matrix test in `crates/datascript-tests` that scans `tests/cases/**/*.dstest` and asserts every `CodeStatus::Active` code in `datascript_diagnostics::codes::REGISTRY` (`crates/datascript-diagnostics/src/codes.rs:228-769`) — minus an explicit, justified allowlist (`DS4002`, `DS5001`, `DS9001`, and `DS3007` once [M2-29](M2-29-engine-datafusion-integration-tests.md) promotes it to `Active` since sandbox tests live in `crates/datascript-datafusion/tests/integration.rs`) — appears in at least one golden case.
- [ ] Restrict `Order::Any` `canonicalize` to `mode: run` (`crates/datascript-tests/src/runner.rs:247-307`) so non-`run` outputs containing dash separator lines cannot be corrupted.
- [ ] Fix `sort_table_rows` (`crates/datascript-tests/src/output.rs:81-110`, line 101) so an empty-string cell in a 1-column table (rendered as a blank line after `format_table` trims trailing spaces at `crates/datascript-engine/src/table.rs:62`) or a data cell starting with `"render:"` does not prematurely terminate row sorting.
- [ ] Fail fast instead of silently swallowing errors or conflicting headers in the harness:
  - Propagate `fs::read_dir` and entry I/O errors in `discover()` (`crates/datascript-tests/src/runner.rs:90-93`) so a missing or unreadable `tests/cases/` directory fails rather than exiting 0 with 0 trials.
  - Reject conflicting or irrelevant `.dstest` headers (e.g., `schema:` combined with `tables:` in `check`/`explain`, or `schema:`/`tables:`/`now:` in `syntax`; `pipeline.rs:138-188`).
- [ ] Fix Windows nested-path normalization in `normalize_paths` (`crates/datascript-tests/src/output.rs:56-59`) so all `\` separators inside rewritten `<test>/...` and `<repo>/...` paths are normalized to `/`, not only the first separator.
- [ ] *(Optional while touching `datascript-tests`; otherwise defer to M6-03)* Patch the multi-engine `DS_BLESS` shadowing bug in `crates/datascript-tests/src/bless.rs:24-59,84-135` (where a stale `---- expected[<first_engine>]` shadows a newly appended `---- expected`).

### Targeted compiler, syntax, and diagnostics tests
- [ ] Extend `crates/datascript-syntax/tests/parser_props.rs:22-28` to validate `d.secondary` spans and `d.help[].suggestion.edits` char boundaries and non-overlap (matching `crates/datascript-syntax/tests/lexer_props.rs:41-48`), and verify that applying `MachineApplicable` suggestions on arbitrary/non-ASCII inputs never panics.
- [ ] Add cursor-marker (`‸`) matrix tests in `crates/datascript-compiler/src/ide/tests.rs` (and `binder_tests.rs` for `stage_schemas`) covering:
  - Arrow-only columns (e.g. `List(Int64)`) in `Analysis::stage_schemas`, `ide::complete`, and `ide::hover` (both in pipelines and tabular `let`s).
  - Multi-signature builtins (`sum`, `sumif`, `avg`, `percentile`, `bin`, `abs`) and variadic repeated-name `coalesce` in `ide::signature_help` (`ide/signature.rs:65-74,100-149`).
  - `$left.` and `$right.` completion inside `join ... on` (`ide/complete.rs:680-694`), `QUOTED_IDENT` replacement (`complete.rs:237-245`), unknown render kind fallback `RenderProps(None)` (`complete.rs:349`), and mid-UTF-8-char offset clamping (`complete.rs:112,152-157`).
  - `ide::hover` on `SyntaxKind::RENDER_PROP`, `SyntaxKind::COLUMN_PATTERN` (`project-away`), scalar-`let` file paths used as `from` sources, `join`/`union` right-hand tables, and `columns_list` truncation past 50 columns (`ide/hover.rs:79-86,101-109,129-135,151-160`).
- [ ] Add 3–4 targeted recovery boundary `.dstest` cases under `tests/cases/errors/recovery/` (redefined `let` `DS1003` preserving the first binding for later statements, mixed syntax + semantic + lint diagnostics in one program, and mid-pipeline broken operator preserving downstream schema recovery) plus a unit/integration test in `crates/datascript-compiler` for the public `program_of` pending-source `DS9001` guard (`crates/datascript-compiler/src/lib.rs:125-134`).
- [ ] Add `datascript-diagnostics` JSON and renderer tests (`crates/datascript-diagnostics/src/json.rs` and `crates/datascript-diagnostics/tests/render.rs`):
  - `JsonError::InvalidOffset` (mid-surrogate-pair UTF-16 offset into `😀` and past EOF; `json.rs:150-155,352-356`), `JsonError::OverlappingEdits` (`json.rs:157,286`), `primary: None` with `file: None` and unknown file (`json.rs:296-303`), multi-line `JsonSpan` with surrogate pairs across lines, and `Applicability::HasPlaceholders` serialization.
  - `Renderer::terminal(true)` ANSI colors for `Severity::Warning` and `Severity::Info` (`render.rs:649-650`), 2-line gap printing the intervening source line (`render.rs:322-327`), wide-char long-line window slicing (`render.rs:468-496,525-538`), `primary: None` with `secondary` labels sorted by `FileId` (`render.rs:191-198`), multi-line `help`/`note` indentation (`render.rs:155-166`), and `Diagnostics::finish` location-less (`file: None`) error capping (`collection.rs:103`).

### CI and documentation
- [ ] Update `.github/workflows/ci.yml`:
  - Add `-p datascript-engine` to the `wasm` job (`ci.yml:79-90`).
  - Run `cargo test -p datascript-datafusion --no-default-features` to verify the non-`compression` build natively.
- [ ] Update `docs/TESTING.md`:
  - Add the missing test layers to the "Test layers" table (`TESTING.md:9-17`): CLI (`tests/cli.rs`), REPL (`tests/repl.rs`), stdio LSP (`tests/lsp_run.rs`), lexer/parser proptests, diagnostic snapshots, and fixture tests.
  - Align the `show: schema` description (`TESTING.md:67`) with `query_schemas` (`crates/datascript-tests/src/pipeline.rs:214-236`), documenting the `query N columns:` headers and `(error)` type rendering.

### Out of scope
- Exhaustive 28-code recovery-pair matrix across every active diagnostic code (covered selectively via the 3–4 boundary recovery cases above).
- Direct tests for unreachable internal `plan.rs` `DS9001` guards (`plan.rs:98,107,236,596,753,756`).
- Per-statement `Order::Any` classification in `output_order_defined` (`pipeline.rs:242-264`).
- Multi-engine `bless.rs` test suite (belongs with the second backend in [M6-03](M6-03-sql-backend.md)).
- Cross-OS `clippy` matrix or MSRV (`1.94`) CI job (belongs with release/packaging in [M6-02](M6-02-packaging.md)).

## Acceptance criteria
- [ ] All six verified bugs above are fixed with regression tests that fail before the fix and pass after.
- [ ] `PipelineDriver::check` verifies `MachineApplicable` suggestions on golden `check` cases, and the diagnostic-code coverage matrix test passes against `codes::REGISTRY`.
- [ ] `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo test -p datascript-datafusion --no-default-features`, and `cargo check -p datascript-syntax -p datascript-compiler -p datascript-engine --target wasm32-unknown-unknown` all pass.
- [ ] `docs/TESTING.md` matches the actual test layers and `show: schema` output format.

## Follow-ups
- Multi-engine `bless.rs` test coverage and (if not patched inline) the stale `expected[<first_engine>]` shadowing fix belong to [M6-03](M6-03-sql-backend.md).
- Cross-OS `clippy` and MSRV CI checks belong to [M6-02](M6-02-packaging.md).
