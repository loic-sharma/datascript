# M1-14 — Warning lints

**Status:** todo
**Depends on:** M1-16
**Crates:** `datascript-compiler`
**Diagnostic codes:** DS2100–DS2149 (D-018)

## Goal

Warnings for programs that run but probably not as intended (DIAGNOSTICS.md § Severity).

## Scope

- DS2101: `take`/`limit` before a later `sort` in the same pipeline. The rows kept are arbitrary.
  Help: move `take` after `sort`, or use `top`.
- DS2102: a predicate that is constant after folding (`where 1 == 1`, `where x != x` on a
  non-null column type is not constant because of nulls: be careful and only flag real constants).
- DS2103: a `sort` key with no `asc`/`desc` (D-005). The default is ascending, unlike KQL. A
  machine-applicable fix inserts `asc`. `top` keys do not warn.
- A lint framework in the binder's output (a pass over the HIR) that later lints plug into, with
  each lint's code and default severity in one table.
- `ds check --deny-warnings` already fails on these (M1-11).

## Acceptance criteria

- [ ] Golden `check` cases for each lint, including a case that must **not** warn
      (`sort by x | take 10`).
- [ ] DS2103 golden case, and a case with `sort by x asc` that must not warn. Applying the fix
      silences the warning.
- [ ] Warnings do not stop `run`: a golden `run` case shows the warning, then the table.

## Follow-ups
