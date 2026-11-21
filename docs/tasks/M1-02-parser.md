# M1-02 — Parser and syntax tree

**Status:** todo
**Depends on:** M1-01
**Crates:** `datascript-syntax`
**Diagnostic codes:** DS0100–DS0101 (D-018)

## Goal
Recursive-descent parser producing a lossless rowan CST with error recovery, plus a typed AST
layer. See D-001 and ARCHITECTURE.md § Syntax.

## Scope
- Grammar from LANGUAGE.md §8 for statements, `let`, sources (path literal, name, reader call,
  parenthesized), every ● operator, and scalar expressions with the precedence table in §4.
- Event-based parser (start node / token / finish node) building the rowan `GreenNode`,
  as in rust-analyzer.
- Recovery: on an unexpected token, wrap it in `ERROR` and resynchronize at `|`, `;`, `)`, or a
  known operator keyword. Never loop and never panic, including on random input (fuzz with
  `cargo fuzz` or a proptest).
- Typed AST: `ast::Program`, `ast::LetStmt`, `ast::Pipeline`, one struct per operator, and the
  expression variants, each with accessor methods. Generate them with a small codegen or macros if
  the boilerplate gets large.
- CST debug dump format used by `mode: syntax` golden tests.
- Public API: `parse(text) -> Parse { green, errors }`, `Parse::program() -> ast::Program`.

Design-review additions:
- Contextual keywords: match identifier text where the grammar expects a keyword, so `| where
  order > 3` and `summarize sum(count) by kind` parse.
- Hyphenated operator names (D-026): right after `|`, join adjacent `ident` `-` `ident` tokens with
  no trivia between them into one operator node (`project-away`, `project-rename`,
  `project-reorder`, `mv-expand`, `make-series`).
- Typed-null literals `<type>(null)` (LANGUAGE.md §1), not function calls.
- `right_table` accepts a path string (`join 'customers.csv' on customerId`).
- `in (...)`, `between (a .. b)`, and non-chaining comparisons (`a < b < c` is an error suggesting
  `a < b and b < c`).
- A `let` produces one `LetStmt` node whether its right side is scalar or tabular; the binder
  decides from the right side (D-020).

## Out of scope
Targeted "foreign habit" diagnostics (M1-03). Parse errors in this task may use the generic DS0101,
but they must name what was expected in words.

## Acceptance criteria
- [ ] Golden `syntax` cases for every operator and expression form, including precedence corner
      cases (`not a == b`, `a - -b`, `x between (1 .. 10)`), `| project-away x`, `| project - away x`
      (error), and `long(null)`.
- [ ] Recovery cases: two broken operators in one pipeline produce two errors and a usable tree
      for the rest.
- [ ] Fuzz/property test: arbitrary input never panics, and the tree text equals the input.
- [ ] Parsing a 10k-line file takes < 50 ms in release (simple benchmark, recorded in the task).

## Follow-ups
