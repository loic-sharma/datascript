# M2-36 — `mv-expand` operator

**Status:** todo
**Depends on:** M2-03
**Crates:** `datascript-syntax`, `datascript-compiler`, `datascript-datafusion`, `datascript-tests`
**Diagnostic codes:** DS0237–DS0239 (syntax), DS2245–DS2249 (types) (D-018)

## Goal
`mv-expand`: one output row per element of a `dynamic` array (or per key–value pair of an object).

Split from the original M2-03 (2026-11-28).

## Scope
Write the syntax and behaviour in LANGUAGE.md §3 (the operator table and a subsection) before
implementing, and record anything that differs from KQL in DECISIONS.md:
- **Syntax.** `mv-expand col`, `mv-expand name = expr`, several columns in one operator
  (expanded in parallel, as in KQL, with shorter arrays padded with null), trailing commas
  (D-059). Decide which KQL options to support now and which give a tailored "not supported"
  error: `with_itemindex = name`, `to typeof(type)`, `limit n`, `bagexpansion = bag | array`.
- **Output schema.** The expanded column keeps its name and position; its type is `dynamic` unless
  `to typeof(...)` is supported and given.
- **Edge cases.** What happens to rows where the value is an empty array, null, a scalar, or an
  object. Check what KQL does for each (with the default `bagexpansion = bag`, an object expands
  to one row per key–value pair, each a one-entry bag), decide, then write it down.
- **Errors.** `mv-expand` on a column that is not `dynamic` (suggest `todynamic`/`parse_json`, or
  `split` for strings), an expression that is not `dynamic`, and an unknown option.
- **Plan and lowering.** A new logical plan node (shown in `explain`) and its DataFusion lowering
  (`unnest` may fit if M2-03 stores arrays natively; with JSON text it needs a UDTF or a parse step
  first).

## Acceptance criteria
- [ ] Syntax and every edge case above are in LANGUAGE.md.
- [ ] Golden cases: one column, several columns of unequal length, an empty array, null, a scalar,
      an object, `explain` output, and each supported option.
- [ ] Golden error cases for each new diagnostic, with help where a fix is likely.
- [ ] `mv-expand` is removed from `LATER_OPERATORS`, and the lexer and parser property tests that
      list it (`lexer.rs`, `tests/parser_props.rs`) still pass.

## Carried over
- [ ] (from M1-02) `mv-expand` parses as a planned unknown operator (`LATER_OPERATORS` in `crates/datascript-syntax/src/parser/grammar.rs`). Replace that with real parsing and remove it from the list.
- [ ] (from M2-19) Accept optional trailing commas in `mv-expand` column lists by parsing them with `list()` (or matching terminator logic).

## Follow-ups
