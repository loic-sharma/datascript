# M2-19 — Trailing commas in lists

**Status:** done — optional trailing commas accepted in all comma-separated lists (including `summarize ... , by`), `DS0127` retired (D-059), IDE signature help and completion updated, spec/decisions/M2-07 handoff and golden tests added.
**Depends on:** M1-13
**Crates:** `datascript-diagnostics`, `datascript-syntax`, `datascript-compiler`, `datascript-tests`
**Diagnostic codes:** none new; retires `DS0127` (never reuse a retired code).

## Goal
Allow an optional trailing `,` in every comma-separated list across DataScript:

- **Delimited lists**: function and reader call arguments (`case(...)`, `strcat(...)`, `read_csv(...)`), `in (...)` value lists, and `render ... with (...)` property lists.
- **Pipe-, semicolon-, paren-, or EOF-terminated operator lists**: `project`, `extend`, `project-away`, `project-rename`, `distinct <cols>`, `sort by` / `order by`, `join ... on`, and `union`.
- **`summarize` lists**: both the aggregate list before `by` and the grouping key list after `by`.

During interactive exploration, users routinely comment out the last line or reorder items in multi-line `project`, `extend`, `summarize`, `case(...)`, and `render ... with (...)` clauses. Accepting optional trailing commas removes that edit friction and produces cleaner single-line diffs. Today the parser explicitly rejects trailing commas with `DS0127` (and `DS0101` before `by` in `summarize`); this task accepts them and retires `DS0127`.

**Parallel-safety / ordering note:** M2-19 edits `list()` in `crates/datascript-syntax/src/parser/grammar.rs` and should not run in parallel with `M2-01`, `M2-03`, `M2-04`, or `M2-05`. Prefer landing M2-19 first so those tasks build their new comma-separated lists directly on the trailing-comma `list()` helper (each has a `(from M2-19)` item in its `Carried over` section).

## Scope

### Parser (`crates/datascript-syntax/src/parser/grammar.rs`)
- [x] Update `list()` to accept an optional trailing `COMMA` without reporting `DS0127` before list terminators:
  - `PIPE`, `SEMICOLON`, `EOF`, `R_PAREN`, `R_BRACK`.
  - `LET_KW` when `p.nth(2) != EQ`, so `from t | project a,\nlet x = 1` recovers cleanly as `DS0106` (missing `;`) without breaking `DS0126` on `project a, let = 1`.
  - A line-initial statement-starting `from` at `depth == 0` (`p.depth == 0 && p.nth_line_start(1) && p.nth_at_kw(1, "from") && EXPR_START.contains(p.nth(2)) && !matches!(p.nth(2), MINUS | NOT_KW)`, matching D-037), so `from 'a.csv' | project a,\nfrom 'b.csv'` recovers cleanly as `DS0106` (missing `;`) instead of a confusing `DS0101` on `'b.csv'`, while keeping a bare identifier on the next line (`a,\nb`), a column named `from` (`a,\nfrom | ...` or `a,\nfrom - 1`), and a pipeline argument inside a call (`f(a,\nfrom 'b.csv')`) as list continuations.
  - Because `p.tokens` contains only significant tokens, trailing commas followed by whitespace or `//` / `/* ... */` comments are handled automatically.
- [x] Update `summarize_body()` (e.g., via a stop-predicate helper alongside `list()`) so the aggregate list accepts a trailing `,` immediately before the `by` keyword (`summarize sum(amount), by customerId`), consuming the trailing `COMMA` inside `SUMMARIZE_OP` before `BY_CLAUSE` starts.
  - Preserve the edge case where an output column is literally named `by`: `at_by()` is `self.at_kw("by") && self.nth(1) != EQ`, so checking lookahead `p.nth_at_kw(1, "by") && p.nth(2) != EQ` keeps `summarize sum(amount), by = count()` parsing `by = count()` as a named aggregate while treating `summarize sum(amount), by customerId` as a trailing comma before `by`.
  - Confirm `summarize count(), by` (missing key after `by`) still reports an expected expression after `by`, and `summarize , by customerId` and `summarize count(),, by customerId` remain syntax errors.
