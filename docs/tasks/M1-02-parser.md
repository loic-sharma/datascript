# M1-02 — Parser and syntax tree

**Status:** todo
**Depends on:** M1-01
**Crates:** `datascript-syntax`

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

## Out of scope
Targeted "foreign habit" diagnostics (M1-03). Parse errors in this task may use the generic DS0101,
but they must name what was expected in words.

## Acceptance criteria
- [ ] Golden `syntax` cases for every operator and expression form, including precedence corner
      cases (`not a == b`, `a - -b`, `x between (1 .. 10)`).
- [ ] Recovery cases: two broken operators in one pipeline produce two errors and a usable tree
      for the rest.
- [ ] Fuzz/property test: arbitrary input never panics, and the tree text equals the input.
- [ ] Parsing a 10k-line file takes < 50 ms in release (simple benchmark, recorded in the task).

## Follow-ups
