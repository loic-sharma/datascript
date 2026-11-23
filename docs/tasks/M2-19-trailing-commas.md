# M2-19 — Trailing commas in lists

**Status:** todo
**Depends on:** M1-13
**Crates:** `datascript-diagnostics`, `datascript-syntax`, `datascript-tests` (and `datascript-compiler` if IDE signature help needs a trailing-comma adjustment)
**Diagnostic codes:** none new; retires `DS0127` (never reuse a retired code).

## Goal
Allow an optional trailing `,` in every comma-separated list across DataScript:

- **Delimited lists**: function and reader call arguments (`case(...)`, `strcat(...)`, `read_csv(...)`), `in (...)` value lists, and `render ... with (...)` property lists.
- **Pipe-, semicolon-, paren-, or EOF-terminated operator lists**: `project`, `extend`, `project-away`, `project-rename`, `distinct <cols>`, `sort by` / `order by`, `join ... on`, and `union`.
- **`summarize` lists**: both the aggregate list before `by` and the grouping key list after `by`.

During interactive exploration, users routinely comment out the last line or reorder items in multi-line `project`, `extend`, `summarize`, `case(...)`, and `render ... with (...)` clauses. Accepting optional trailing commas removes that edit friction and produces cleaner single-line diffs. Today the parser explicitly rejects trailing commas with `DS0127` (and `DS0101` before `by` in `summarize`); this task accepts them and retires `DS0127`.

## Scope

### Parser (`crates/datascript-syntax/src/parser/grammar.rs`)
- [ ] Update `list()` to accept an optional trailing `COMMA` without reporting `DS0127` before list terminators (`PIPE`, `SEMICOLON`, `EOF`, `R_PAREN`, `R_BRACK`, and `LET_KW` when `p.nth(2) != EQ` so `from t | project a,\nlet x = 1` recovers cleanly as `DS0106` missing `;` without breaking `DS0126` on `project a, let = 1`). Because `p.tokens` contains only significant tokens, trailing commas followed by whitespace or `//` / `/* ... */` comments are handled automatically.
- [ ] Update `summarize_body()` (e.g., via a stop-predicate helper alongside `list()`) so the aggregate list accepts a trailing `,` immediately before the `by` keyword (`summarize sum(amount), by customerId`), consuming the trailing `COMMA` inside `SUMMARIZE_OP` before `BY_CLAUSE` starts.
  - Preserve the edge case where an output column is literally named `by`: `at_by()` is `self.at_kw("by") && self.nth(1) != EQ`, so checking lookahead `p.nth_at_kw(1, "by") && p.nth(2) != EQ` keeps `summarize sum(amount), by = count()` parsing `by = count()` as a named aggregate while treating `summarize sum(amount), by customerId` as a trailing comma before `by`.
  - Confirm `summarize count(), by` (missing key after `by`) still reports an expected expression after `by`, and `summarize , by customerId` and `summarize count(),, by customerId` remain syntax errors.
- [ ] Verify `render_prop()` and `render_body()`: `render_prop()`'s `while` loop (`while p.at(COMMA) && !(NAME_START.contains(p.nth(1)) && p.nth(2) == EQ) && p.nth(1) != R_PAREN`) already stops before `, )`, letting `list(p, render_prop)` consume the trailing `,` inside `RENDER_PROPS`. Confirm both single-value (`with (title = "x",)`) and multi-value (`with (ycolumns = a, b,)`) properties work, and that double commas (`with (ycolumns = a,, b)`, `with (title = "x",,)`, `with (ycolumns = a, b,, title = "x")`) and empty-list comma (`with (,)`) remain errors.
- [ ] Verify every other `list()` caller works cleanly with trailing commas: `project`, `extend`, `project-away`, `project-rename`, `sort by` / `order by`, `distinct <cols>`, `join ... on`, `union`, and `arg_list()` (calls, reader calls, `in (...)`, and `in [...]` error recovery still reporting `DS0128` without an extra error for a trailing comma).
- [ ] Confirm non-lists and malformed commas remain errors:
  - Empty-list comma: `f(,)`, `x in (,)`, `render table with (,)`, `project ,`, `summarize , by a`.
  - Leading comma: `f(, a)`, `project , a`, `summarize by , a`.
  - Consecutive / double commas: `f(a,, b)`, `f(a,,)`, `project a,, b`, `project a,,`, `summarize count(),, by a`.
  - Non-list constructs: `where a, b` (`DS0115`), `where a,`, `between (1 .. 10,)`, `top 5 by amount desc,`, `distinct *,` (`DS0116`), `take 5,`, `(a,)`, `a[0,]`, `long(null,)`.

### Retire `DS0127` (`datascript-diagnostics`, `datascript-syntax`, `docs/DIAGNOSTICS.md`)
- [ ] Remove `pub const DS0127` from `crates/datascript-diagnostics/src/codes.rs` (retired codes have no constant, matching `DS2007` and `DS2101`).
- [ ] Change `DS0127`'s entry in `REGISTRY` (`codes.rs`) to `code: Code::new(127)`, `severity: None`, `status: CodeStatus::Retired`, with a summary noting it was retired when trailing commas were allowed (citing the new `D-xxx` decision).
- [ ] Update `DS0127`'s row in `docs/DIAGNOSTICS.md` to match (`| DS0127 | — | *retired* (...) |`), keeping `codes::tests::registry_matches_diagnostics_md` and `registry_is_sorted_and_statuses_are_consistent` green. Never reuse `DS0127`.
- [ ] Remove `Parser::trailing_comma` from `crates/datascript-syntax/src/parser/targeted.rs`.

