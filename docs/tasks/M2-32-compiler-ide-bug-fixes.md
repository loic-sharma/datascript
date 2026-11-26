# M2-32 — Compiler, IDE and diagnostics bug fixes from the test audit

**Status:** todo
**Depends on:** —
**Crates:** `datascript-compiler`, `datascript-diagnostics`, `datascript-syntax`
**Diagnostic codes:** none (fixes to existing behaviour only; D-018).

## Goal
The November 2026 integration test audit found six bugs in the compiler front-end, the IDE
features and the diagnostics renderer. Each is small and user-visible. Fix each one with a
regression test that fails before the fix.

## Scope
- [ ] **Did-you-mean edits produce invalid syntax for names that need quoting.** The
      `Suggestion::replace(span, best)` calls in `Binder::unknown_column`, `Binder::unknown_name`
      (`binder.rs`), `Binder::missing_key` (`binder/join.rs`), `Binder::missing_column`
      (`binder/ops.rs`) and `Binder::unknown_table` (`binder/sources.rs`) insert the bare name.
      Wrap it with `datascript_syntax::display_name(best)`, as `binder/render.rs` already does.
      Test with a column whose name has a space and one that is a reserved word.
- [ ] **`binder::schema_of` drops Arrow-only types.** It maps every column through `to_arrow`, so
      `Analysis::stage_schemas` stores `DataType::Null` for `Column::arrow_only` columns. Preserve
      the real type, as `plan::schema_of` and `Doc::let_kind` do. Test with a `List(Int64)` column
      in `stage_schemas`, completion and hover.
- [ ] **Signature help stays on the first parameter for `coalesce`.** `ide::signature::info`
      finds `active_parameter` by parameter *name*, and every `coalesce` parameter is named
      `value`. Match by position. Test `coalesce` and one or two other multi-signature builtins
      (`sumif`, `percentile`).
- [ ] **`$right.` inside `join ... on` completes left-side columns.** `ide::complete` passes
      `doc.input_of(op)` for the whole `on` expression. After `$right.` offer the right table's
      columns; after `$left.` the left side's.
- [ ] **Wide characters shift carets in long-line windows.** `slice_columns` in
      `datascript-diagnostics/src/render.rs` drops a 2-column character that straddles
      `window.start` without padding, so everything after it moves one column left of its caret.
      Pad with a space. Add a snapshot in `tests/render.rs`.
- [ ] **`DS_BLESS=0` rewrites `editors/vscode/test/grammar/tokens.json`.**
      `crates/datascript-syntax/tests/textmate_tokens.rs` checks `var_os("DS_BLESS").is_some()`.
      Treat `""` and `"0"` as off, like `datascript-tests`' runner.

## Acceptance criteria
- [ ] Every bug above has a regression test that fails before the fix and passes after.
- [ ] `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`,
      `cargo test --workspace`, and
      `cargo check -p datascript-syntax -p datascript-compiler --target wasm32-unknown-unknown`
      pass.

## Follow-ups
