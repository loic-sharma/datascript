# M2-01 — Remaining tabular operators

**Status:** todo
**Depends on:** M1-13
**Crates:** `datascript-syntax`, `datascript-compiler`, `datascript-datafusion`

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

## Follow-ups