### AST/CST, compiler, and IDE consumers
- [ ] Verify AST accessors in `crates/datascript-syntax/src/ast.rs` and downstream consumers (`datascript-compiler`, `datascript-lsp`, `datascript-datafusion`, `datascript-engine`) ignore the optional trailing `COMMA` token cleanly (the CST already placed the trailing `COMMA` token in the list node under `DS0127`, so node shapes are unchanged).
- [ ] Check `crates/datascript-compiler/src/ide/signature.rs` (`let written = commas.len() + 1;`) and `crates/datascript-compiler/src/ide/complete.rs` with trailing commas in calls, reader options, `render with (...)`, and operator lists; adjust if a trailing comma causes `signature_help` to pick the wrong overload when the cursor is on an earlier argument (e.g. `round(amount,)` with cursor on `amount`).

### Spec, decisions, and formatter handoff
- [ ] Update `docs/LANGUAGE.md` (§8 Grammar sketch and any relevant list prose) to document optional trailing commas in comma-separated lists.
- [ ] Record the design decision in `docs/DECISIONS.md` (next free `D-xxx` ID): rationale, list positions covered (including `summarize ... , by`), non-list exclusions, and retirement of `DS0127`.
- [ ] Formatter handoff (`M2-07`): when M2-19 lands, if `M2-07` has not landed yet, update `docs/tasks/M2-07-formatter.md` with a `(from M2-19)` item (and remove any "no trailing commas (`DS0127`)" rule/example) so `ds fmt` preserves trailing commas (whitespace-only formatting); if `M2-07` has already landed, update `ds fmt` and its tests directly to preserve trailing commas.

### Golden tests (`tests/cases/`)
- [ ] Remove or replace `tests/cases/errors/syntax/ds0127_trailing_comma_before_pipe.dstest` and `tests/cases/errors/syntax/ds0127_trailing_comma_in_call.dstest` (both queries are now valid).
- [ ] Add `mode: syntax` golden cases covering trailing commas in:
  - Delimited lists: function calls (`strcat(...)`, `case(...)`), reader calls (`read_csv('x.csv', delimiter = ';',)`), `in (...)`, and `render ... with (...)` (single-value and multi-value properties like `ycolumns = a, b,`).
  - Operator lists: `project`, `extend`, `project-away`, `project-rename`, `distinct <cols>`, `sort by` / `order by`, `join ... on`, `union`, and inside a parenthesized pipeline `(from t | project a,)`.
  - `summarize`: trailing comma before `by`, trailing comma at the end of `by`, both together, and `summarize sum(amount), by = count()` (column named `by`).
  - Multi-line lists with trailing commas and comments (`// last item`).
- [ ] Add `mode: check` and `mode: run` golden cases verifying queries with trailing commas bind and execute cleanly end to end (across operator lists, `summarize ... , by ... ,`, and calls such as `case(...)`, `in (...)`, `read_csv(...)`).
- [ ] Add negative `mode: syntax` golden cases confirming `(,)`, `f(, a)`, `a,, b`, `a,,`, `project , a`, `summarize , by a`, `summarize count(),, by a`, `where a, b` (`DS0115`), `where a,`, `between (1 .. 10,)`, `top 5 by amount desc,`, and `distinct *,` (`DS0116`) still produce clean errors.

## Acceptance criteria
- [ ] Optional trailing commas parse without errors in every comma-separated list (delimited `(...)` lists, operator lists before `|` / `;` / `)` / EOF, and `summarize` aggregates before `by` as well as `by` keys), including when followed by whitespace or comments.
- [ ] `summarize sum(amount), by = count()` still parses `by = count()` as a named aggregate rather than a `by` clause.
- [ ] Empty-list commas (`(,)`), leading commas (`, a`), double commas (`a,, b`, `a,,`), and commas in non-list constructs (`where a, b` / `DS0115`, `where a,`, `between (1 .. 10,)`, `top 5 by a,`, `distinct *,` / `DS0116`) still produce clean errors.
- [ ] `DS0127` is retired (`CodeStatus::Retired`, no constant, `severity: None`) in `crates/datascript-diagnostics/src/codes.rs` and `docs/DIAGNOSTICS.md`, `Parser::trailing_comma` is removed from `targeted.rs`, and the diagnostics registry tests pass.
- [ ] Binder, IDE (`signature_help` / `complete`), planner, and DataFusion runtime handle trailing commas cleanly, covered by `mode: syntax`, `mode: check`, and `mode: run` golden tests.
- [ ] `docs/LANGUAGE.md` and `docs/DECISIONS.md` are updated in the same change.
- [ ] Formatter handoff is completed (`docs/tasks/M2-07-formatter.md` updated with a `(from M2-19)` item if `M2-07` is still `todo`, or `ds fmt` updated directly if `M2-07` has landed).
- [ ] `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, and `cargo check -p datascript-syntax -p datascript-compiler --target wasm32-unknown-unknown` pass.

## Follow-ups
