# M1-16 — Binder: operator schemas, column naming and joins

**Status:** todo
**Depends on:** M1-05
**Crates:** `datascript-compiler`
**Diagnostic codes:** DS1030–DS1049, DS2030–DS2039; also implements the registered DS1005 and
DS1007 (D-018)

## Goal
Compute the output schema of every ● operator, following the column naming and ordering rules,
on top of the binder core from M1-05. Split out of M1-05 so each task fits one session.

## Scope
- Per-operator schema transformation for all ● operators: `project*`, `extend` (replace in place),
  `summarize` (keys first, distinct keys with no aggregates), `sort`/`top`, `take`, `count`,
  `distinct`, `union` (by name, widening, DS2003), `getschema`.
- Column naming rules of LANGUAGE.md §3.3: unnamed aggregates and keys, `Column1`, …, DS1005.
- Aggregate context rules (DS2004, DS2005) where they depend on the operator.
- `join`: `$left`/`$right`, key type compatibility, `on key` producing one key column (D-019:
  left position, right value for `rightouter`, coalesced for `fullouter`), `$left.a == $right.b`
  keeping both, DS1007 for other collisions, semi/anti returning one side.
- `stage_schemas` filled for every operator.

## Out of scope
Source resolution and the capability pass (M1-17). `render` (M1-10).

## Acceptance criteria
- [ ] Golden `check` cases for each operator (happy path + at least one error each), using the
      harness's `schema:` header with a mock provider until a real provider exists.
- [ ] The README example's `join` yields one `customerId` column and no DS1007.
- [ ] Golden cases for each join kind's key column, including `fullouter` with nulls on both sides.

## Follow-ups