- [x] Catch `type(null,)` in `at_typed_null()` / `primary()` (lines ~1655–1708) so `long(null,)` stays a syntax error (`DS0101`: expected `)`) instead of falling through to `arg_list()` and producing a misleading `DS1004: unknown function 'long'` (with "did you mean `log`?"). Specifically, `at_typed_null(p)` currently requires `p.nth(3) == R_PAREN`; matching `p.nth(3) == R_PAREN || (p.nth(3) == COMMA && p.nth(4) == R_PAREN)` and finishing the `TYPED_NULL` branch with `expect_closing(p, open)` after bumping `type`, `(`, `null` reports `DS0101` on `,` while keeping `long(x)` and `long (null)` as calls (`tests/cases/syntax/typed_null.dstest`).
- [x] Verify `render_prop()` and `render_body()`: `render_prop()`'s `while` loop (`while p.at(COMMA) && !(NAME_START.contains(p.nth(1)) && p.nth(2) == EQ) && p.nth(1) != R_PAREN`) already stops before `, )`, letting `list(p, render_prop)` consume the trailing `,` inside `RENDER_PROPS`. Confirm both single-value (`with (title = "x",)`) and multi-value (`with (ycolumns = a, b,)`) properties work, and that double commas (`with (ycolumns = a,, b)`, `with (title = "x",,)`, `with (ycolumns = a, b,, title = "x")`) and empty-list comma (`with (,)`) remain errors.
- [x] Verify every other `list()` caller works cleanly with trailing commas: `project`, `extend`, `project-away`, `project-rename`, `sort by` / `order by`, `distinct <cols>`, `join ... on`, `union`, and `arg_list()` (calls, reader calls, `in (...)`, and `in [...]` error recovery still reporting `DS0128` without an extra error for a trailing comma).
- [x] Confirm non-lists and malformed commas remain errors with pinned diagnostic codes:
  - Empty-list comma (`DS0101`): `f(,)`, `x in (,)`, `render table with (,)`, `project ,`, `summarize , by a`.
  - Leading comma (`DS0101`): `f(, a)`, `project , a`, `summarize by , a`.
  - Consecutive / double commas (`DS0101`): `f(a,, b)`, `f(a,,)`, `project a,, b`, `project a,,`, `summarize count(),, by a`.
  - Non-list constructs: `where a, b` (`DS0115`), `where a,` (`DS0101`), `top 5 by amount desc,` (`DS0101`), `take 5,` (`DS0101`), `between (1 .. 10,)` (`DS0101`), `(a,)` (`DS0101`), `a[0,]` (`DS0101`), `distinct *,` (`DS0116`), `long(null,)` (`DS0101`).

### Retire `DS0127` (`datascript-diagnostics`, `datascript-syntax`, `docs/DIAGNOSTICS.md`)
- [x] Remove `pub const DS0127` from `crates/datascript-diagnostics/src/codes.rs` (retired codes have no constant, matching `DS2007` and `DS2101`).
- [x] Change `DS0127`'s entry in `REGISTRY` (`codes.rs`) to `code: Code::new(127)`, `severity: None`, `status: CodeStatus::Retired`, with summary `"*retired* (was: trailing comma at the end of a list; D-0xx allows them)"` (not `never implemented`, since `DS0127` was implemented).
- [x] Update `docs/DIAGNOSTICS.md`:
  - Add `DS0127 (D-0xx)` to the `**Retired codes**:` line.
  - Update `DS0127`'s row in the Code registry table to `| DS0127 | — | *retired* (was: trailing comma at the end of a list; D-0xx allows them) |`, keeping `codes::tests::registry_matches_diagnostics_md` and `registry_is_sorted_and_statuses_are_consistent` green. Never reuse `DS0127`.
- [x] Remove `Parser::trailing_comma` from `crates/datascript-syntax/src/parser/targeted.rs`.

### AST/CST, compiler, and IDE consumers (`datascript-compiler`)
- [x] Verify AST accessors in `crates/datascript-syntax/src/ast.rs` and downstream consumers (`datascript-compiler`, `datascript-lsp`, `datascript-datafusion`, `datascript-engine`) ignore the optional trailing `COMMA` token cleanly (the CST already placed the trailing `COMMA` token in the list node under `DS0127`, so node shapes are unchanged).
- [x] Update `written` in `crates/datascript-compiler/src/ide/signature.rs` (currently `let written = commas.len() + 1;`) to count actual argument nodes so a trailing `,` directly before `)` does not add an extra argument unless the cursor is after that comma (`written.max(active + 1)` already handles the cursor being after the comma). As a result, `round(am|ount,)` picks the 1-arg overload (`round(x)`), while `round(amount, |)` highlights parameter 1 (`digits`) and prefers the 2-arg overload (`round(x, digits)`). Add unit tests in `crates/datascript-compiler/src/ide/tests.rs` (or `signature.rs`) covering both cursor positions.
- [x] Confirm `crates/datascript-compiler/src/ide/complete.rs` (which already treats "after a `COMMA`" as "next item") works after a trailing comma in function/reader calls, operator lists (`project`, `union`, `project-away`, `distinct`), and `render ... with (...)`, and add unit tests in `crates/datascript-compiler/src/ide/tests.rs`.

### Spec, decisions, and formatter handoff
- [x] Update `docs/LANGUAGE.md` (§8 Grammar sketch and relevant operator/call sections) to document optional trailing commas in comma-separated lists, explicitly marked **≠ KQL** (KQL rejects trailing commas, so queries using them cannot be pasted verbatim into Kusto; see LANGUAGE.md intro and D-014).
- [x] Record the design decision in `docs/DECISIONS.md` (next free `D-xxx` ID): rationale, list positions covered (including `summarize ... , by`), non-list exclusions (including `long(null,)`), the **≠ KQL** note, and retirement of `DS0127`.
- [x] Formatter handoff (`M2-07`): when M2-19 lands, check `docs/tasks/M2-07-formatter.md` and the codebase (not the `docs/tasks/README.md` index row, which may be stale on `main`).
  - If `M2-07` has not landed yet, update `docs/tasks/M2-07-formatter.md` with a `(from M2-19)` item (and remove any "no trailing commas (`DS0127`)" rule/example).
  - If `M2-07` has already landed, update `ds fmt` and its tests directly.
  - Either way, specify/implement that the whitespace-only formatter never adds or removes a trailing `,` (preserving an existing one verbatim), and that its wrapping and single-item-hugging rules handle an existing trailing `,` before `)` or at the end of an operator list by keeping the `,` attached to the last item's line before the closing `)` or the next `|`.

