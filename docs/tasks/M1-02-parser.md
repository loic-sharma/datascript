# M1-02 — Parser and syntax tree

**Status:** done — event-based recursive-descent parser over a lossless rowan CST with recovery, typed AST (`ast::*`), CST dump for `mode: syntax`, 47 golden cases, proptest fuzzing and a 10k-line benchmark (6–7 ms).
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
  (Landed as `parse(text, file: FileId)`, like `lex`, so diagnostic spans name their file.)

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
- [x] Golden `syntax` cases for every operator and expression form, including precedence corner
      cases (`not a == b`, `a - -b`, `x between (1 .. 10)`), `| project-away x`, `| project - away x`
      (error), and `long(null)`. (`tests/cases/syntax/`, 47 cases.)
- [x] Recovery cases: two broken operators in one pipeline produce two errors and a usable tree
      for the rest. (`recovery_two_operators`, `recovery_missing_expression`.)
- [x] Fuzz/property test: arbitrary input never panics, and the tree text equals the input.
      (`crates/datascript-syntax/tests/parser_props.rs`: random text and grammar fragments, also
      checks spans and walks every AST accessor; run once with 200k cases. Hostile nesting such
      as 100,000 `(` gives one DS0100 instead of a stack overflow.)
- [x] Parsing a 10k-line file takes < 50 ms in release (simple benchmark, recorded in the task).
      `cargo run --release -p datascript-syntax --example parse_bench`: 10,010 lines (483 KB),
      lex + parse in **5.9–7.3 ms** (best of 5, Apple Silicon, 2026-11-20).

## Notes
- Design choices are recorded in D-029: table positions parse as any expression (the binder
  rejects non-tables), a query statement is always a `PIPELINE`, `not(` binds like a call,
  contextual keywords stay `IDENT` tokens, and the recovery points.
- DS0100 (expression nested too deeply, limit 256) was allocated from this task's block.
- The re-blessed `tests/cases/lexer/` cases now show CSTs, and the token soups among them also
  show generic parse errors. That is expected; they exist to show tokens.
- `PipelineDriver` now answers `Driver::output_order_defined` from the parse (TESTING.md).
- `crates/datascript-syntax/examples/dump.rs` prints the CST and diagnostics of a string, handy
  for M1-03.

## Follow-ups
- M1-03 replaces generic DS0101 messages with targeted codes. The places are easy to find in
  `parser/grammar.rs`: `=` in a predicate (now "expected `|` or `;`, found `=`"), `project - away`
  (now a trailing-token error), operator at a line start without `|` ("expected `|` before the
  next operator", DS0102 with a fix), `;` between statements (DS0106), `| ` with no operator
  (DS0103), unknown operator did-you-mean (DS0104), missing `by`/`on` (DS0121/DS0122),
  `&&`/`||`/`<>`/`!x`, and `render` not last (DS0120, not checked yet).
- M2 operators (`lookup`, `mv-expand`, `parse`, `make-series`, `sample`, `as`, `project-reorder`,
  `print`) parse as an unknown-operator error with a "planned" note; their tasks add them.
- M2-03: `a['key']` lexes the index as a `QUOTED_IDENT`, so it does not parse as indexing yet;
  `dynamic([...])` bodies and `*` as an argument (`arg_max(x, *)`) are not parsed.
