# M1-17 — Binder: source resolution and capability pass

**Status:** todo
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
- [ ] Golden cases: `let p = 'x.csv'; p | take 1`, the same `p` used in `read_csv(p, ...)`, and a
      non-constant path (DS3006).
- [ ] A unit test with a mock provider: first pass returns pending sources, second pass binds
      fully, and no diagnostic mentions a pending source.
- [ ] A source used in two places with a missing file gives DS3001 at both spans.
- [ ] Golden DS4001 case with a mock `Capabilities` missing one function.

## Carried over
- [ ] (from M1-15) A table name as a source works (DS9001 in the skeleton); columns with no DataScript type give DS2008 instead of a silent error type; `caps` is consulted.

## Follow-ups
