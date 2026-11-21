# M1-25 — `from` mistakes and statement starts

**Status:** done
**Depends on:** M1-24
**Crates:** `datascript-syntax`, `datascript-diagnostics`, `datascript-tests`
**Diagnostic codes:** DS0152–DS0159 (D-018)

## Goal
[M1-24](M1-24-from-keyword.md) makes `from` required and reports a missing one. This task adds what
comes after: targeted errors for `from` in the wrong place and for SQL habits around it, and a rule
for where a statement starts that uses `from` instead of line breaks and indentation, so the
missing-`;` fix (DS0106) is certain more often and recovery from an unclosed bracket keeps more of
the program.

Read D-036 (written by M1-24) and D-033, D-035 first.

## Design
**Statement starts: by grammar position, not by layout.** After a complete expression, where only
`|`, `;`, `)` or the end could come next, a `from` keyword (that is, `from` followed by an
expression start, D-036) can never continue the statement, because `from` as a name cannot follow
a complete expression. So at that point a `from` keyword starts a new statement, whatever line it is
on and however it is indented. The parser checks this in `pipeline_tail` and in recovery, where it
checks `at_new_statement` today.

Where the `from` sits chooses only the message and how certain the fix is:
- **First on its line**: DS0106 with one `MachineApplicable` `;` fix, even after an operator that
  takes a list. This overrides D-035's `,`-then-`;` rule for that case.
- **On the same line** (`from t | take 5 from u`): after `project`, `extend`, `summarize` or
  `distinct` this is most likely SQL's `SELECT a FROM t` (`| project a from t`). Report it with the
  SQL-order diagnostic below, with no machine-applicable fix. Anywhere else it is DS0106 with a
  `MaybeIncorrect` `;` fix.
- **Inside a bracket that is never closed** (`f(a⏎  from t | take 1`): recovery stops at the `from`,
  so the next statement is kept, without D-033's "unindented" condition. A bracket that is closed
  later (`closing_ahead`) is not affected; the `from` inside it is a plain error.

The old layout rule (any expression start first on a line, outside brackets, is a new statement)
stays only as a fallback for a statement that forgot its `from` (`orders | take 5` on the next
line). Recovery still needs it, and DS0150 explains the statement.

Because the rule depends on where the grammar is, the `from` inside `range x from 1 to 10 step 1`
(LANGUAGE.md §3.1) needs no special case. After `range x` the generator is not complete, so the
parser never reaches the point where `from` would start a statement, even with `from` on its own
line (`range x⏎from 1 to 10`). Keep it that way, so M2-01 inherits this.

## Scope
- **Parser** (`grammar.rs`): `pipeline_tail`, `at_new_statement`, `statement_after_unclosed_at`,
  `closing_ahead`, and DS0106 (`missing_semicolon` in `targeted.rs`).
- **Targeted diagnostics** (`targeted.rs`), each with golden cases, codes from DS0152–DS0159:
  - **`from` in the wrong place**, with a machine-applicable fix that removes it (and the space
    after it): after a pipe (`t | from x`, and `| FROM x`, which today is DS0105 with no help), and
    on the right of `join`, `union` and `lookup` (`join from customers on id`).
  - **`FROM` in capitals at a head**: `FROM orders | take 5`. DS0105 with a machine-applicable fix
    to `from`. (Until now it parses as the name `FROM` followed by stray tokens.)
  - **SQL order without a pipe**: `from t where x` is already DS0102 (missing `|`), because `where`
    is an operator. `from t select a`, `from t SELECT a` and `from t GROUP BY a` are not: they get a
    generic DS0101. Give them a code that says to pipe into the DataScript operator
    (`from t | project a`, `from t | summarize … by a`), with a fix where it is certain. The same
    code covers SQL's `FROM` after the column list on one line: `| project a from t`.
- **Existing tests**: DS0106 and DS0112 recovery cases that rely on the "expression at the start of
  a line" rule. Where `from` makes a fix certain, make it `MachineApplicable` (D-035).
- **Docs**: DIAGNOSTICS.md registry entries for the new codes, and the DS0106 summary. DECISIONS.md:
  record the statement-start rule and the DS0106 override of D-035 (a new decision, or an addition
  to D-036), and note at D-033 and D-035 what it changes.

## Acceptance criteria
- [x] Golden cases for each new diagnostic, each with its fix, checked by `mode: syntax` (the fix
      applied parses).
- [x] A missing `;` before a line starting with `from` gets a machine-applicable DS0106 fix, also
      after `project`, `extend` and the other list operators, and also when the `from` line is
      indented.
- [x] `from t | take 5 from u` on one line gets DS0106 with a `MaybeIncorrect` fix;
      `from t | project a from u` gets the SQL-order diagnostic; `f(a⏎  from t | take 1` keeps
      the second statement.
- [x] `t | from x`, `| FROM x`, `join from customers on id`, `union from 'b.csv'` and `FROM t` each
      get one error with a fix.
- [x] Core crates still build for `wasm32-unknown-unknown`.
- [x] M2-01 updated with a "Carried over" item: `range x⏎from 1 to 10` stays one statement, with a
      golden case once `range` parses.

## Carried over
- [x] (from M1-24) `join from t` and `union from t` get a generic DS0101 from `grammar::right_table` today, and the `from` token is left as a stray `IDENT` child of `JOIN_OP`/`UNION_OP` (`tests/cases/syntax/from_in_join.dstest`). Replace it with the targeted diagnostic and fix. `| from x` now gets a plain DS0104 (its alias is gone) and `| FROM x` a DS0105 with no help.

## Follow-ups
- Landed: DS0152 (`from` after `|`, and on the right of `join`/`union`), DS0153 (SQL clause order),
  DS0105 for `FROM` at a head, and the statement-start rule (D-037). `t | from x` gets a
  `MaybeIncorrect` fix turning `|` into `;` rather than removing `from` (D-037 says why); removal is
  certain before an operator name. `tests/cases/syntax/from_in_join.dstest` was replaced by
  `ds0152_join_from` and `ds0152_union_from`.
- `lookup` and `range` are not parsed yet: handed to M2-01.
