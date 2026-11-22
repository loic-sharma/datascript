# M1-14 — Warning lints

**Status:** done — lint pass over the HIR (`lints::LINTS`), DS2102 constant `where` and DS2103 `sort` without direction with an `asc` fix, golden cases in `tests/cases/lints/` (D-051)
**Depends on:** M1-16
**Crates:** `datascript-compiler`
**Diagnostic codes:** DS2100–DS2149 (D-018)

## Goal

Warnings for programs that run but probably not as intended (DIAGNOSTICS.md § Severity).

## Scope

- DS2102: a predicate that is constant after folding (`where 1 == 1`, `where x != x` on a
  non-null column type is not constant because of nulls: be careful and only flag real constants).
- DS2103: a `sort` key with no `asc`/`desc` (D-005). The default is ascending, unlike KQL. A
  machine-applicable fix inserts `asc`. `top` keys do not warn.
- A lint framework in the binder's output (a pass over the HIR) that later lints plug into, with
  each lint's code and default severity in one table.
- `ds check --deny-warnings` already fails on these (M1-11).
- No lint for `take`/`limit` before `sort`: DS2101 is retired (D-050).

## Acceptance criteria

- [x] Golden `check` cases for each lint, including cases that must **not** warn.
- [x] DS2103 golden case, and a case with `sort by x asc` that must not warn. Applying the fix
      silences the warning.
- [x] Warnings do not stop `run`: a golden `run` case shows the warning, then the table.

## Notes

- The fix for DS2103 is checked by `binder_tests::ds2103_fix_silences_the_warning` (check-mode
  golden cases do not print fixes); `lints/ds2103_fixed.dstest` shows the fixed form is quiet.
- Lint severities are fixed at their default; there is no way to configure them yet. Nothing
  asks for it in M1.
- Existing golden cases already wrote `asc`/`desc` on every `sort` key, so none changed.

## Follow-ups

None.
