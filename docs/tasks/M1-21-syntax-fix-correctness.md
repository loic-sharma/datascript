# M1-21 — Syntax suggestions that produce valid code

**Status:** done — `===`/`= =` are one DS0110 with one fix; word fixes pad against tokens other fixes turn into words; `SELECT` → `project` is certain only for a plain column list (`| SELECT *` is removed instead); `SELECT … FROM` after a pipe gets the SQL rewrite help; DS0106 after a list operator offers `,` and `;`, neither certain (D-035).
**Depends on:** M1-18
**Crates:** `datascript-syntax` (`parser/targeted.rs`), `datascript-tests`
**Diagnostic codes:** DS0102–DS0149 (D-018)

## Goal
Every `MachineApplicable` fix from the parser produces code that parses and means what the user
meant, also when several fixes are applied together. Found in a review of M1-03. (Lexer-level
fixes are in M1-20.) Runs after M1-18, which also edits `targeted.rs`.

## Scope
- **`===` and `= =`.** `where a === 1` gives 3 errors, and the `==` fix gives `a ==== 1`.
  `where a = = 1` gives 4 errors, and the fix gives `a == == 1`. Give one diagnostic ("`===` is not
  an operator, use `==`") with one correct fix (`foreign_binary_operator` and the rhs handling).
- **Fixes that collide.** In `where a!=1&&!b`, the `&&` fix is `" and"` and the `!` fix is
  `not(`…`)`, so together they give `andnot(b)`. `word_edit` adds a trailing space only when the
  next token is word-like; it should also when another fix will make it word-like (or always pad).
- **`SELECT` → `project` is not certain.** `| SELECT *`, `| SELECT DISTINCT a` and
  `| SELECT a FROM t` still fail after the fix. Make it `MaybeIncorrect` unless the rest is a plain
  column list. Recognise `SELECT … FROM` after a pipe as a whole SQL query (it gets a bare DS0101
  "found `FROM`" today) and give the same rewrite help as at the start of a query.
- **DS0106 changes meaning.** `| project a⏎  b` gets a certain `;` fix, which makes `b` a separate
  query. The user likely forgot a comma. Make it `MaybeIncorrect` and mention `,` when the previous
  operator takes a list.

## Acceptance criteria
- [x] Harness: `mode: syntax` applies all machine-applicable fixes together (not one at a time)
      and re-parses, so collisions show up in golden output.
- [x] Golden cases: `===`, `= =`, `a!=1&&!b`, `| SELECT *`, `| SELECT DISTINCT a`,
      `| SELECT a FROM t`, the DS0106 list-continuation case.

## Carried over

## Follow-ups
- The harness already applied every certain fix in one pass (M1-03); it now also orders a
  replacement before an insertion at the same offset, so the result no longer depends on the order
  diagnostics are reported in.
- New cases: `errors/syntax/ds0110_triple_equals`, `ds0110_spaced_double_equals`,
  `ds0111_fixes_combined`, `ds0111_fixes_combined_bang_first`, `ds0105_select_star_after_pipe`,
  `ds0105_select_star_no_table`, `ds0105_select_distinct_after_pipe`,
  `ds0105_select_columns_after_pipe`, `ds0105_select_from_after_pipe`,
  `ds0105_select_from_after_operator`, `ds0106_list_continuation`,
  `ds0106_after_non_list_operator`.
