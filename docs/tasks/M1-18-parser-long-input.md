# M1-18 — Parser: long and deep input

**Status:** todo
**Depends on:** M1-03
**Crates:** `datascript-syntax`
**Diagnostic codes:** DS0100–DS0101 (D-018); no new codes expected

## Goal
No input crashes the parser or makes it slow. Long flat chains of operators must be bounded
the same way nesting already is. Found in a review of M1-02/M1-03.

## Scope
- **Stack overflow on long chains.** `MAX_NESTING` (`parser/grammar.rs`, `expr_bp`) only counts
  recursion. Left-associative chains (`a + a + … + a`) and postfix chains (`a.b.b…`) are built by
  a loop through `precede`, so the tree gets one level deeper per operator with no limit.
  - `parse()` aborts at about 200k terms inside rowan's `Arc::drop_slow`.
  - `ds check` (debug build) aborts on `'orders.csv' | where amount > 0` followed by 50,000 ×
    ` + amount`.
  - Recursive walkers (`walk_expr` in `tests/parser_props.rs`, the binder later) overflow much sooner.
  - Fix: count each operator folded in `expr_bp_inner` and each `postfix` step toward the depth
    budget (restore on return) and report DS0100 past the limit.
- **Quadratic time on unspaced chains.** Release build: `a+a+…` takes 84 ms at 5k terms and
  1.37 s at 20k; `a.b.b…` is similar. The spaced form takes 2 ms at 20k. The likely cause is
  rowan's `NodeCache` hashing small nodes all the way down. Confirm the cause; the depth cap may be
  enough, otherwise avoid the cache for expression nodes.
- **Quadratic scan in `foreign_binary_operator`** (`parser/targeted.rs`). For every `&&`, `||`,
  `<>` or `=` it filters all tokens of the file. 8k lines of `T | where a && b;` take 152 ms
  against 3.1 ms with `and`. Binary-search the token range `lhs_start..rhs.1` instead.

## Acceptance criteria
- [ ] Property/pathological tests in `parser_props.rs` for flat chains: `a+a+…`, `a + a + …`,
      `a.b.b…`, and many `&&` lines. Each parses (or reports one DS0100) without overflowing,
      including a recursive walk of the result and dropping the tree.
- [ ] A golden case showing the DS0100 for an over-long chain.
- [ ] A timing test or benchmark: 20k-term chains (spaced and unspaced) and 8k lines of `&&`
      parse in linear time (record numbers in Notes).

## Carried over

## Follow-ups
