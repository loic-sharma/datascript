# M1-17 — Binder: source resolution and capability pass

**Status:** done — sources (paths, globs, reader calls, host tables, D-020 path `let`s) resolve through the provider with DS3xxx/DS1002 at every use; DS2008, DS2040–DS2043 and DS4001 (functions, join kinds, formats); deviations reach `Analysis`
**Depends on:** M1-05
**Crates:** `datascript-compiler`
**Diagnostic codes:** DS2040–DS2049; also implements the registered DS2008, DS3006 and DS4001
(D-018)

## Goal
Everything in the binder that talks about sources and engines, on top of the binder core from
M1-05. Split out of M1-05 so each task fits one session.

## Scope
- `SourceRef` (path, glob, reader call with options, host table name) and the `SchemaProvider`
  trait with `SchemaLookup::{Ready, Failed, Pending}` (ARCHITECTURE.md § Source resolution,
  D-022). `Analysis::pending_sources`. Pending sources get the error type with no diagnostic.
- D-020: a constant string in table position (literal or a scalar `let` bound to one) is a path
  source. A non-constant path or reader option is DS3006.
- Reader calls: option names and constant values per LANGUAGE.md §3.1 (DS3003 is M1-08's code;
  the binder reports it from the catalog of options).
- `SourceError` (D-023) → DS3xxx diagnostics at every span where the source appears, following
  DIAGNOSTICS.md (did-you-mean over `nearby`, both files named for `Conflict`).
- DS2008 when a projection-only column (no DataScript type) is used in an expression.
- Capability pass over the bound program against `&Capabilities`: DS4001 at the call site for an
  unsupported function, join kind or format. `Capabilities::deviations` is carried through to
  `Analysis` for frontends to show; it produces no diagnostic.

## Acceptance criteria
- [x] Golden cases: `let p = 'x.csv'; p | take 1`, the same `p` used in `read_csv(p, ...)`, and a
      non-constant path (DS3006).
- [x] A unit test with a mock provider: first pass returns pending sources, second pass binds
      fully, and no diagnostic mentions a pending source.
- [x] A source used in two places with a missing file gives DS3001 at both spans.
- [x] Golden DS4001 case with a mock `Capabilities` missing one function.

## Carried over
- [x] (from M1-15) A table name as a source works (DS9001 in the skeleton); columns with no DataScript type give DS2008 instead of a silent error type; `caps` is consulted.
- [x] (from M1-24) Table positions are the operand of `from` (`Pipeline::source()`, after `Pipeline::from_token()`) and the right side of `join`/`union`/`lookup`. A constant string or a scalar `let` bound to one is a path there (D-020, D-036).
- [x] (from M1-24) When the reader definitions land, replace the check in `catalog_tests::syntax_readers_match_the_compiler` with a comparison of `datascript_syntax::READERS` against them (today it only checks each name reads a known `SourceFormat`).
- [x] (from M1-05) A name in `from` that is not a tabular `let` still gives DS9001 "a table name as a source" (`Binder::bind_source`); a scalar `let` there that is not a constant string needs its own error. `let alias = orders;` where `orders` is a host table gives DS1002 today; it should get DS1006 with the `from` fix, as it does for a tabular `let` (`Binder::bind_scalar_let`).

## Follow-ups
- Decisions in D-041: the binder decides formats (DS3002) and checks reader options itself;
  capability checks run where functions, `join` kinds and sources are bound, not in a separate
  pass; host tables are `SourceRef::Table`; projection-only columns live in
  `hir::Column::arrow_only`.
- Golden cases are in `tests/cases/sources/` (mock provider; `datafusion_formats.dstest` uses the
  real engine). The harness gained a `without:` header for a restricted `Capabilities`
  (TESTING.md). DS2008 is covered by a unit test, since `schema:` cannot declare an Arrow-only
  type.
- `tests/cases/harness/explain.dstest` is un-skipped (it needed host tables).
- Handed on: M1-08 (DataFusion formats, globs, reader options and host tables behind the new
  `SourceRef` cases) and M1-06 (planning projection-only columns).
