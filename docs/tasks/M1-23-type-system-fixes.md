# M1-23 — Type system and catalog fixes

**Status:** todo
**Depends on:** M1-04
**Crates:** `datascript-compiler` (`types.rs`, `operators.rs`, `catalog.rs`, `builtins.rs`, `arrow.rs`)
**Diagnostic codes:** none new (M1-04 builders for DS2002/DS2003/DS2006/DS2009)

## Goal
Fix bugs and inconsistencies in the type lattice, operator table and function catalog, found in a
review of M1-04. Binder-side consequences are carried over to M1-05; DataFusion-side ones to
M1-08/M1-09.

## Scope
- **Untyped `null` next to a `timespan` in `*` or `/`.** `null * 1d`, `1d * null` and `1d / null`
  resolve to `TimeSpan × TimeSpan`, so the binder would send `Duration * Duration` to DataFusion.
  `resolve_null` falls back to the other operand's type for `Keep` rows. For a non-exact class use
  the class default (`Numeric`/`Integer` → `long`; `Comparable`/`Any` → the other type).
- **`decimal` with `real`.** `Type::common(Decimal, Real)` is `Real`, but `TypeClass::Numeric`
  excludes `Decimal`, so `decimal + real`, `decimal < real` and `avg(decimal)` fail (only `==`
  works). Make the operator table agree with the lattice, or narrow LANGUAGE.md §4.2 to M2.
- **Integer math functions keep `int`.** `abs`, `round`, `floor`, `ceiling`, `bin` on `int`
  return `int` while `-int` is `long` (D-021); `abs(toint(-2147483648))` overflows. Use an
  integer → `long` return rule like `sum_type`.
- **Functions can return the internal `Null` type** (`abs(null)`, `sum(null)`, `min(null)`,
  `iff(p, null, null)`, `coalesce(null, null)`, `percentile(null, 50)`, `bin(null, null)`).
  Document this on `Resolved::return_type` and `resolve`, or default it here.
- **Planned M2 functions.** `make_list`, `make_set`, `arg_max`, `arg_min` and `split` are in
  LANGUAGE.md §7 but give a plain `UnknownFunction` with no hint. Keep a `PLANNED` list so the
  binder can say "not available yet".
- **M2-only Arrow types reachable in M1.** `from_arrow_source` maps Struct/List/Map to `dynamic`
  and `arrow.uuid` to `guid`. Return `UnsupportedArrowType` (DS2008) for these until M2-03.
- **Arity/argument messages.** When overloads fail at different positions only the highest is
  reported (`catalog.rs`), which can blame a later argument. Prefer the form whose name matched
  exactly, or the first mismatch of the first form. `describe_accepted` uses only the first
  repeated form.
- Nits: the `Var` + `Null` branch in `ArgTypeError::help` is dead; the `UnknownFunction` doc
  mentions KQL names but `FOREIGN_NAMES` has none.

## Acceptance criteria
- [ ] Unit test running every operator row with `Null` on each side.
- [ ] Commutativity property test: for `+ * == !=` and every type pair, `binary(l, r)` succeeds
      exactly when `binary(r, l)` does, with mirrored operand types.
- [ ] Unit tests for `decimal` operands and for functions with `null` arguments.
- [ ] Every catalog example is resolved (with column types inferred from the example), not only
      parsed.
- [ ] Unit test that the `Error` short-circuit returns `bool` for `and`/`or`/`not` and the string
      operators.
- [ ] Any spec change is in LANGUAGE.md and DECISIONS.md.

## Carried over

## Follow-ups
