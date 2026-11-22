# M2-06 — Complete the function library

**Status:** todo
**Depends on:** M1-13, M2-13
**Crates:** `datascript-compiler`, `datascript-datafusion`

## Goal
Fill in the remaining functions listed in LANGUAGE.md §7 and the commonly used KQL string,
datetime, and math functions that aren't there yet (keep a checklist in this file).

## Acceptance criteria
- [ ] Every function has catalog docs, an example, and a golden case.
- [ ] Functions we deliberately do not support are listed with the reason, and calling one gives a
      tailored error instead of "unknown function".

## Carried over
- [ ] (from M1-02) `*` as a function argument (`arg_max(x, *)`) is not parsed yet.

## Follow-ups
