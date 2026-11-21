# M1-18 — Parser: long and deep input

**Status:** done — each operator folded into a chain now costs one level of the 256-level nesting budget (DS0100 "expression is too long"), and the foreign-operator check binary-searches its token range; long and deep input parses in linear time without overflowing.
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
- [x] Property/pathological tests in `parser_props.rs` for flat chains: `a+a+…`, `a + a + …`,
      `a.b.b…`, and many `&&` lines. Each parses (or reports one DS0100) without overflowing,
      including a recursive walk of the result and dropping the tree.
- [x] A golden case showing the DS0100 for an over-long chain.
- [x] A timing test or benchmark: 20k-term chains (spaced and unspaced) and 8k lines of `&&`
      parse in linear time (record numbers in Notes).

## Notes
- **Depth budget.** `fold_level` (`parser/grammar.rs`) spends one level of `MAX_NESTING` per infix
  fold in `expr_bp_inner` and per `.`/`[` step in `postfix`; `expr_bp` restores the counter when
  its frame ends. It checks one level early so the error lands on the operator, not on its operand.
  The limit hit at an operator says "expression is too long" with a note; the bracket/prefix case
  keeps "expression is nested too deeply". Recorded as D-032; LANGUAGE.md §4 and the DS0100 row
  in DIAGNOSTICS.md updated.
- **Cause of the quadratic time, confirmed.** rowan 0.16 interns nodes with at most 3 children
  (`NodeCache::node`). Unspaced `a+a` gives `BIN_EXPR` exactly 3 children, so every level is
  interned; when the table grows, hashbrown rehashes every entry with `node_hash`, which recurses
  through the whole subtree, so cost is O(entries × depth). Spaced chains have 5 children and are
  never interned. The depth cap bounds this; no change to the cache was needed.
- **Timings** (release, `long_input_parses_in_linear_time`, min of 3, Apple Silicon):

  | Input | before 5k | before 20k | after 5k | after 20k |
  |---|---|---|---|---|
  | `a+a+…` | 84 ms | 1.33 s | 1.3 ms | 1.6 ms |
  | `a + a + …` | — | 2 ms (task) | 0.6 ms | 1.5 ms |
  | `a.b.b…` | — | ~1.3 s (task) | 0.6 ms | 1.0 ms |
  | 2k / 8k lines of `T \| where a && b;` | — | 152 ms at 8k (task) | 1.4 ms | 6.5 ms |

  The test is `#[ignore]`d (timing is noisy in debug and under load) and asserts a 20k/5k ratio
  under 8 (linear is 4, quadratic 16). Run it with
  `cargo test --release -p datascript-syntax --test parser_props -- --ignored --nocapture`.
- The 50,000 × ` + amount` query from the task now gives one DS0100 under `ds check` (debug).
- The golden case `tests/cases/errors/syntax/ds0100_long_chain.dstest` runs in `check` mode
  (a `syntax` dump would be thousands of lines), so it also shows the skeleton binder's DS9001
  for `+`. That line disappears when M1-05 implements `+`; rebless then.

## Carried over

## Follow-ups
- None. (Long source lines in rendered diagnostics are already M1-22's.)
