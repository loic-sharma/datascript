# M1-21 — Syntax suggestions that produce valid code

**Status:** todo
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
- [ ] Harness: `mode: syntax` applies all machine-applicable fixes together (not one at a time)
      and re-parses, so collisions show up in golden output.
- [ ] Golden cases: `===`, `= =`, `a!=1&&!b`, `| SELECT *`, `| SELECT DISTINCT a`,
      `| SELECT a FROM t`, the DS0106 list-continuation case.

## Carried over

## Follow-ups
