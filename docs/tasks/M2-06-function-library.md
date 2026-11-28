# M2-06 — Complete the function library

**Status:** todo
**Depends on:** M1-13, M2-13
**Crates:** `datascript-syntax`, `datascript-compiler`, `datascript-datafusion`, `editors/vscode` (TextMate grammar for new string operators)
**Diagnostic codes:** DS0270–DS0279 (syntax), DS2280–DS2299 (types), DS4055–DS4059 (runtime) (D-018)

## Goal
Fill in the remaining functions listed in LANGUAGE.md §7 and the commonly used KQL string,
datetime, and math functions that aren't there yet (keep a checklist in this file).

Not this task's: `split`, `make_list` and `make_set` belong to M2-35 (they need `dynamic`);
`todecimal`, `toguid` and `new_guid` belong to M2-10.

## Acceptance criteria
- [ ] Every function has catalog docs, an example, and a golden case.
- [ ] Functions we deliberately do not support are listed with the reason, and calling one gives a
      tailored error instead of "unknown function".

## Carried over
- [ ] (from M1-02) `*` as a function argument (`arg_max(x, *)`) is not parsed yet.
- [ ] (from KQL review, 2026-11) `iif` is KQL's alias for `iff`. Keep `iff` as the only name and give
      `iif(...)` a tailored error with a machine-applicable fix to `iff` via `FOREIGN_NAMES` in
      `crates/datascript-compiler/src/catalog.rs` (reusing `DS1004`, as `todouble` → `toreal` and
      `if` → `iff` do). `iif` stays in `PREDICATE_FUNCTIONS` (`targeted.rs`) so `iif(x = 1, …)`
      still gets the `==` fix. Check first what `iif(...)` gives today: it is one edit from `iff`,
      so the "similar name" help may already offer the fix, and this only changes the wording.
      Update the comments on `FOREIGN_NAMES` and `UnknownFunction::equivalent`, which say KQL
      names need no entry (D-014); `todouble` already contradicts them.
- [ ] (from KQL review, 2026-11) `floor(x, size)` with two arguments (KQL's alias for `bin`): give a
      tailored `DS2002` help suggesting `bin(x, size)` in `ArityError::diagnostic`
      (`crates/datascript-compiler/src/catalog.rs`, matching the existing `count(x)` pattern)
      instead of a bare arity error.
- [ ] (from KQL review, 2026-11) Add one-argument `trim(s)`, `trim_start(s)` and `trim_end(s)` that
      trim whitespace, keeping/adding KQL's two-argument `trim(regex, s)`, `trim_start(regex, s)`
      and `trim_end(regex, s)` (D-031). Record the extra overload in DECISIONS.md and mark the
      1-argument forms `≠ KQL` in LANGUAGE.md §7. Guard `Binder::check_constant_args` (`binder.rs`)
      so arg 0 is validated as a regex only for the 2-argument forms.
- [ ] (from KQL review, 2026-11) Prioritize the everyday log-filtering infix string operators:
      `has_any`, `has_all`, `hasprefix`, `hassuffix` (and their `_cs` / `!` forms where KQL has
      them). `has_any`/`has_all` take a `(...)` list like `in` (respecting M2-19 trailing commas).
      Update the lexer (`NEGATABLE`), `editors/vscode/syntaxes/datascript.tmLanguage.json` (checked
      against the lexer per D-053), and `INFIX` in `crates/datascript-compiler/src/ide/complete.rs`.

## Follow-ups
