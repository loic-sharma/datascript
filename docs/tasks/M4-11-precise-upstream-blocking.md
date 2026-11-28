# M4-11 — Notebooks: block a cell run only on the upstream `let`s it uses

**Status:** todo
**Depends on:** M4-08, M4-10
**Crates:** `datascript-compiler`, `datascript-server`
**Diagnostic codes:** none (reuses `DS1085` from M4-08's block, D-018)

## Goal
In M4 v1, any broken upstream `let` blocks every cell below it with `DS1085`, even cells that
don't use it ([WEB_UI.md §3](../WEB_UI.md#upstream-let-prepending--ds1085), D-064). Make a run
block only when the cell actually uses a broken `let`, directly or through other `let`s, without
changing what live check reports.

## Background
An earlier draft of the PR #25 spec tried to specify this up front and kept finding binder-level
edge cases. Design it with the binder in hand and settle each one in DECISIONS.md:
- **Lookup order differs by position**: in row expressions with known columns a column wins over a
  `let`; under unknown columns, in table positions, and in constant positions (`take n`, render
  properties via `const_value_of`) the `let` wins. A column named like a broken `let` must not
  block; a constant or table use must.
- **Indirect use**: a valid `let b` that uses broken `a` has no diagnostic of its own (D-038).
  Today a reference to a broken `let` binds to a bare error expression with no name, so the binder
  needs to record which `let` it resolved to (or a "poisoned" marker carrying the cause) for
  `DS1085` to name the chain.
- **Duplicates**: with `let a` in two upstream cells, live check binds the first and reports
  `DS1003` on the second. Decide whether using `a` blocks.
- **Spans**: anything referenced but left out of the compiled program has spans that cannot be
  mapped back to cells; keep every declaration in the program, or carry pre-mapped spans.
- **Parity**: the server and `datascript-wasm` (M5-02) must share one implementation in
  `datascript-compiler`.

## Acceptance criteria
- [ ] A broken upstream `let` the target does not use no longer blocks the run; one it uses
      directly, through a chain, as a table, or as a `take n` constant still gives one `DS1085`
      (naming the chain when indirect); a column named like a broken `let` does not block.
- [ ] Live check output is unchanged, and the M4-08 and M4-10 `DS1085` tests (and M5-02's, if
      done) are updated to the new rule.
- [ ] WEB_UI.md §3, LANGUAGE.md §2.1 and D-064 describe the new rule.

## Follow-ups