### Golden tests (`tests/cases/`)
- [x] Remove or replace `tests/cases/errors/syntax/ds0127_trailing_comma_before_pipe.dstest` and `tests/cases/errors/syntax/ds0127_trailing_comma_in_call.dstest` (both queries are now valid).
- [x] Add `mode: syntax` golden cases covering trailing commas in:
  - Delimited lists: function calls (`strcat(...)`, `case(...)`), reader calls (`read_csv('x.csv', delimiter = ';',)`), `in (...)`, and `render ... with (...)` (single-value and multi-value properties like `ycolumns = a, b,`).
  - Operator lists: `project`, `extend`, `project-away`, `project-rename`, `distinct <cols>`, `sort by` / `order by`, `join ... on`, `union`, and inside a parenthesized pipeline `(from t | project a,)`.
  - `summarize`: trailing comma before `by`, trailing comma at the end of `by`, both together, and `summarize sum(amount), by = count()` (column named `by`).
  - Multi-line lists with trailing commas and comments (`// last item`).
  - Statement-boundary recovery: `from 'a.csv' | project a,\nfrom 'b.csv'` (`DS0106`) and `from 'a.csv' | project a,\nlet x = 1` (`DS0106`).
- [x] Add `mode: check` and `mode: run` golden cases verifying queries with trailing commas bind and execute cleanly end to end (across operator lists, `summarize ... , by ... ,`, and calls such as `case(...)`, `in (...)`, `read_csv(...)`).
- [x] Add negative `mode: syntax` golden cases confirming `(,)`, `f(, a)`, `a,, b`, `a,,`, `project , a`, `summarize , by a`, `summarize count(),, by a`, `where a, b` (`DS0115`), `where a,` (`DS0101`), `between (1 .. 10,)` (`DS0101`), `top 5 by amount desc,` (`DS0101`), `take 5,` (`DS0101`), `(a,)` (`DS0101`), `a[0,]` (`DS0101`), `distinct *,` (`DS0116`), and `long(null,)` (`DS0101`) still produce their expected errors.

## Acceptance criteria
- [x] Optional trailing commas parse without errors in every comma-separated list (delimited `(...)` lists, operator lists before `|` / `;` / `)` / EOF, and `summarize` aggregates before `by` as well as `by` keys), including when followed by whitespace or comments.
- [x] `summarize sum(amount), by = count()` still parses `by = count()` as a named aggregate rather than a `by` clause; `from 'a.csv' | project a,\nfrom 'b.csv'` recovers cleanly as `DS0106`.
- [x] Empty-list commas (`(,)`), leading commas (`, a`), double commas (`a,, b`, `a,,`), and commas in non-list constructs (`where a, b` / `DS0115`, `where a,` / `DS0101`, `between (1 .. 10,)` / `DS0101`, `top 5 by a,` / `DS0101`, `take 5,` / `DS0101`, `(a,)` / `DS0101`, `a[0,]` / `DS0101`, `distinct *,` / `DS0116`, `long(null,)` / `DS0101`) produce their expected syntax errors (never falling through to `DS1004` for `long(null,)`).
- [x] `DS0127` is retired (`CodeStatus::Retired`, no constant, `severity: None`, `*retired* (was: …)`) in `crates/datascript-diagnostics/src/codes.rs` and `docs/DIAGNOSTICS.md` (both the `**Retired codes**:` list and the Code registry table), `Parser::trailing_comma` is removed from `targeted.rs`, and the diagnostics registry tests pass.
- [x] `signature_help` picks the right overload for both `round(am|ount,)` and `round(amount, |)`, and `complete` works after trailing commas in calls, operator lists, and `render ... with (...)`, each covered by unit tests.
- [x] Binder, planner, and DataFusion runtime handle trailing commas cleanly, covered by `mode: syntax`, `mode: check`, and `mode: run` golden tests.
- [x] `docs/LANGUAGE.md` (with **≠ KQL** marks) and `docs/DECISIONS.md` are updated in the same change.
- [x] Formatter handoff is completed after inspecting `docs/tasks/M2-07-formatter.md` and the codebase (`M2-07-formatter.md` updated with a `(from M2-19)` item if `M2-07` has not landed, or `ds fmt` updated directly to preserve trailing commas and handle wrapping/single-item hugging if it has).
- [x] `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, and `cargo check -p datascript-syntax -p datascript-compiler --target wasm32-unknown-unknown` pass.

## Follow-ups
