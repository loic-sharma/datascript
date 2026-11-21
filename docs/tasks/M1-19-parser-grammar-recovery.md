# M1-19 — Parser: grammar fixes and recovery

**Status:** done — comparison chains are one error with full help; bare `not` under a tighter operator (DS0114), method calls (DS0113), `where a, b` (DS0115), `distinct *, a` (DS0116) and `x in ()` get targeted errors; an unclosed bracket no longer swallows the next statement; `DESC` reads as descending in the AST; hardening and AST doc comments.
**Depends on:** M1-18
**Crates:** `datascript-syntax`
**Diagnostic codes:** DS0100–DS0101, DS0102–DS0149 (D-018)

## Goal
Fix grammar and recovery problems found in a review of M1-02/M1-03. Runs after M1-18 because both
edit `parser/grammar.rs`.

## Scope
- **Chained-comparison help is wrong** (`grammar.rs`, around the `previous` tracking).
  `a < b < c < d` reports twice, and the second help says ``compare each pair separately:
  `a < b < c and c < d` ``, which is still a chain. Report one error per chain (an 8k chain gives
  7,998 errors today) with help that splits every link.
- **Spurious chain error after a missing operand.** `T | where a == b and == c` correctly reports
  "expected an expression after `and`", then also "comparison operators cannot be chained" with
  help `a == b and b == c`. Reset `previous` after a lower-precedence operator, or skip the check
  when the right side was empty.
- **Prefix `not` inside tighter operators.** `1 + not x == y` parses as `1 + (not (x == y))`, and
  `a == not b == c` hides a chained comparison. LANGUAGE.md §4 puts `not` at level 3. Either reject
  `not` as the operand of comparison/arithmetic operators, or write the permissive rule into
  LANGUAGE.md §4 and DECISIONS.md (extend D-029).
- **`x in ()`** parses with no error; §8 requires at least one value. Report it in the parser.
- **Unclosed bracket swallows the next statement.** `expect_closing` → `recover` stops only at `;`
  or `let`. In `T | where f(a` followed by `U | take 1` on the next line, the second query is lost.
  Also stop at a new statement start at depth 0 (or an operator name at the start of a line).
- **Method-style calls** `a.b.c(1)` give "expected `|` or `;`, found `(`". Add a targeted message
  (a free code from DS0108–DS0149) saying DataScript has no method calls, with a function-call
  suggestion where one exists.
- **Sort direction case.** `sort_key` accepts `DESC` case-insensitively (with DS0105), but
  `SortKey::direction_token` (`ast.rs`) only matches lowercase, so the binder sees no direction.
  Match case-insensitively in the AST.
- Hardening nits: `TokenSet` silently drops kinds ≥ 128 (`parser/mod.rs`), so add a compile-time
  or debug assert. `operator_named` can complete an empty node if `certain_operator` ever gives
  `len == 0`, so use `complete_if_nonempty` or assert.
- Doc comments on public AST accessors that lack them (`SortKey::expr`, `BinExpr::lhs/rhs/op`,
  `InExpr::lhs`, `ArgList::args`, `CallExpr::callee`, and any others).

## Acceptance criteria
- [x] Golden cases: `a < b < c < d`, `a == b and == c`, `1 + not x == y`, `a == not b == c`,
      `x in ()`, an unclosed `(` followed by a new statement on the next line, `a.b.c(1)`,
      `sort by a DESC` (AST direction is `desc`), `distinct *, a`, `project a = b = c`,
      `join (U | where a, b) on k`.
- [x] Any spec change is in LANGUAGE.md and DECISIONS.md.

## Notes
- **`not`**: rejected rather than made permissive (D-033, LANGUAGE.md §4). The tree keeps the
  old shape (`not` takes everything up to `and`/`or`), so the error does not cascade; the
  suggestion `not(...)` is `MaybeIncorrect` because the user may have meant a narrower operand.
- **Codes**: DS0113 method calls, DS0114 bare `not`, DS0115 `where a, b`, DS0116 `distinct *, a`.
  `x in ()` stays DS0101 ("expected a value in the list, found `)`", with a label and help).
- **`project a = b = c`**: after `name =`, a further `=` is read as a mistyped comparison, so it
  gets the existing DS0110 with its `==` fix. **`join (U | where a, b) on k`** gets DS0115; inside
  a call's arguments (`f(T | where a, b)`) the comma still starts the next argument.
- **Recovery**: a line counts as a new statement inside an unclosed bracket only if it is
  unindented, can start an expression (not `-`, `not`, or a word operator/keyword such as `in`,
  `by`, `on`), and follows a token that ends an expression. Indented or comma-continued lines
  stay inside. `closing_ahead` stops at the same place, and a parenthesized pipeline stops there
  too, so `let x = (T | where a⏎U | take 1` keeps the second query.
- **Method calls** parse as a `CALL_EXPR` whose first child is the `FIELD_EXPR`, so
  `CallExpr::callee()` is `None` and the arguments are still walked (test in `tests/ast.rs`).
- `TokenSet::new` asserts every kind is below 128 (a build error in a `const`);
  `certain_operator` never returns a zero-token name.
- Cases: `tests/cases/syntax/` for parse shapes and recovery, `tests/cases/errors/syntax/ds0113_*`
  … `ds0116_*` for the new codes.

## Carried over

## Follow-ups
- None.
