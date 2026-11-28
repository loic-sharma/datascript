# M2-10 — `decimal` and `guid` types

**Status:** todo
**Depends on:** M1-13, M2-13
**Crates:** `datascript-compiler`, `datascript-datafusion`, `datascript-engine` (table formatting
for `decimal`/`guid`), `datascript-fixtures` (`types.parquet` has no `arrow.uuid` column yet)
**Diagnostic codes:** none new expected (reuse DS2003, DS2006, DS2008); DS2310–DS2319 if needed (D-018)

## Goal
Make `decimal` and `guid` real types. In M1 they exist only as typed nulls (`decimal(null)`,
`guid(null)`): files read decimals as `real` and `arrow.uuid` columns as unsupported, and
`decimal` mixes with no other type (D-039).

## Scope
- Decide how `decimal` mixes with `int`, `long` and `real` (the lattice in `types.rs`, the operator
  table in `operators.rs`, `TypeClass::Numeric`, aggregates such as `avg` and `sum`), and record it
  in LANGUAGE.md §4.1/§4.2 and DECISIONS.md. Keep the commutativity test in `operators.rs` green.
- Read Arrow decimals as `decimal` (LANGUAGE.md §5.1), and FixedSizeBinary(16) with `arrow.uuid` as
  `guid` (`arrow::from_arrow_source`, which returns `UnsupportedArrowType` for it in M1).
- `todecimal`, `toguid`/`new_guid` as in KQL, with catalog docs and examples.
- Drop the `decimal` help in `BinaryTypeError::help` ("do not mix with other types yet") once
  mixing is defined.

## Acceptance criteria
- [ ] Golden cases over a Parquet file with decimal and uuid columns.
- [ ] Unit tests for every operator row with `decimal` operands.

## Carried over
- [ ] (from M2-20) If M2-20 has landed, apply its always-null conversion error to `todecimal` and
      `toguid` (e.g. `toguid(42)`, `todecimal(datetime(...))`) instead of accepting `ANY`.
- [ ] (from M2-33) Remove the `Type::Guid` guard in `crates/datascript-compiler/src/binder/sources.rs`
      and `crates/datascript-compiler/src/ide/mod.rs` when `arrow::from_arrow_source` maps
      `arrow.uuid` to `guid`.

## Follow-ups
