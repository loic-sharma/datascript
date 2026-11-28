# M2-09 — Reader schema overrides

**Status:** todo
**Depends on:** M1-13, M2-13
**Crates:** `datascript-syntax`, `datascript-compiler`, `datascript-datafusion`
**Diagnostic codes:** DS0280–DS0289 (syntax), DS1080–DS1084 (names), DS2300–DS2309 (types), DS3050–DS3059 (sources) (D-018)

## Goal
`read_csv('x.csv', schema = (price: real, ts: datetime))` for partial or full schema
declaration, overriding inference. The parser needs `name: type` lists inside a call
argument.

## Acceptance criteria
- [ ] Partial schemas merge with inferred columns. Unknown column names in the schema give did-you-mean.
- [ ] Parse failures at runtime point at the schema entry.

## Carried over
- [ ] (from M2-13) DIAGNOSTICS.md deferred a second `DS2003` help to this task: when a column read from a file has the wrong type, also suggest declaring it, e.g. `read_csv('products.csv', schema = (price: real))`. Add it, with a golden case, and remove the "From M2-09 on" note in DIAGNOSTICS.md.
- [ ] (from M2-33) The JSON reader decodes into any target schema by key name; pass the declared schema to it for `read_json` rather than casting after the read, and use the M2-33 type-check table for values that do not match a declared type.

## Follow-ups
