# M1-03 — Syntax diagnostics quality pass

**Status:** todo
**Depends on:** M1-02
**Crates:** `datascript-syntax`
**Diagnostic codes:** DS0102–DS0149 (D-018)

## Goal
Replace generic parse errors with targeted, helpful ones. This task is all about error quality.

## Scope
Implement and golden-test DS0102–DS0120 from [DIAGNOSTICS.md](../DIAGNOSTICS.md), including:
- Missing `|` between operators (`where x > 1 project a`), with a suggestion to insert it.
- Trailing `|` with nothing after it.
- Unknown operator, with did-you-mean (`summerize`, `select`, `filter` → `where`, `orderby`), and
  `project - away` with spaces suggesting `project-away` (D-026).
- SQL habits: `SELECT ... FROM`, `GROUP BY`, `ORDER BY`, `LIMIT` as an operator name (we do accept
  `limit`), `WHERE` in caps (case-sensitive keywords), `<>` → `!=`.
- `=` used as comparison in `where`/`iff`/`join on`, with a machine-applicable fix.
- `&&`, `||`, `!x` → `and`, `or`, `not(x)`, including `!has_error` (LANGUAGE.md §1).
- `render` not last.
- Unbalanced parentheses: point at the opening one.
- Missing `by` in `sort`/`top`, missing `on` in `join`, `kind=innerunique` (D-004 explanation).

Design-review additions: DS0106 missing `;` between statements (machine-applicable insert), DS0107
`<>`, DS0112 unbalanced brackets, DS0121 missing `by`, DS0122 missing `on`, DS0123 unknown join
kind, DS0124 `innerunique`. All codes are already in the registry.

## Acceptance criteria
- [ ] Each diagnostic has at least one golden case with the full rendered output.
- [ ] Machine-applicable suggestions are tested by applying them and re-parsing without errors
      (a harness helper `apply_suggestions`).
- [ ] A review pass over all syntax golden outputs against the style rules in DIAGNOSTICS.md.

## Follow-ups
