# M2-13 — Fix task contracts and diagnostic code blocks

**Status:** todo
**Depends on:** M1-13
**Crates:** none (docs only: `docs/tasks/`, `docs/DIAGNOSTICS.md`, `docs/DECISIONS.md`)
**Diagnostic codes:** none (this task reserves blocks for other tasks; it allocates no codes)

## Goal
A review of `docs/` (November 2026) found task files whose contracts are wrong or incomplete:
missing dependencies, missing handoffs, missing diagnostic code blocks and pre-`from` examples.
Fix them before the affected M2/M4/M5/M6 tasks start, so those agents read a correct contract.
Check each item against the code first; some may already be fixed. Every task whose contract this
changes lists M2-13 in `Depends on`, so do this task first.

Land the D-018 block reservations as a small separate commit to `main` (DIAGNOSTICS.md, Allocation).

## Scope

### Diagnostic code blocks (D-018)
- [ ] `DS0004` (M0-02, engine: "program has no query to run") sits inside M1-01's `DS0001–DS0019`
  block and the Lexical range. Fix the Allocation table (`DS0001–DS0019 except DS0004`), note the
  exception in the Range table, and correct M1-01's claim that it landed `DS0001–DS0006`.
- [ ] Reserve non-overlapping blocks in DIAGNOSTICS.md for each task that will add codes, and add a
  `**Diagnostic codes:**` line to its task file: M2-01, M2-02, M2-03, M2-04, M2-05, M2-06, M2-09,
  M2-10 (check M2-12, which says "none new expected").
- [ ] M4-01 tells its implementer to take Q-007's info diagnostic "from this task's block" but has
  no block beyond `DS3007`. Reserve one.

### Dependencies and ownership
- [ ] M4-01 lists `POST /api/complete`, but `complete` is built in M3-02, which M4-01 does not
  depend on, and M4-04 already owns wiring completion. Move `/api/complete` to M4-04.
- [ ] M4-04: add `POST /api/hover`, backed by the compiler's `hover` (M3-02 puts it in
  `datascript-compiler`).
- [ ] M6-02 depends only on M1-13 but publishes `@datascript/wasm` (M5-02) and VS Code packages
  (M3-04), and may embed `web/` (M4-02). Fix `Depends on` in the task file and index, or scope
  npm publishing out.
- [ ] M5-03 omits M4-03/M4-04 from `Depends on`. M5-02's JS API sketch (`run(text)` returning one
  result) does not cover multiple statements or what `web/` and M5-04 need (`schema`, `hover`,
  `explain`, `registerTable`, file listing/removal, statement index, row limit, `now`,
  `AbortSignal`). ARCHITECTURE.md's WASM section has the same sketch.
- [ ] M2-06 overlaps M2-03 (`make_list`, `make_set`, `split`) and M2-10 (`todecimal`, `toguid`,
  `new_guid`). Say which task owns each.

### Missing handoffs
- [ ] M1-02 left `parse`, `make-series` and `print` in `LATER_OPERATORS`
  (`crates/datascript-syntax/src/parser/grammar.rs`). M2-01 and M2-03 got `(from M1-02)` items;
  M2-02 (`parse`) and M2-04 (`make-series`) did not. Add them, and check `print` is covered by M2-01.
- [ ] M2-04 should carry over updating `web/` charts if `dynamic` columns become chartable
  (LANGUAGE.md §6.2 forbids them today; M4 runs before M2).
- [ ] M0-05's handoff to M5-01 is not in M5-01's "Carried over". M5-01 should also test JSON/NDJSON
  and globs on wasm (`is_json_array` uses `std::fs`).
- [ ] M2-01 should carry over updating the TextMate grammar in `editors/vscode` for new operators.

### Crates lists
- [ ] Add `datascript-syntax` to M2-03, M2-04, M2-06 and M2-09 (M2-09's `schema = (col: type)`
  needs parser support for `:`).
- [ ] M2-10: add `datascript-engine` (table formatting for `decimal`/`guid`) and
  `datascript-fixtures` (`types.parquet` has no `arrow.uuid` column).
- [ ] M3-06: add `datascript-engine` and `web/`. M4-01: add `datascript-compiler` and
  `datascript-diagnostics`; note that `rust-embed` needs a `web/` directory to exist before M4-02.

### Task contents
- [ ] Examples written without `from` (D-036): M4-01's acceptance criterion
  (`'/etc/passwd' | take 1` fails parsing with DS0150 and never reaches DS3007; use
  `from '/etc/passwd' | take 1` and `from '../x.csv'`), M2-05, M4-02, DECISIONS.md Q-005, and
  LANGUAGE.md §4 (`in (T | project a)` → `in (from T | project a)`).
- [ ] M2-02 assumes streaming runtime warnings, but a `BatchStream` error aborts the query (D-055).
  Either add `datascript-engine` to design warnings during a run, or have `parse` return `null`
  without a warning, like the `to*` functions.
- [ ] M2-08's criterion "every registered code has a wrong example" cannot hold for retired codes
  (DS2007, DS2101), planned ones (DS3007) or DS9001. Exempt them. It also still says "If M3-01 is
  done" (it is).
- [ ] M2-09 should say it adds the `DS2003` help that DIAGNOSTICS.md deferred to it.
- [ ] M2-12 quotes the DS3008 message wrongly: the header is ``cannot read `<path>` ``
  (`crates/datascript-datafusion/src/errors.rs`), with the format in the label. Its Goal also
  repeats its "Carried over" item.
- [ ] M2-03 should update DS2012's wording once `dynamic` exists.
- [ ] M3-03: mark UDF items "(after M2-05)", note syntax-only behaviour (D-053), and check whether
  it needs `datascript-compiler` for constant-folded scalar `let` references.
- [ ] M6-01 depends on M2-08 and M5-03 only, not all of M2; say whether that is intended. It also has
  maintainer-only items (Q-005, Q-008); mark them.

## Acceptance criteria
- [ ] Every item above is fixed or marked as not needed, with the reason.
- [ ] Every task file that will add diagnostics names its block, and DIAGNOSTICS.md's Allocation table has no overlaps.
- [ ] The index matches the task files (`Depends on`, status).

## Follow-ups
