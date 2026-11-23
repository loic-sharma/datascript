# M2-06 — Complete the function library

**Status:** todo
**Depends on:** M1-13, M2-13
**Crates:** `datascript-syntax`, `datascript-compiler`, `datascript-datafusion`
**Diagnostic codes:** DS0270–DS0279 (syntax), DS2280–DS2299 (types), DS4055–DS4059 (runtime) (D-018)

## Goal
Fill in the remaining functions listed in LANGUAGE.md §7 and the commonly used KQL string,
datetime, and math functions that aren't there yet (keep a checklist in this file).

Not this task's: `split`, `make_list` and `make_set` belong to M2-03 (they need `dynamic`);
`todecimal`, `toguid` and `new_guid` belong to M2-10.

## Acceptance criteria
- [ ] Every function has catalog docs, an example, and a golden case.
- [ ] Functions we deliberately do not support are listed with the reason, and calling one gives a
      tailored error instead of "unknown function".

## Carried over
- [ ] (from M1-02) `*` as a function argument (`arg_max(x, *)`) is not parsed yet.
- [ ] (from KQL review, 2026-11) `iif` is KQL's alias for `iff`. Keep `iff` as the only name and give
      `iif(...)` a tailored error with a machine-applicable fix to `iff`, as `filter` → `where`
      does. `iif` stays in `PREDICATE_FUNCTIONS` (`targeted.rs`) so `iif(x = 1, …)` still gets the
      `==` fix.
- [ ] (from KQL review, 2026-11) `floor(x, size)` with two arguments (KQL's alias for `bin`): give a
      tailored error suggesting `bin(x, size)` instead of a bare arity error.
- [ ] (from KQL review, 2026-11) Add one-argument `trim(s)`, `trim_start(s)` and `trim_end(s)` that
      trim whitespace, keeping KQL's two-argument `trim(regex, s)` (D-031). Record the extra
      overload in DECISIONS.md.
- [ ] (from KQL review, 2026-11) Prioritize the everyday log-filtering string operators/functions:
      `has_any`, `has_all`, `hasprefix`, `hassuffix` (and their `_cs` / `!` forms where KQL has them).

## Follow-ups
