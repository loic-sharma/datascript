# M2-01 — Remaining tabular operators

**Status:** todo
**Depends on:** M1-13, M2-13
**Crates:** `datascript-syntax`, `datascript-compiler`, `datascript-datafusion`
**Diagnostic codes:** DS0200–DS0219 (syntax), DS1050–DS1059 (names), DS2200–DS2219 (types) (D-018)

## Goal
Add the simpler ○ operators and sources from LANGUAGE.md: `lookup`, `as`, `sample`,
`project-reorder`, `datatable`, `range`, `print`.

## Acceptance criteria
- [ ] Parser, binder, IR (extend the IR only if desugaring isn't possible), and lowering for each.
- [ ] Golden happy-path and error cases per operator.
- [ ] LANGUAGE.md marks them ●.

## Carried over
- [ ] (from M1-02) These operators currently parse as an unknown-operator error with a "planned" note; replace that with real parsing.
- [ ] (from M1-24) Generators start a query without `from` (D-036). The parser already sends `range`, `datatable` and `print` at a pipeline head to `grammar::generator` (via `Parser::at_generator`), which reports DS0104 "not available yet" and skips them; replace that with their grammar, so a generator is a `PIPELINE` whose first child is the generator node. Update `tests/cases/syntax/generator_at_head.dstest`. `range(1, 5)` with no space must stay a call.
- [ ] (from M1-25) `range x⏎from 1 to 10` must stay one statement (D-037): after `range x` the generator is not complete, so `from` there is not a statement start. Today `grammar::generator` recovers with `Parser::at_new_statement`, which stops at a `from` first on its line; the real grammar must not. Add a golden case once `range` parses.
- [ ] (from M1-25) `lookup from t on k` should get DS0152 with the fix removing `from`, as `join` and `union` do: call `grammar::right_table` for the table after `lookup`.
- [ ] (from M3-02) Teach completion the new operators in `datascript_compiler::ide::complete` (`classify`): the right side of `lookup` is a table position like `join`'s, and each operator's clauses need their keywords and column contexts. Add a case per operator to `crates/datascript-compiler/src/ide/tests.rs`, and an `operator_doc` line each (`ide/mod.rs`).
- [ ] (from M2-13) Add the new operators (`lookup`, `as`, `sample`, `project-reorder`) and generators (`datatable`, `range`, `print`) to the TextMate grammar in `editors/vscode/syntaxes/datascript.tmLanguage.json`; the Rust test that checks the grammar against the lexer (D-053) should cover them.
- [ ] (from M2-19) Accept optional trailing commas in every new comma-separated list (`datatable(...) [...]` schema and value lists, `print` items, `project-reorder` columns, `lookup ... on` keys) by parsing them with `list()` (or matching terminator logic).
- [ ] (from KQL review, 2026-11) Add `union withsource = <col>`: a column naming which input each row
      came from. For file sources, decide whether it holds the table name, the path, or the file a
      glob matched (useful for `from 'logs/*.parquet' | union …`), and record that in DECISIONS.md.
      Note that returning per-file glob names affects `Scan`/lowering in
      `crates/datascript-datafusion/src/lower.rs` (which merges uniform-schema globs into one
      multi-path `ListingTable`), not just `Union`. Also cover `withsource` in completion
      (`ide/complete.rs`) and the TextMate grammar (`datascript.tmLanguage.json`, alongside
      `kind(?=\s*=)`). If this is too much for M2-01, move it to its own task.

## Follow-ups
