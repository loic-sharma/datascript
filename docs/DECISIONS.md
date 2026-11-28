# Decisions

A lightweight decision log. Add an entry when you make a choice someone might later question.
Status is one of: **accepted**, **proposed** (open for the maintainer to confirm), or **superseded by D-xxx**.

## Accepted

### D-001 — Hand-written recursive-descent parser over a lossless rowan CST
Error messages and recovery are the top priority, and the LSP and formatter need a lossless tree.
Parser generators (lalrpop, pest) make targeted messages and recovery hard. Prior art: rust-analyzer, Biome.

### D-002 — Engine-neutral logical plan IR in the compiler
The compiler lowers to its own small relational IR instead of building DataFusion plans or SQL
directly. This keeps DataFusion out of the core crates (WASM, pluggable backends), lets the
compiler check engine capabilities before running, and gives `ds explain` something stable to print.

### D-003 — Arrow is the result interchange format
Every engine returns Arrow RecordBatches, and every frontend consumes Arrow (Arrow IPC across the
WASM boundary). Most candidate engines already speak Arrow.

### D-004 — `join` defaults to `kind = inner`; no `innerunique`
KQL's default `innerunique` deduplicates the left side silently, which surprises users and would
need a dedicated diagnostic to explain. DataScript uses SQL-like `inner`. Using `innerunique` gives
an error (`DS0124`) that explains the divergence and how to get the same effect (`distinct` on the
left first).

### D-006 — DuckDB-style file sources
A string literal at the head of a pipe is a file path or glob (`'data/*.parquet' | ...`), with
`read_csv`/`read_parquet`/`read_json` for explicit options. Relative paths resolve against the
script's directory. Chosen by the maintainer. *(D-036: the path now follows `from`:
`from 'data/*.parquet' | ...`.)*

### D-007 — `=` is never comparison; `let` cannot shadow
Both choices exist to make errors clearer: `=` in a predicate is always a mistake we can fix
automatically, and shadowing a `let` is a common source of confusion in long scripts.

### D-008 — Binary name `ds`, crates `datascript-*`
Short to type. If `ds` turns out to clash with common tools, packaging can also install `datascript`.

### D-009 — Dual MIT / Apache-2.0 license
Rust ecosystem convention, compatible with DataFusion (Apache-2.0).

### D-010 — A `let` bound to a bare string is tabular if it is used as a table *(superseded by D-020)*
`let orders = 'data/*.parquet';` and `let name = "Alice";` have the same syntax. The parser
produces one `LetStmt` node and the binder decides: the `let` is **tabular** (a source) if any use
of the name is in a table position (pipe head, right side of `join`/`union`); otherwise it is a
**scalar** string. A name used both ways gets `DS2007`, with labels on both uses. This keeps the
README style working without a separate keyword. Alternatives rejected: a path-shaped heuristic
(extension or glob characters) is fragile; requiring `read_parquet(...)` for every tabular let
is noisy. *(Design review, 2026-11-20.)*

### D-011 — `Engine` futures are `Send` on native targets, `?Send` on wasm32
`axum` handlers and the multi-threaded tokio runtime used by the LSP require `Send` futures, while
DataFusion on wasm32 is single-threaded and its futures are not `Send`. The trait is declared once
and the bound is chosen per target (`trait-variant` or
`#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]` / `#[cfg_attr(not(...), async_trait)]`).
Frontends on native may assume `Send + Sync` engines. *(Design review, 2026-11-20.)*

### D-012 — Source columns are widened to DataScript types on read
Real files rarely match the canonical Arrow types: pandas writes `Timestamp(ns)`, recent DataFusion
reads Parquet strings as `Utf8View`, CSV inference yields `Timestamp(s|ms|ns)` and `Date32`, and
Parquet commonly has `Int16`, `UInt32`, `Float32` and decimals. The table in
[LANGUAGE.md §5.1](LANGUAGE.md#51-reading-arrow-types) maps every common Arrow type to a DataScript
type; the backend casts at the scan. Lossy cases (`UInt64` above `i64::MAX`, nanosecond precision)
are documented there. Only types with no mapping (structs, lists, maps before M2) stay
projection-only. *(Design review, 2026-11-20.)*

### D-013 — Keywords are contextual
Words like `by`, `on`, `kind`, `with`, `asc`, `desc`, `count`, `order` and every operator name are
ordinary identifiers except where the grammar expects them, because columns named `order`, `kind`,
`count` and `type` are common. Only `let`, `true`, `false`, `null`, `and`, `or`, `not` are reserved.
See [LANGUAGE.md §1.1](LANGUAGE.md#11-keywords). *(Design review, 2026-11-20.)*

### D-014 — Scalar semantics follow KQL unless marked ≠ KQL
Where LANGUAGE.md and the function catalog are silent, a function or operator behaves as in KQL.
Explicit choices: regular expressions use the Rust `regex` syntax (RE2-like: no backreferences or
lookaround); `long / long` is integer division truncating toward zero; integer division or modulo by
zero is a runtime error (`DS4003`); integer overflow is a runtime error; `startofweek` starts on
Sunday (as in KQL). Backends that cannot match a rule emulate it or list it in
`Capabilities::deviations`, and golden cases record the difference in an `---- expected[<engine>]`
section. Checked overflow and division by zero are a deliberate cost: DataFusion wraps by default,
so M1-09 needs checked arithmetic, and SQL engines differ (M6-03). Whether KQL itself errors here
is open (Q-008). *(Design review, 2026-11-20; amended in design review 2.)*

### D-015 — `now()` is evaluated once per program run
All statements in one `ds run` (or one REPL submission, or one web UI Run) see the same `now()`,
so `let cutoff = ago(1d)` means the same instant in every statement. *(Design review, 2026-11-20.)*

### D-016 — `ds ui` confines file sources to its root directory
`GET /api/files` blocking path traversal is pointless if `/api/run` can read `'/etc/passwd'`. In
`ds ui`, every path a query resolves (literal, glob, reader call, host table) must canonicalize to a
location under the root; anything else is `DS3007`. `--allow-outside-root` lifts this for users who
want it. The CLI and REPL are not sandboxed (the user runs them on their own files).
*(Design review, 2026-11-20.)*

### D-017 — Golden tests run single-partition and order-insensitive by default
DataFusion returns rows in no fixed order without a `sort`, and parallel float aggregation can
change the last digits. The test driver runs with `target_partitions = 1`, and `order` defaults to
`any` unless every query's output order is defined (its last `sort` or `top` is followed only by
order-preserving operators, LANGUAGE.md §3.4). See
[TESTING.md](TESTING.md#header-keys). *(Design review, 2026-11-20.)*

### D-018 — Diagnostic codes are reserved per task before use
Parallel tasks used to pick the "next free" code and collide. Each task file lists the code block it
owns (e.g. `DS0120–DS0139` for M1-03); the registry in DIAGNOSTICS.md records the reservation, and
`codes.rs` is the source of truth once it exists. *(Design review, 2026-11-20.)*

### D-005 — `sort by` defaults to ascending, and omitting the direction warns
KQL defaults `sort` to descending, which surprises nearly everyone, so DataScript sorts ascending
by default. `top` stays descending by default, because top-N is about the largest values. The cost
is that a ported KQL query silently reverses, and that `sort by x | take 10` is not
`top 10 by x`. To make that visible, a `sort` key without `asc` or `desc` is warning `DS2103`
(M1-14), with a machine-applicable fix inserting `asc`; the LSP offers it as a quick fix. This
closes Q-001. *(Design review 2, 2026-11-20.)*

### D-019 — `join ... on key` produces one key column
With KQL's rule, `orders | join topCustomers on customerId` (the README example) outputs both
`customerId` and `customerId1`, identical for an inner join, plus a `DS1007` info. DataScript
follows SQL `USING` instead: `on key` yields one `key` column in the left column's position
(`rightouter` takes the right value, `fullouter` coalesces). `on $left.a == $right.b` keeps both
columns. Other name collisions still get KQL's `name1` renaming and `DS1007`. ≠ KQL. See
[LANGUAGE.md §3.3](LANGUAGE.md#33-column-naming). *(Design review 2, 2026-11-20.)*

### D-020 — String `let`s are scalars; a constant string in table position is a path
Supersedes D-010. Under D-010 the meaning of `let p = 'x.csv'` depended on later uses, and a
natural script (`read_csv(p, delimiter = ';')` plus `p | take 5`) was an error (`DS2007`). Now the
binder decides a `let`'s kind from its right side alone; a string `let` is a scalar. Wherever a
table is expected, a constant string (literal or a scalar `let` bound to one) is read as a file
source. The README style keeps working, hover can show a `let`'s type before any use, and there
is no inference to replicate. `DS2007` is retired before it was ever implemented. *(Design review 2, 2026-11-20.)*
*(D-036 changes how a `let`'s kind is decided: by the syntax, not the binder. A `let` is tabular
when its value is a `PIPELINE`; "a tabular name on the right side makes the `let` tabular" no
longer holds, so `let alias = orders;` is a scalar `let`. The rule for constant strings in table
positions stands, with "pipe head" now meaning the operand of `from`.)*

### D-021 — All integers are read as `long`, and integer arithmetic yields `long`
With `Int8`–`Int32` columns read as `int` and `int * int` staying `int`, everyday Parquet data
(`qty * price_cents`) would overflow at 2³¹ and raise `DS4003`. Now every integer source column
widens to `long` at the scan, integer arithmetic always returns `long`, and `sum` of integers is
`long`. `int` remains as a type for `toint()` and `int(null)` (KQL compatibility). *(Design review 2, 2026-11-20.)*

### D-022 — Sources are resolved by re-binding, not by a pre-pass
The first design walked the tree in a separate `collect_sources` pass before binding. That pass
had to repeat name resolution, `let` handling and constant folding, and would drift from the
binder (more so with UDFs, M2-05). Now the binder asks a synchronous, caching `SchemaProvider`,
which answers `Pending` on a miss; the binder records `pending_sources`, and the session fetches
them and binds again. Because sources depend only on constants, two passes suffice. See
[ARCHITECTURE.md § Source resolution](ARCHITECTURE.md#semantics-datascript-compiler). *(Design review 2, 2026-11-20.)*

### D-023 — Engines report source problems as structured `SourceError`s
`Engine::schema_of` used to return a `Diagnostic`, but the engine cannot know the span, and one
source can appear in several places. It now returns a structured `SourceError` (`NotFound { nearby }`,
`Conflict { files, column, types }`, …). The compiler builds the DS3xxx diagnostic, with the span,
help and wording rules from DIAGNOSTICS.md, in one place for all backends. *(Design review 2, 2026-11-20.)*

### D-024 — Ambiguous Arrow types carry canonical extension types
`dynamic` and `string` are both stored as Utf8, and `guid` is a plain `FixedSizeBinary(16)`, so
frontends could not tell them apart. Result fields use Arrow's canonical extension types
(`arrow.json`, `arrow.uuid`) in field metadata. If Q-003 picks the Arrow Variant type, its
canonical extension type replaces `arrow.json`. *(Design review 2, 2026-11-20.)*

### D-025 — JSON diagnostics count positions in UTF-16 code units
Every consumer of the JSON form is JavaScript (CodeMirror, the WASM API, VS Code webviews), and
JavaScript strings index in UTF-16. Byte offsets would force every consumer to re-encode the
source. The JSON form carries UTF-16 offsets and 1-based line/column only; the LSP conversion
already needs the same mapping. *(Design review 2, 2026-11-20.)*

### D-026 — The lexer is context-free; the parser joins hyphenated operator names
The first design lexed `project-away` as one token by tracking the previous non-trivia token.
Now the lexer always emits `project` `-` `away`, and the parser joins them right after `|` when
they are adjacent with no trivia. Incremental relexing then never depends on earlier tokens, and
the TextMate grammar (M3-04) can follow the same rule. `| project - away` is `DS0104` with a
suggestion. *(Design review 2, 2026-11-20.)*

### D-027 — Diagnostic renderers are hand-written
The terminal and plain formats in DIAGNOSTICS.md and TESTING.md are a contract: every golden test
compares plain output byte for byte. `annotate-snippets` and `ariadne` each have their own layout
(ariadne draws labels differently; neither emits the exact `┌─`/`-->` header, gutter and `= help:`
lines shown in the docs) and a dependency bump could re-render every `.dstest` file. The renderer
in `datascript-diagnostics` is under 600 lines including tests, has no dependencies, builds for wasm32, and tests
check its output against the examples in DIAGNOSTICS.md and TESTING.md verbatim. Two details
follow from it: text renderers print only the help *message* (a `Suggestion`'s edits are for the
JSON form and the LSP, so the help text spells out the rewrite, as in the DS0110 example), and a
location-less diagnostic carries an optional `file` so renderers can name the file (style rule 11).
*(M0-02, 2026-11-20.)*

### D-028 — Lexical details settled by the lexer
M1-01 had to pin down cases LANGUAGE.md §1 left open. Strings (normal and verbatim) end at the end
of their line, so an unterminated string spoils one line, not the rest of the file. A malformed
literal keeps its literal kind (`1x` is a `TIMESPAN` token with DS0003); only text that starts no
token becomes `ERROR`, so the parser sees an expression where the user meant one and does not
cascade. `literal::*` re-reads values for later stages, which treat an `Err` as already reported.
`['x']` is one token only with no trivia inside the brackets, so `['a', 'b']` stays a list (M2
`dynamic` literals must read a lone `["a"]` inside `dynamic(...)` as a one-element array).
Non-ASCII letters in an unquoted name and SQL-style `` `name` `` are DS0001 with a quoting fix
rather than a run of errors. Unicode white space (no-break space, BOM) is white space, but
zero-width characters are DS0001, since they are invisible and almost always pasted by accident.
*(M1-01, 2026-11-20.)*

### D-029 — Parser shape: permissive table positions, `not(` binds like a call
M1-02 settled what LANGUAGE.md §8 leaves to the parser. **Table positions are any expression.** A
source, the right side of `join`/`union`, and a `let` value parse as a scalar expression; a
following `|` wraps it in a `PIPELINE` node. The binder, which knows names and types, rejects an
expression that is not a table (D-020 already makes it decide what a `let` is), so the parser does
not need a second, narrower grammar, and the binder's errors can say *why* (`x + 1` is a `long`,
not a table). A query statement is always a `PIPELINE` node, even with no operators. `join ... on`
conditions are expressions too (`$left.a == $right.b` is a `BIN_EXPR` of `FIELD_EXPR`s), so M1-03
can point at `=` in them. **`not` followed by `(` reads as a call**, as in KQL: `not(x) == y` is
`(not x) == y`, while `not x == y` is `not (x == y)` (§4 level 3). **Contextual keywords stay
`IDENT` tokens** in the tree; nodes say what they are. **Trivia** belongs to the innermost node
open before the next significant token, so every node starts and ends at a significant token.
**Recovery** resynchronizes at `|`, `;`, `let`, `)` inside brackets, an operator name at the
start of a line (a forgotten `|`), and an expression at the start of a line outside brackets (a
forgotten `;`). One token gets at most one error, and the parser never reports at a token the
lexer already rejected. **Nesting** is limited to 256 levels of expressions (`DS0100`), so the
recursive parser cannot overflow the stack on hostile input. *(M1-02, 2026-11-20.)*
*(D-036 changes the shape: `from source` is always a `PIPELINE` node, with or without a `|`, and a
`let`'s kind is read from that node rather than decided by the binder. The operand of `from` and
the right side of `join`/`union` are still any expression.)*

### D-030 — Syntax habits parse as what the user meant
M1-03 settled how targeted syntax diagnostics (DS0102–DS0124) shape the tree. **Habit tokens
parse as the operator meant and are reported once**: `&&`, `||`, `<>` and `=` (where a predicate
is expected) are binary operators with the precedence of `and`, `or`, `!=` and `==`, and `!x` is a
prefix `not`; the typed AST reads them as those operators, so later stages see a usable tree and
add no second error. `=` counts as a comparison only in a `where` predicate, a `join ... on`
condition, and the arguments of `iff`, `iif`, `case`, `countif` and `sumif` (which take no named
arguments); inside any other call `name = value` stays a named argument. **`||` directly before an
operator name is two pipes** (DS0103), not `or`. **A certain fix means a certain parse**: when an
unknown operator name has a machine-applicable replacement (`filter` → `where`, `ORDER BY` →
`sort by`, `Take` → `take`, one close did-you-mean match), the rest parses as that operator, so
errors inside it appear in the same pass. `project - away` is reported (DS0104) and parses as
`project-away`; `sort x`/`top 3 x` report DS0121 and parse the keys. **Brackets**: DS0112 is
given only when no matching bracket follows before the end of the statement, and points at the
opening one; brackets left open at the same place share one error with a label each. A
`SELECT ... FROM` statement is one DS0105 and is skipped to the next `;`. **Fixes are tested**:
`mode: syntax` output shows the query with every machine-applicable fix applied and whether it
parses (TESTING.md). *(M1-03, 2026-11-20.)*

### D-031 — Function catalog details settled by M1-04
LANGUAGE.md §7 lists names, not signatures; where it is silent the catalog follows KQL (D-014).
`trim` takes `(regex, source)` as in KQL, not a single string. `now`, `startof*`/`endof*` and
`indexof` take KQL's optional trailing arguments. Arrow `Decimal32` and `Decimal64` (newer than
the table was written against) read as `real` like the other decimals. A `T` parameter widens
all its arguments to one type, so `iff(p, 1, 2.5)` is `real`; an untyped `null` operand takes the
type of the first operator row the other operand fits (`dt - null` is `datetime`). *(M1-04,
2026-11-20.)*

### D-032 — The nesting limit counts operator chains
D-029's limit of 256 levels counted only recursion, but left-associative chains (`a + b + …`) and
postfix chains (`a.b.c`, `a[0][1]`) are built by a loop that wraps the tree once per operator.
A 200,000-term chain parsed into a tree deep enough to overflow recursive walkers and rowan's own
drop, and rowan's node interning rehashed deep nodes with a recursive hash, which made unspaced
chains quadratic. **Each folded operator now costs one level of the same budget**, so the
limit bounds the depth of the tree, not of the parser's stack. A chain of more than about 256
operators is DS0100 ("expression is too long"); split it with `let`. Parsing stays linear on any
input. *(M1-18, 2026-11-21.)*

### D-033 — Bare `not` stays at level 3; method calls and recovery
Settled by M1-19, extending D-029. **A bare `not` is not an operand of a tighter operator.**
`1 + not x == y` used to parse as `1 + (not (x == y))`, and `a == not b == c` hid a chained
comparison. Rather than write that permissive rule into §4, the parser reports `DS0114` (as
Python does) and suggests `not(...)`, whose meaning is plain; the tree keeps the old shape so
nothing else cascades. After `and`/`or` and at the start of an expression a bare `not` is fine.
**Method-style calls** `a.f(x)` get `DS0113`; the node is a `CALL_EXPR` with no name, whose first
child is the member access, so the arguments are still parsed and walked. **A comparison chain is
one error** with help that splits every link. **`where a, b`** is `DS0115` with an `and` fix
(machine-applicable unless an `or` would regroup); inside a call's arguments the comma still
starts the next argument. **`distinct *, a`** is `DS0116`. **Recovery from an unclosed bracket
stops at a line that clearly starts a new statement**: unindented, able to start an expression,
and after a complete expression, so `f(a⏎U | take 1` keeps the second query while `f(a,⏎b)` and
`(a⏎- b)` stay continuations. The missing `;` there is not reported separately. *(M1-19,
2026-11-21.)* *(D-037 adds a second stop: the keyword `from` after a complete expression, at any
indentation.)*

### D-034 — Lexical gaps closed by M1-20
`5μs` with the Greek mu (U+03BC) is accepted like the micro sign (U+00B5): the two look the same,
and rejecting one of them would be an error the user cannot see. `timespan(...)` takes a leading
`-` before a short-form body (`timespan(-90m)`), not only before the clock form, since `-90m` reads
naturally and the clock form already allows it. An empty quoted name `['']` is rejected by the
lexer (new code DS0007) rather than the binder: the lexer is context-free (D-026) and `['']` is a
name wherever it appears, and no column, table or `let` can usefully be called that. `0x1F` stays
an error (no hexadecimal literals) but now says so, with the decimal value. Combining marks after
the first character continue a word, so a decomposed `naïve` is one DS0001 like the composed one.
Suggestions that quote a name (backticks) escape `\` and the chosen quote, so the fix lexes back to
the same name. *(M1-20, 2026-11-21.)*

### D-035 — When a syntax fix is certain (M1-21)
A fix is `MachineApplicable` only if applying it, together with every other such fix, parses and
keeps the meaning. **`===` and `= =`** are one DS0110 on both tokens with one fix to `==`. **Word
fixes pad against tokens other fixes turn into words** (`&&`, `||`, `!`), so `a!=1&&!b` becomes
`a!=1 and not(b)`. **`SELECT` → `project`** is certain only when the rest of the operator is a plain
column list; `SELECT *`, `SELECT DISTINCT a` and `SELECT a AS b` get a `MaybeIncorrect` fix. A whole
**`SELECT … FROM` after a pipe** gets the same DS0105 as at the start of a query; its rewrite
replaces the whole pipeline, and is offered only when no operator comes before the `SELECT`, since
it would drop them. **DS0106 after an operator that takes a list** (`project`, `extend`,
`summarize`, `sort`, `top`, `distinct`, `join`, `project-away`, `project-rename`) offers `,` first
and `;` second, neither certain: `;` would turn a forgotten comma into a second query. The golden
harness applies all certain fixes in one pass, a replacement before an insertion at the same
offset. *(M1-21, 2026-11-21.)* *(D-037: before a `from` first on its line, `;` alone is offered
and is certain, even after an operator that takes a list.)*

### D-036 — Every pipeline starts with `from`
Decided by the maintainer as Q-009, landed by M1-24. A query statement, the value of a tabular
`let`, a pipeline in parentheses and a pipeline passed as a call argument start with `from`:
`from 'orders.csv' | take 5` (≠ KQL). The parser then knows where a table starts, a `let`'s kind
is syntactic, completion after `from` can offer only tables and paths, and the SQL habit `from` is
right. The cost is a habit KQL users must change, softened by a machine-applicable fix.
- **No bare-source fallback.** `orders | take 5` is `DS0150`, an error, not a warning; keeping both
  forms would keep the ambiguity. The fix inserting `from` is `MachineApplicable` when the head is
  clearly a table (a string, name or call followed by `|`; a string or reader call alone); a bare
  name alone gets a `MaybeIncorrect` fix, since it may be a scalar typed by mistake; a `let` bound
  to a bare string or name is a scalar and gets no error. `from` with nothing after it is `DS0151`.
  The parser tells a reader call from other calls with `datascript_syntax::READERS`, which a test
  in `datascript-compiler` checks.
- **Where `from` is not written.** The right side of `join`, `union` and `lookup` is already a
  table position (`join customers on id`); a pipeline there is parenthesized and starts with
  `from`. Row generators (`range`, `datatable`, `print`, M2-01) start a query on their own; until
  M2-01 they are `DS0104` "not available yet" at a pipeline head. A parenthesized pipeline starts a
  query without another `from` (`(from t | where a) | take 5`).
- **`from` stays contextual** (D-013) and an `IDENT` token (D-029). It is the keyword at a pipeline
  head when an expression start follows it, other than `-` and `not`, so `let y = from - 1` is a
  scalar and `| where from > 3` a column. `from(` is the keyword and a parenthesized source. A table
  named `from` is `from ['from']` or `from from`. The match is case-sensitive. At the head of a
  query statement, `from` followed by `|`, `;`, `let` or the end is `DS0151`; in a `let` value, in
  parentheses and in call arguments only `from |` is, because `let y = from;` and `f(from)` read
  a name.
- **Tree shape.** `from x` is always a `PIPELINE` node, with zero operators if none follow, in every
  position. The `from` token is a **direct child** of the `PIPELINE`, before the source expression
  (no separate `FROM_CLAUSE` node), so `Pipeline::source()` still finds the source as the first
  expression child; `Pipeline::from_token()` returns the token.
- **A `let`'s kind is syntactic.** A `let` whose value *is* a `PIPELINE`, or a `PIPELINE` in
  parentheses, is tabular (`LetStmt::is_tabular`); anything else is scalar. `let alias = orders;`
  is a scalar `let` even if `orders` is a table; the binder reports it with a hint to write
  `let alias = from orders;` (M1-05). A reader call alone as a `let` value
  (`let r = read_csv('x');`) is a `PIPELINE` with `DS0150`, so it stays a table after the fix.

Supersedes the parts of D-006, D-020 and D-029 noted there. *(M1-24, 2026-11-21.)*

### D-037 — Statements start at `from` by grammar position (M1-25)
**Rule.** After a complete expression, where only `|`, `;`, `)` or the end could come next, the
keyword `from` (D-036: `from` followed by an expression start other than `-` and `not`) starts a
new statement, whatever line it is on and however it is indented: `from` as a name cannot follow a
complete expression. The parser checks this in `pipeline_tail`, in the statement loop, and when
recovering (`Parser::query_start_at`). The layout rule (an expression start first on its line,
outside brackets) stays as a fallback for a query that forgot its `from`. `range x from 1 to 10`
needs no special case: after `range x` the generator is not complete.
- **DS0106.** A `from` first on its line gets one `MachineApplicable` `;` fix, even after an
  operator that takes a list (overrides D-035 there). On the same line it is `MaybeIncorrect`, and
  after `project`, `extend`, `summarize` or `distinct` it is SQL's `SELECT a FROM t`: DS0153, with
  help but no fix, since which table was meant is unclear.
- **Unclosed brackets.** Recovery from a bracket that is never closed stops at such a `from`, so
  the next query is kept without D-033's "unindented" condition. `closing_ahead` still uses only
  the unindented rule, so a bracket closed later is not cut short: the `from` inside it is a
  plain DS0101.
- **`from` in the wrong place is DS0152.** On the right of `join` and `union` the fix removes it
  and is certain. After `|` the fix depends on what follows: before an operator name
  (`| from where x`) it removes `from` and is certain; otherwise (`t | from x`) a new query most
  likely starts there, so the fix turns the `|` into `;`, `MaybeIncorrect`, with `union`/`join`
  mentioned as the alternative. (The task asked for removal in every case, but `t | x` does not
  parse.) The `from x` after it is then parsed as the next statement.
- **SQL order is DS0153.** A SQL keyword after the table without `|` (`from t select a`,
  `from t GROUP BY a`, `from t WHERE x`) gets the fix DS0104/DS0105 would give after a `|`, with
  the `|` inserted, and the same certainty. Operator names of the language (`order`, `where`)
  stay DS0102.
- **`FROM` in capitals at a pipeline head** is DS0105 with a certain fix to `from`, and parses as
  the keyword. *(M1-25, 2026-11-21.)*

### D-038 — Binder scopes, recovery and untyped results (M1-05)
**Columns win over scalar `let`s.** In an expression a name is first a column of the current row,
then a scalar `let`. `let`s cannot shadow each other (DS1003), but a column can appear later from a
file the program does not control, so making that an error would break scripts when a file gains a
column. A `let` value has no row, so a name there must be a `let` (DS1002, with a note that a `let`
cannot use columns). **A table is not a value**: a tabular `let` used in an expression, or
`let alias = orders;`, is DS1006 (the name is the problem, with a fix inserting `from`); a pipeline
inside an expression (`(from t | count) + 1`, `x in (from t | ...)`) is DS2010. **Syntax errors
silence the node they are in**: for each syntax or lexical error the binder finds the innermost
expression, operator or `let` around it and gives it the error type without binding it, so a
syntax error adds no semantic errors while the rest of the statement, and other statements, are
still checked. Errors directly in a pipeline (a missing `|`) and insertion points mark nothing.
**Untyped results**: a function whose result would have the internal `Null` type (`abs(null)`,
`iff(p, null, null)`) is DS2009 at the `null` argument, like `null + null` where a value needs a
type. **Aggregate context** is one parameter of expression binding (`Agg`): rows (DS2004 names the
place: `where`, `by`, a `let`), `summarize`'s aggregate list (a column outside an aggregate, or an
item with no aggregate, is DS2005), or inside an aggregate (no nesting). *(M1-05, 2026-11-21.)*

### D-039 — M2 types stay out of M1; integer math is `long` (M1-23)
**`decimal` mixes with nothing in M1.** The lattice said `decimal` meets `real` at `real`, but the
operator table's `num` excludes `decimal`, so only `decimal == real` worked. A `decimal` value can
only come from `decimal(null)` in M1 (files read decimals as `real`), so the lattice was narrowed
rather than the table widened: `decimal + real`, `decimal < real` and `avg(decimal)` are errors
with help suggesting `toreal`, and M2-10 decides the conversions. **`dynamic` and `guid` columns
are unsupported (`DS2008`) until M2**: nested Arrow types, Utf8 with `arrow.json` and
FixedSizeBinary(16) with `arrow.uuid` read as no type, so they can be projected but not used in
expressions, instead of producing values no operator or function accepts. **`abs`, `round`,
`floor`, `ceiling` and `bin` of an integer return `long`**, as `-x` and `sum` do (D-021), so
`abs(toint(-2147483648))` cannot overflow; the catalog's `IntegerAsLong` return rule covers them
and `sum`/`sumif`. **An untyped `null` next to a number class is `long`**: `null * 1d` scales by a
`long` rather than multiplying two timespans. **Overload errors** blame the argument where the
form that matched most arguments exactly (no `null`, error or widening) failed, the first form on
ties, rather than the furthest argument any form reached. **Untyped results are not defaulted in
the catalog**: `Resolved::return_type` may be `Null`, which the binder reports as `DS2009`
(D-038). *(M1-23, 2026-11-21.)*

### D-040 — Operator schemas and column naming details (M1-16)
LANGUAGE.md §3.3 left some cases open; the binder settles them as follows. **`extend` items see
the items before them** (`extend total = amount * qty, big = total > 100`), as in KQL; an item
that replaces a column replaces it for the later items too. `project` items see only the input
(a renamed column cannot be read again in the same `project`). **Unnamed computed columns** in
`project` get `Column1`, … as in `extend`; a bare column keeps its name. Generated names skip any
name already taken: the input columns (for `extend`) and every name written in the same operator.
**Unnamed `summarize` items**: an aggregate call over exactly one column is `f_col`, any other
aggregate call `f_` (`countif(qty > 1)` → `countif_`, `percentile(x, 50)` → `percentile_`), and
an expression that is not a bare aggregate call (`sum(a) * 2`) gets `Column1`, …. Derived names
that collide get the first free `1`, `2`, … suffix, so explicit names always keep theirs.
**Duplicate names** (`DS1005`) keep the first column, so later operators still see one column of
that name and report nothing more. **`project-rename`** applies its items in order, and a new name
another column has is `DS1005` (swapping two names needs `project-away` or `project`).
**`distinct`** takes column names only (`DS1032`, with help to `extend` first). **`summarize`**
with neither an aggregate nor a key is `DS2030`. **`join`**: renamed right columns take the first
`name1`, `name2`, … that neither side uses, and one `DS1007` lists every rename of a join; it is
not reported when a condition has an error, since the rename may only be a symptom. A condition
that is not a column name or `$left.a == $right.b` (either order) is `DS1030`, and `$left`/`$right`
anywhere else are `DS1031`. Key columns widen per §4.2; types that do not are `DS2003`, like a
`union` column conflict. *(M1-16, 2026-11-21.)*

### D-041 — Where the compiler checks sources and capabilities (M1-17)
**Formats are decided by the compiler.** The binder finds a path's format from its extension and
reports `DS3002` itself, and checks a reader's option names and values from its own catalog
(`source::READERS`, `DS3003`, `DS2041`–`DS2043`), before the provider is asked; the engine never
sees a path it could only guess at. A format the engine cannot read is `DS4001` at the same point,
so the provider is never asked for it either. **The capability checks are not a separate pass.**
Functions and `join` kinds are checked where the binder binds them, which has the callee and the
kind token at hand for the span; an unsupported function keeps its type, so nothing cascades.
**Host tables.** A name in a table position that is not a `let` is `SourceRef::Table` and is
looked up like a file; `NotFound` for it is `DS1002` "unknown table" (did-you-mean over the host's
names and the tabular `let`s). `let alias = orders;` looks `orders` up the same way so it can
report `DS1006`. **Projection-only columns** (no DataScript type) keep their Arrow type in
`hir::Column::arrow_only`; a bare name in `project` passes them through, `project-away` and
`project-rename` keep them, and any other use is `DS2008`. *(M1-17, 2026-11-21.)*

### D-042 — How the planner desugars into the IR (M1-06)
The IR keeps column names unique in every node's output and refers to columns by name, so the
planner adds projections rather than giving the IR qualified names. **`summarize`**: each
`Aggregate` item is one aggregate call over row expressions. An item that computes over
aggregates (`sum(a) / count()`) has each call computed as a hidden column (`<item>_1`, `_2`, …,
skipping taken names) and a `Project` after the `Aggregate` finishes it; `count` is an `Aggregate`
with no keys. **`join`**: each condition holds one expression per side, cast to the key's common
type. For inner and outer joins a `Project` on the right gives right columns their output names
(the `DS1007` renames) and each `on key` right column a hidden name, then a `Project` after the
join merges the pair (D-019). Semi and anti joins need neither. **`union`** projects every input to
the output columns, with typed nulls for a missing column and casts for widened ones; an input that
already matches is kept. **`distinct a, b`** is `Distinct` over `Project`. **`getschema`** is a
`Values` node computed at compile time: ordinals start at 0, as in KQL, and a column with no
DataScript type shows its Arrow type. **Sort keys always carry an explicit null placement**,
because engine defaults differ. **`extend`** stays one `Project` per item. A reference to a
projection-only column has `Type::Error` in the IR; its Arrow type is in the node schemas. The
compiler has one `JoinKind` (`capabilities::JoinKind`, re-exported by `hir` and `ir`). The IR
derives serde `Serialize` and `Deserialize`. *(M1-06, 2026-11-21.)*

### D-043 — Session caching, row limits and the panic boundary (M1-07)
**Failed schema lookups are cached like successful ones**, until `Session::invalidate_schemas` or
`invalidate_source` is called; a source is fetched at most once per call even if a concurrent
invalidation lands between passes. A one-shot CLI run never needs invalidation; the REPL, the LSP
and `ds ui` invalidate when files may have changed (M1-12, M3-01, M4-10). The cache key is the
base directory, the sandbox and the `SourceRef`, because the sandbox can refuse a path that is
fine without it. **The row limit is enforced by the session**, not the backend: `ResultStream::
collect` stops after `row_limit` rows, drops the stream (cancelling the query) once one more row
arrives, and sets `QueryResult::truncated`. How frontends word that is D-064's (M4-10).
**`QueryResult`** is `{ schema, batches, render, diagnostics, truncated }`: `render` is copied from
the plan; `diagnostics` holds per-query warnings (none yet), while errors stop the run and go to
`RunOutput::diagnostics` together with every non-error diagnostic from checking. **Panics**: every
`Session` method runs under `catch_unwind` (`catch_panic` / `catch_panic_async`, also public for
frontends that call the compiler directly) and a panic becomes `DS9001` "internal error:
DataScript crashed while …" with the panic's first line as a note. On wasm32 panics abort, so M5-01
adds a panic hook. **Writers**: CSV and JSON print values exactly as the table does (one
`value_text` function); JSON uses native booleans and numbers where JSON has them and strings
otherwise (`NaN`, `inf`, datetimes, timespans). *(M1-07, 2026-11-21.)*

### D-044 — How the DataFusion engine reads sources (M1-08)
**Globs are read file by file** *(amended by D-061: for CSV, TSV and Parquet; JSON globs infer one
joined schema and are read by one `ListingTable`)*. The engine infers each file's schema on its own
and merges them by name. When every file has the same schema, one `ListingTable` reads them all;
otherwise each file is its own table, projected to the merged columns (typed nulls where a file
lacks one) and combined with `UNION ALL` in path order, because DataFusion applies a table schema
to CSV files by position. **The engine remembers what `schema_of` found** (files and per-file
schemas, keyed like the session cache: base directory, sandbox, `SourceRef`), and the scan uses it
instead of inferring again; every `schema_of` call reads afresh and replaces the entry, so a
session that invalidates also refreshes the engine. **Columns with only nulls** (an all-empty CSV
column, Arrow `Null`) read as `string` *(for JSON/NDJSON sources, D-061 makes all-null columns
`Utf8` + `arrow.json` instead)*, and in a glob take the other files' type; LANGUAGE.md §5.1 has
the row.
**Host tables** are registered on the engine (`DataFusionEngine::with_table(name, SourceRef)`), not
in `ExecContext`, so the session cache key does not change; their relative paths resolve against
the run's base directory, so hosts pass absolute paths for `--table` (M1-11). A host table whose
file is missing is `DS3008`, since `NotFound` for a table means "no such table" (`DS1002`).
**Workarounds for DataFusion 55**: it applies `null_regex` when inferring a CSV schema but not
when reading rows, so with `null_value` the scan reads text and applies `nullif` before the cast;
Parquet is read without view types so columns keep the file's types (`Binary`, not `BinaryView`,
in `DS2008`). `UInt64` columns go through a checked UDF, so a value above `i64::MAX` is `DS4003`
naming the column. **`DS3009`** (invalid glob pattern) was added in M1-08's block. **Decimal
precision**: LANGUAGE.md §5.1 no longer says `getschema` shows the loss; `getschema` shows the
DataScript type (`real`), and keeping the file's type in the HIR is not worth it before M2 makes
decimals exact. **wasm**: the crate's `wasm` feature is empty and `compression` (default) is
native-only; `.cargo/config.toml` sets the getrandom backend for wasm32. *(M1-08, 2026-11-21.)*

### D-045 — How the DataFusion engine lowers and runs plans (M1-09)
**Scalar functions are our own UDFs.** The engine builds DataFusion without its string, math,
regex and datetime function sets, and every catalog function is a kernel in
`datascript-datafusion` (`udf.rs`), so semantics follow LANGUAGE.md and not a DataFusion builtin
(`log` is natural, `substring` is 0-based, `startofweek` is Sunday, `round` halves away from zero).
DataFusion's own operators are used only where they match exactly: comparisons, `and`/`or`/`not`,
`in`, `between`, real arithmetic, `CASE`, `coalesce` and widening casts. **Integer and temporal
arithmetic is checked** (D-014): `+ - * / %` and unary `-` on `long`, `datetime` and `timespan`,
`abs`, `bin`, `round` and `sum` over integers and timespans are UDFs/UDAFs that fail with `DS4003`,
so the engine lists no deviations. **Every UDF carries its expression's span**, so a runtime
error points at the expression that failed, whether DataFusion hits it while folding constants
or while running; errors with no span (a malformed row) go to the scan when the query reads one
source. **Invalid argument values** found at run time (a regex that does not compile, an unknown
datetime part or format specifier, a percentile outside 0–100) are the new code `DS4004`, from
M1-09's block; checking constant arguments at compile time is left to M1-13. **Aggregates**:
`count`, `min`, `max`, `avg`, `stdev`, `variance` and `dcount` (`count(distinct)`, exact) are
DataFusion's; `sum` of integers and timespans and `percentile` (nearest-rank: the value at rank
⌈p/100·n⌉, the first percentage seen per group) are UDAFs; `take_any` is `min`, so it is
deterministic. `stdev`/`variance` of one value are null (sample statistics). `sum`, `sumif` and
`avg` also take a `timespan`, as in KQL (`avg` truncates to whole µs). **Conversions** return null
when a value does not convert; `tolong`/`toint`/`toreal` of a `datetime` or `timespan` are null
(≠ KQL, which gives ticks), strings must be in integer syntax for `tolong`, and `tostring(null)` is
null while `strcat` skips nulls. **Joins** alias each side (`join<n>_left`/`_right`) and project
the qualifiers away afterwards, so semi and anti joins whose sides share names work. **Results**
are rebuilt with the plan's output schema, which carries extension-type metadata (D-024) and
catches a lowering that returns the wrong type (`DS9001`). `Capabilities::functions` is the
engine's mapping table (`lower::SUPPORTED`); a test checks it is the whole catalog and that every
signature lowers. *(M1-09, 2026-11-21.)*

### D-046 — How `ds` reports, prints and stops (M1-11)
**Exit codes** are 0, 1 (the program has errors, or warnings with `--deny-warnings`) and 2 (usage
error, unreadable script, unwritable output, internal error), plus **130 for Ctrl-C** (128 +
SIGINT, what shells report). The program runs on a tokio worker thread so the main thread notices
Ctrl-C even while a query computes; the CLI then exits, which drops and cancels the query.
**Diagnostics** use the terminal renderer (Unicode, colored unless `NO_COLOR` is set and not
empty) when stderr is a terminal, and the plain ASCII renderer otherwise, so logs stay readable;
`--color always` forces the colored terminal renderer, `--color never` drops color. **A missing or
unreadable script** is `DS3001` / `DS3008` with no span (did-you-mean over the script's directory),
and `--table` with an unknown extension is `DS3002`: the existing codes for the same problems, not
new ones. **Command-line misuse** (`--format csv` with several queries, `--statement` out of range,
Arrow to a terminal, a bad `--table` value) is a clap usage error with a tip, not a `Diagnostic`:
there is no program location and no code block for the CLI, and clap's format matches its other
usage errors. **Several queries**: `--format table` prints `== query <n> (line <l>) ==` before each
table (only when there is more than one), `json` prints `[{statement, columns: [{name, type}],
rows}]` with the type as the table names it, and `csv`, `ndjson` and `arrow` need one result
(`--statement <n>`). `--statement` checks the whole program, so an error in another statement
still stops the run, and runs only the chosen query (`Session::run_statement`); the line comes
from the new `QueryPlan::span` / `QueryResult::statement`. **`--format arrow`** writes the Arrow
IPC *file* format (what `.arrow` and Feather v2 readers open). **`--max-rows`** (default 100, 0 =
all) cuts only tables, after the whole result is collected, so the footer can say how many rows
were left out; `csv`, `json`, `ndjson` and `arrow` always hold every row. Nothing is written (and
no `--output` file created) when there are no results. A closed pipe (`ds run … | head`) exits 0.

### D-047 — `render` semantics and RenderSpec (M1-10)
The design review of M1-10 found LANGUAGE.md §6 too thin to implement without inventing rules, so
they are settled up front. **Resolved at compile time**: the compiler fills every default and puts
an explicit spec on the plan, so the CLI, web UI, VS Code and WASM never disagree on what a chart
plots. **Roles and types** (§6.2): every kind gets defaults (the old text covered four of nine);
defaults never reuse a column; `timespan` is not numeric for charts in M1 because frontends would
need a unit, and can be added later without breaking specs. **KQL alignment** (checked against
Microsoft's render docs, 2026-11-21): y defaults to all numeric columns for bar, column and scatter
charts; `card` shows the first row and takes only `title`; `kind = unstacked` is `default`;
`piechart` takes `xtitle`, `ytitle` and `legend`. KQL kinds and properties DataScript lacks
(`anomalychart`, `ysplit`, `xaxis`, …) get a "not supported yet" message, not a did-you-mean.
**Deliberate ≠ KQL**, marked in §6: defaults by type rather than position (KQL's own pages
disagree on this), a `datetime` x for line and area charts, series from all string columns rather
than one (KQL silently drops the others), `series` not on `piechart`. **Properties are strict**: a property
that does not apply to the kind is an error (`DS5006`), not silently ignored as in KQL, so a typo'd
intent never draws a different chart than the user expects; values are bare column names, not
expressions. **Unknown columns reuse `DS1001`**, as D-046 reuses codes for the same problem.
**`render` only in query statements** (`DS5008`): elsewhere its chart could never be shown.
**`DS5001` once per program run**, not per query, so a script with five charts prints one line;
`render table` is drawn by every runtime and never gives it. **Golden and `explain` output** share
one summary line so plan and run cases agree. **JSON**: camelCase, absent rather than `null`,
additive only (ARCHITECTURE.md, "Render layer").

### D-048 — How the REPL keeps lets, submits input and reports errors (M1-12)
Settled before implementation so the task stays inside `datascript-cli`. **Lets are kept as
text**: each submission is checked and run as one program, the saved let statements followed by
the new input, and every diagnostic span is mapped back to the input it came from. Each input is its
own snippet (`<repl:N>`, lines from 1), so an error renders against what the user just typed, and
an error in a saved let (its file was deleted) against that let's own snippet. The alternative, a
compiler API taking earlier inputs as a prelude, is cleaner but changes `datascript-compiler` for
one frontend; the LSP and web UI work on whole documents and don't need it. *(Partly superseded by
D-062: the web UI, CLI, and LSP now share `StitchedProgram` in `datascript-compiler` for `.dsnb`
notebooks.)* **Saving**: an input's
lets are saved once it checks without errors, before its queries run; a check error saves nothing,
a runtime error or Ctrl-C undoes nothing. **Redefinition is allowed in the REPL** (≠ `DS1003` in
programs, LANGUAGE.md §2): a new `let x` removes the old one and every saved let that uses it,
transitively, with an info line naming them. Keeping the users and re-binding them to the new `x`
would need the new definition placed before them, where it may use names that don't exist yet;
silently keeping a stale value would be worse. Two `let x` in one input are still `DS1003`. Saved
scalar lets are evaluated per submission, so `ago(1d)` follows the clock (D-015 holds within one
submission). **Submit rule**: Enter submits complete input (judged with the lexer) that ends with
`;`, or any input on a blank line; meta commands submit on Enter. The old "continue while the line
ends with `|`" rule is dropped, because the usual style puts `|` at the start of the next line.
**Schemas are invalidated before every submission and `.schema`** (D-043), so no `.reload`.
**Meta command mistakes** are plain messages with a tip, like CLI misuse in D-046: there is no
program location, and the task owns no code block. **Piped stdin** runs the same loop without the
editor and exits 1 if any input failed, which gives end-to-end tests a way in. *(Design review of
M1-12, 2026-11-21.)*

*Implementation notes (M1-12, 2026-11-22).* **Dependencies**: `reedline` is pinned to 0.49,
the last release that builds on the workspace MSRV (1.94; 0.50 and later need 1.95), and
`nu-ansi-term` 0.50 is a third new dependency, because reedline 0.49 does not re-export the
`Style` type its highlighter API takes. **Ctrl-C**: checking and running are spawned on a worker
thread (`ReplSession::prepare` / `check_task` / `commit`, then `run_task` / `finish_run`), as in
`ds run`: polled inline on the main thread, a busy DataFusion query never let the `ctrl_c` race
fire. Aborting the task drops the query. **Warnings and infos about a saved let are not repeated**
on later submissions (they were shown when it was entered); errors in saved lets still are. The
**redefinition info line** only appears when users were removed; replacing a let nobody uses is
silent. **`.schema` diagnostics** render against an unnumbered `<repl>` snippet holding the meta
command, since meta commands don't count as inputs. **`.schema` and `.tables` print plain aligned
tables** without the type row of a result: they show text, not data. **`--table` and
`--max-rows` before a command** (`ds --table t=x.csv run …`) are a usage error with a tip; clap's
`args_conflicts_with_subcommands` would also reject the global `--color` there. In piped mode,
`.quit` stops reading and keeps the exit status so far.

### D-049 — M1 close-out: constant counts, check-time argument values, unicode widths (M1-13)
**Accepted.** Choices made while closing M1 against the ROADMAP:

- **`take`/`limit`/`top` counts** are a constant whole number of 0 or more: a literal or a scalar
  `let` bound to one, like render titles and paths (D-020). Anything else is `DS2050` (block
  DS2050–DS2059, reserved for M1-13), which says whether the count is a column, computed,
  negative or not a whole number. Before, every non-literal count was `DS9001` "not implemented
  yet", an internal-error code for what is a user mistake.
- **Constant arguments the language restricts are checked by the binder**: a regular expression
  (`matches regex`, `trim`, `extract`), a `datetime_part`/`datetime_diff` part, a
  `format_datetime` format and a `percentile` percentage. A bad constant is `DS4004` from `ds
  check`, pointing at the argument; a computed one is still `DS4004` when the query runs. The code
  stays `DS4004` so the error is the same whenever it is found. The rules live in
  `datascript_compiler::arg_values`, which the DataFusion engine also calls, so the two cannot
  disagree. The binder parses regexes with `regex-syntax` (the parser half of `regex`, which
  builds for wasm32); an engine may still reject a pattern that parses but is too large.
- **`DS2006` help** suggests a conversion only when it gives a value: never `toreal(timestamp)`
  (null, §7), never `todatetime(qty)`. A number where a `bool` is needed gets `x != 0`, as in
  `DS2001`; a number where a `timespan` is needed gets "write a timespan with its unit". When
  swapping two arguments makes the call type-check, the help says the arguments may be in the
  wrong order and offers the swap (`datetime_part(ts, 'year')`).
- **Signature notes** keep `where T is …` outside the code span, since it may name types in
  backticks: `` `min(x: T) -> T`, where T is a number, `string`, … ``.
- **Terminal widths** come from `unicode-width`: carets and table cells count CJK and most emoji
  as two columns and combining marks as none. Summing per-string widths also gets emoji ZWJ
  sequences right in the prefix before a caret; only the window cut from a very long line counts
  per character. No grapheme segmentation was needed. `datascript_diagnostics::text_width` is the
  one function both the renderer and the table formatter use.
- **SQL habits**: `LEFT JOIN t`, `FULL OUTER JOIN t`, `LEFT ANTI JOIN t`, … in operator position
  are `DS0105` with a fix to `join kind = …`; `CROSS JOIN` says there is none. `NULLS FIRST` in
  capitals is `DS0105` with a fix to lowercase, and the key still parses.
- **REPL** gets `.max-rows [n]`, and a cut table's footer names it instead of the startup flag.
*(M1-13, 2026-11-22.)*

### D-050 — No lint for `take` before `sort`; DS2101 retired
**Accepted.** M1-14 planned warning `DS2101` for `take`/`limit` followed later by `sort`. It is
dropped before it shipped:

- The pipe already says what happens: `take 10 | sort by x` reads as "take 10 rows, then sort
  them". The SQL habit (`ORDER BY … LIMIT`) leads people to `sort | take`, which is correct.
- The pattern is often intended: taking a few arbitrary rows and sorting them to look at them is
  normal exploration, and KQL does not warn on it. LANGUAGE.md already says `take` keeps no
  order unless it follows `sort`.
- As written it would also flag deterministic queries (`sort by a | take 10 | sort by b`), and
  fixing that still leaves false positives that make `--deny-warnings` a nuisance.

`DS2103` stays: `sort` defaulting to ascending is a silent difference from KQL (D-005), which
the pipe does not show. `DS2101` is retired, never to be reused. If users turn out to make this
mistake, a new opt-in lint gets a new code. *(2026-11-22.)*

### D-051 — What the M1-14 lints flag
**Accepted.** Lints run on the bound program, after the binder, from one table
(`datascript_compiler::lints::LINTS`) holding each code and default severity.

- **`DS2102`** checks only `where`, and folds only what is certain to be the same for every row:
  literals, casts, `not`, unary `-`, comparisons, `+ - *`, and `and`/`or` with LANGUAGE.md §4's
  null rules (`x > 0 or true` is true whatever `x` is). Columns never fold, so `x != x` and
  `x == x` do not warn (nulls). Function calls do not fold (`now()`, `rand()`). A scalar `let`
  does not fold either: `let includeAll = true; … | where includeAll or …` is a switch the user
  flips on purpose, and warning on it would make `--deny-warnings` a nuisance. A condition that
  folds to `null` (no rows) does not warn, because only a literal `null` reaches it and that
  already reads as "nothing".
- **`DS2103`** is reported once per `sort`, at its first key without a direction, with the other
  such keys as secondary labels; the one fix adds `asc` to all of them. `top` keys never warn.
*(M1-14, 2026-11-22.)*

### D-052 — How the language server checks documents (M3-01)
**Accepted.** `datascript-lsp` uses `tower-lsp-server` (and the `ls-types` it re-exports) with
incremental sync. It does not pick an engine; `ds lsp` passes a `Session` over DataFusion.

- **When it checks**: an opened document at once, a changed one after a 150 ms debounce. Each
  scheduled check gets a generation number; a check whose document changed meanwhile publishes
  nothing, and publishing holds the documents lock, so a stale result can never overwrite a newer
  one. A check is not cancelled when superseded: its schema reads still warm the cache.
- **Syntax first, without flicker**: the full check (`Session::check`) races a 50 ms grace. With
  warm schemas it wins and the editor sees one update. When schemas must still be read, the
  lexical and syntax diagnostics are published on their own, then the full set. Publishing syntax
  errors on every keystroke would make semantic errors blink out while typing.
- **Schema cache**: the server asks the client to watch `**/*` (dynamic registration of
  `workspace/didChangeWatchedFiles`), because any file can be a source (`read_csv('data.txt')`,
  globs, a file that does not exist yet). A change to anything but a `.ds` script clears the whole
  cache (`invalidate_schemas`) and re-checks open documents. Mapping one changed file to the
  `SourceRef`s it affects would need glob matching against every base directory, for little gain:
  the next check reads only what open documents use. Clients that cannot register watchers get no
  invalidation until the server restarts.
- **Conversion**: positions are UTF-16 (D-025). The message carries the help and note lines as the
  terminal prints them (`help: …`), since editors show only the message. The primary label (when
  it says something) and secondary labels become related information. The code description links
  to `codes::docs_url`, the DIAGNOSTICS.md code registry until M2-08's pages exist.
- **Line breaks**: `SourceFile` now ends a line at `\n`, `\r\n` or a lone `\r`, as the LSP and
  the parser do, so positions agree everywhere (until now a lone `\r` was not a line break).
- **Base directory**: a `file:` document's directory; for other documents (`untitled:`), the first
  workspace folder, else the server's working directory.
*(M3-01, 2026-11-22.)*

### D-053 — The VS Code extension: trust, grammar and tests (M3-04)
**Accepted.**

- **Syntax-only mode**: the client sends `initializationOptions: { "syntaxOnly": true }` when
  the workspace is not trusted. The server then publishes only `parse` diagnostics, never calls
  `Session::check`, registers no file watcher and ignores watched-file changes, so it reads no
  file. A flag at startup, rather than a setting the client can flip, keeps the server simple: the
  client restarts the server when trust is granted. Unknown options and options of the wrong type
  are ignored, so an older client and a newer server still work together.
- **Restricted settings**: `datascript.server.path` and `datascript.server.extraArgs` are
  `restrictedConfigurations`, so VS Code ignores their workspace values in an untrusted folder.
  Otherwise opening a folder could run any program it names.
- **The grammar follows the lexer, and a test checks it**: the Rust test
  `datascript-syntax/tests/textmate_tokens.rs` writes the lexer's tokens for the fixture files
  `editors/vscode/test/grammar/*.ds` to `tokens.json` (UTF-16 columns, split per line) and
  fails if the fixtures miss a token kind. The extension's grammar test runs VS Code's tokenizer
  (`vscode-textmate` with `vscode-oniguruma`) over the same files and checks that every character
  of every token carries the scope its kind maps to. A hand-written scope assertion per fixture
  line (`vscode-tmgrammar-test`) would cover only what someone thought to assert, and would not
  catch the grammar drifting when the lexer changes.
- **Operator names**: right after `|` and any trivia, also across lines, the first identifier is
  the operator name. Like the parser, the grammar joins only the known hyphenated names, and only
  when the three tokens are adjacent (D-026). Other contextual keywords (`by`, `asc`, `has`, …)
  are scoped wherever they appear. Semantic tokens (M3-03) can correct the rare column called `by`.
- **Extension tests** run VS Code twice: with `--disable-workspace-trust`, and with trust enabled
  and the folder untrusted (Restricted Mode, no startup prompt). `@vscode/test-electron`'s
  `runTests` always disables trust, so the runner uses its download and starts VS Code itself.
  VS Code has no API to grant trust, so the restart on trust is not tested end to end.
- **Packaging**: esbuild bundles the client into `dist/extension.js`, so the `.vsix` ships no
  `node_modules`. The server is not bundled yet: `datascript.server.path` (default `ds` on
  `PATH`) names it. Bundling platform binaries is M6-02's.
*(M3-04, 2026-11-22.)*

### D-054 — VS Code Run prints CLI output first; the results panel comes later
**Accepted.**

- **First, text in a terminal**: M3-05 shows a Run result the way `ds run --format table`
  prints it, in a terminal, instead of waiting for the `web/` grid (M4-02) and charts (M4-03).
  The results panel moves to M3-06. Charts are not drawn until then; the output says so, as the
  CLI does.
- **Through the language server, not by starting `ds run`**: Run must use unsaved edits. The
  server already holds each open document's current text and a warm schema cache, publishes
  diagnostics and supports request cancellation. `ds run -` could read the text from stdin, but
  its errors would name `<stdin>`, relative paths would resolve against the working directory, and
  `ds run` has no machine-readable diagnostics, so it would need two new flags and a process per
  run. The custom `datascript/run` request returns the text the CLI would print, made by the same
  code; M3-06 adds a data format to the same request.
*(Planning, 2026-11-22.)*

### D-055 — How `ds run` and the REPL stream results (M2-11)
**Accepted.**

- **One streaming API for every frontend.** `Session::start(src, file, ctx)` checks and plans
  once and returns a `RunStream`: the diagnostics `run` would give before running (warnings,
  `DS0004`, or the errors that stop it), the selected `QueryPlan`s (`select(index)` replaces
  `run_statement`), and `next_query()`, which starts one query and hands back a `QueryStream`
  (schema, `render` spec, spans and a `BatchStream`). It owns its engine handle and context, so
  it is `Send` on native targets and can move into a spawned task (the REPL, and M4-10's
  WebSocket). `Session::run` is now `start` plus `QueryStream::collect`, so golden tests are
  unchanged. **Panic boundary**: checking and `Engine::execute` run under `catch_panic_async`,
  and each batch stream is wrapped so a panic while producing a batch becomes a `DS9001` item
  that ends the stream.
- **Tables buffer up to `--max-rows` rows**, print them at once, then count what is left only
  while it is cheap: at most 10,000 more rows are read (not kept). If the result ends within that
  budget, the footer is unchanged: `… 48 more rows (show them with --max-rows 50)`. Otherwise
  reading stops, which cancels the query, and the footer says
  `… at least 16381 more rows (show them all with --max-rows 0)` (`.max-rows 0` in the REPL).
  The count is a lower bound because rows arrive in whole batches. Counting without a budget
  would keep memory flat but run the whole query; dropping the count entirely would change every
  existing footer.
- **CSV, NDJSON, Arrow and JSON are written batch by batch**, byte for byte as before (checked
  against the M1 binary over every fixture). JSON writes each result's
  `{"statement", "columns"` head with `serde_json` and then the rows, so the bytes match the
  collected document.
- **A runtime error no longer hides earlier output.** Results already written stay written; the
  error is reported after them and the exit code is 1. A query that fails on its first batch
  writes nothing for itself, and `--output` is not created until the first result has something
  to write, so a run that fails before any output still creates no file. A cut table stops
  reading, so an error past its last counted row is never seen: the program is only run as far
  as needed to show what was asked.
- **`DS5001` is given before anything runs**, from the plans' `render` specs, so it comes out
  ahead of the tables in `ds run` (in the REPL, after them, with the other run diagnostics). It is
  given even when a later runtime error stops the program.
- **Ctrl-C** is unchanged: the program runs on a worker task, exiting (or aborting the REPL's
  task) drops the `RunStream`, which cancels the query, and `ds run` exits 130.
*(M2-11, 2026-11-22.)*

### D-056 — How VS Code Run works (M3-05)
**Accepted.**

- **One printer.** The code that prints a run's results (`stream_results`, the table footer,
  `DS5001`) moved from the CLI to `datascript_engine::printer`, so `ds run`, the REPL and
  `datascript/run` print through the same function. `datascript/run` returns
  `{ output, failed }`: the check's diagnostics, the tables and the runtime error, rendered with
  the renderer `ds run` uses on a terminal (`Renderer::terminal(color)`), in the order `ds run`
  writes them to standard error and standard output. `failed` is what makes `ds run` exit 1. The
  table footer says `--max-rows`, as `ds run`'s does; the editor's setting
  `datascript.run.maxRows` maps to it. A parity test runs both through the `ds` binary
  (`crates/datascript-cli/tests/lsp_run.rs`), since the LSP crate cannot run the CLI.
- **Names.** Diagnostics name the document by its path relative to the workspace folder, as
  `ds run` run from that folder does; a file outside it by its full path; an untitled document by
  its URI.
- **The query runs on its own task.** tower-lsp-server reads, dispatches and answers messages on
  one task, and polls request futures there. A query that computes for a long stretch between
  yields (DataFusion yields every 128 batches) would hold up every message, `$/cancelRequest`
  included: cancelling over stdio did nothing until the query ended. The handler spawns the run
  and aborts the task when it is dropped, so a cancelled query is dropped at its next yield and
  the reply is immediate.
- **Runtime errors** are published after the check's diagnostics for the text that ran, and
  cleared on the next edit (the check after it publishes without them). A run of a text that was
  edited meanwhile publishes nothing.
- **CodeLens**: "▶ Run" over each query statement and "▶ Run file" next to the first one when
  there are several. A lens's range is its whole statement, so the client finds the statement at
  the cursor from the lenses. A syntax-only server returns no lenses and refuses
  `datascript/run` (`RequestFailed`, -32803); the client also hides lenses and refuses to run
  while the workspace is untrusted.
- **No folder open**: an untitled document resolves relative paths against the workspace folder,
  or else the server's working directory. The VS Code client starts the server in the first
  workspace folder, or else in the home directory, so with no folder `from 'data.csv'` reads
  `~/data.csv`. Refusing to run would also have been possible, but a scratch query over an
  absolute path is a reasonable thing to do with no folder open.
- **Cancellation** in VS Code: Ctrl+C in the "DataScript" terminal, closing it, or running the
  same document again cancels the request.
*(M3-05, 2026-11-22.)*

### D-057 — Completion, hover and signature help (M3-02)
**Accepted.**

- **Where**: `datascript_compiler::ide`, so every frontend gives the same answers. Signatures:
  `complete` / `hover` / `signature_help(src: &SourceFile, analysis: &Analysis, offset: usize)`.
  ARCHITECTURE.md sketched `complete(&Analysis, offset)`, but an `Analysis` holds no text or tree,
  and keeping the tree in it would make every `check` result carry it for the few callers that
  want it. Parsing again is cheap; the caller must pass the text it checked.
- **Context from the token before the word.** In incomplete code the word being typed often
  lands in an `ERROR` node, while the tokens before it parsed normally, so the context (after
  `|`, after `from`, inside `with (`, a reader's options, after a complete expression…) is read
  from the previous token and its ancestors. Columns are the output of the stage before the
  cursor's operator, found in `Analysis::stage_schemas` by span.
- **Items are not filtered** by what is typed; editors filter and rank fuzzily. The order is:
  columns (in table order), properties, values, tables, scalar `let`s, functions, operators,
  keywords.
- **Files**: the compiler reads no files, so it returns `Completions::files` (the directory typed
  so far, and whether the cursor is inside quotes) and the frontend lists that directory: data
  files with a known format, and folders. Outside a string the names are inserted quoted.
- **The LSP checks afresh for each request** (`Session::check`, warm schemas make it ~1 ms), since
  completion usually comes right after an edit, before the debounced check. A syntax-only server
  checks with `NoSchemas` and lists no files: keywords, operators, functions and `let` names, no
  columns (D-053). Every call into the compiler is behind `catch_panic`; a crash is logged to the
  client and the request answers nothing.
- **Not offered yet**: host tables (no input for them; M4-04), columns made by earlier `extend`
  items and `$left.`/`$right.` (M3-03).
*(M3-02, 2026-11-22.)*

### D-058 — Spec/code mismatches from the docs review (M2-14)
**Accepted.**

- **A glob's format comes from the pattern's extension.** LANGUAGE.md said all matches must
  share a format, as if the matches decided it. The compiler reads no files (ARCHITECTURE.md), and
  it needs the format before asking the provider (to report `DS4001` for a format the engine
  cannot read), so it cannot look at the matches. `'logs/*'` stays `DS3002`, now labelled "the
  pattern has no known extension", and its help already offers `read_csv('logs/*')`. A pattern
  with an extension (`'logs/*.csv'`) only matches files of that format, so the old rule holds.
- **CSV `delimiter`, `quote` and `escape` are one ASCII character.** The engine splits on single
  bytes, so a non-ASCII character such as `§` read with half a UTF-8 sequence. The binder now
  rejects it with the existing `DS2041` "must be a single character", labelled with why.
- **`timespan / real` truncates toward zero**, as the spec said and like `timespan / long`; it used
  to round (`5us / 2.0` was `3us`, now `2us`). `timespan * real` still rounds. A timespan divided by
  zero, `0` or `0.0`, is `DS4003` "timespan division by zero" (it used to be an overflow for `0.0`
  and suggested converting to `real` for `0`, which a timespan cannot do).
- **Short-form timespan fractions use every digit**, then truncate to µs. The lexer kept nine
  fraction digits whatever the unit, so `0.0000000005d` (43.2 µs) read as zero.
- **Arrow `Null` columns are `string`** in `from_arrow_source` too, as §5.1 says; the DataFusion
  glob merge still checks for `Null` first so it takes the other files' type (D-044).
- **Doc-only fixes:** a constant `matches regex` pattern is reported by `ds check` (D-049), and the
  `"1" + 1` help suggests `toreal()` (`tolong()` for `%`), since a string column may hold `"1.5"`.
*(M2-14, 2026-11-22.)*

### D-059 — Optional trailing commas in lists (M2-19)
**Accepted.** Every comma-separated list in DataScript accepts an optional trailing `,` (**≠ KQL**;
KQL rejects trailing commas, so queries that use them cannot be pasted verbatim into Kusto).

- **Rationale.** During interactive exploration, users routinely comment out the last line or
  reorder items in multi-line `project`, `extend`, `summarize`, `case(...)`, and
  `render ... with (...)` clauses. Accepting optional trailing commas removes edit friction and
  keeps diffs single-line.
- **List positions covered.**
  - Delimited `(...)` lists: function and reader call arguments, `in (...)` value lists, and
    `render ... with (...)` property lists (including after a multi-value property such as
    `ycolumns = a, b,`).
  - Operator lists terminated by `|`, `;`, `)`, or EOF: `project`, `extend`, `project-away`,
    `project-rename`, `distinct <cols>`, `sort by` / `order by`, `join ... on`, and `union`.
  - `summarize` lists: both the aggregate list before `by` and the grouping key list after `by`.
    A comma before `by = expr` is not trailing, preserving a column named `by` (matching
    `Parser::at_by`).
  - Whitespace and comments after a trailing comma are ignored automatically because the parser
    operates on significant tokens.
- **Statement-boundary recovery.** A trailing `,` before `let` (when not `let =`, which remains
  `DS0126`) or a line-initial statement-starting `from` at depth 0 (D-037) ends the list so
  `from 'a.csv' | project a,\nfrom 'b.csv'` and `from 'a.csv' | project a,\nlet x = 1` recover
  cleanly as `DS0106` (missing `;`) while keeping `a,\nb`, `a,\nfrom | ...`, `a,\nfrom - 1`, and
  `f(a,\nfrom 'b.csv')` as list continuations.
- **Non-lists and malformed commas stay errors.** Empty-list commas (`(,)`), leading commas
  (`, a`), consecutive commas (`a,, b`, `a,,`), and non-list constructs (`where a, b` `DS0115`,
  `where a,` `DS0101`, `top 5 by amount desc,` `DS0101`, `take 5,` `DS0101`, `between (1 .. 10,)`
  `DS0101`, `(a,)` `DS0101`, `a[0,]` `DS0101`, `distinct *,` `DS0116`, and `long(null,)` `DS0101`)
  remain syntax errors. `at_typed_null` matches `type(null,)` directly so `long(null,)` reports
  `DS0101` instead of falling through to a call and reporting `DS1004` ("unknown function `long`").
- **`DS0127` retired.** `DS0127` (trailing comma at the end of a list) is retired (`CodeStatus::Retired`,
  no constant, `severity: None`), never to be reused.
*(M2-19, 2026-11-22.)*

### D-061 — Web front-end stack: React, Vite, Tailwind CSS, CodeMirror 6, Apache ECharts
**Accepted.** Closes Q-002. *(D-060 is M3-03.)* The shared `web/` SPA (used by `ds ui`, the M5
WASM playground, M3-06's VS Code results panel via component reuse, and a potential future Tauri
desktop shell) standardizes on:
- **React + TypeScript + Vite + Tailwind CSS**: the React runtime (~45 KB gzipped) is negligible
  next to CodeMirror, ECharts, and the WASM engine (size not yet measured; M5-01), and provides the strongest ecosystem for
  virtualized grids (`@tanstack/react-virtual`), drag-and-drop cell reordering, and accessible UI
  primitives.
- **CodeMirror 6**: lightweight, multi-instance friendly for notebooks, and small enough for WASM.
- **Apache ECharts** (tree-shaken Canvas modules): covers all 9 `RenderSpec` kinds (with a custom
  KPI card component for `card`), handles large point counts and interactive zoom, exports
  PNG/SVG, and supports matching dark/light themes.
- **Desktop readiness**: all UI state talks through a typed `Runtime` interface (`ServerRuntime`,
  `WasmRuntime`, and later `TauriRuntime` wrapping Rust's `Session` natively with no Electron
  overhead). See [WEB_UI.md §6](WEB_UI.md#6-architecture-stack--multi-target-reuse).
*(Web UI design session, 2026-11-27.)*

### D-062 — Notebook-first local web UI with `.dsnb` source-only Markdown format
**Accepted.** `ds ui` is a notebook-first local data workbench rather than a single scratch query
box (full specification in [WEB_UI.md](WEB_UI.md); language rules in
[LANGUAGE.md §2.1](LANGUAGE.md#21-notebooks-dsnb)). Key choices:
- **Single canonical `.dsnb` format & parser**: strictly source-only plain-text Markdown (no
  persisted cell metadata in v1, no cached outputs on disk; optional local caching under the user's
  OS cache dir / WASM `IndexedDB` is deferred to M4-07). A small, dependency-free CommonMark
  top-level fence scanner/serializer and `StitchedProgram` live in `datascript-compiler` (M4-08,
  `wasm32`-compatible), reused across `datascript-server`, `ds run`/`ds check`/`ds lsp` (M4-06),
  `datascript-cli` (`ReplProgram`), and `datascript-wasm` (M5) — `web/` consumes structured cells
  with opaque `originId`s via `Runtime` for lossless diff saves and never duplicates the parser.
  Supersedes D-048's note that the web UI does not need a shared stitcher.
- **Cross-cell `let` scope & `DS4061`**: top-to-bottom lexical order keeping `DS1003` no-shadowing
  (D-007) so `.dsnb` notebooks stay 1:1 with `.ds` scripts. Live `POST /api/check` checks the whole
  notebook in one pass; single-cell runs prepend only the cell's transitively referenced upstream
  `let`s, and `StitchedProgram` emits `DS4061` when the run is blocked by a broken referenced `let`
  or by an upstream syntax error that may define a referenced name. Warnings, upstream queries, and
  unreferenced broken `let`s do not block. The exact rule, the deliberate over-approximation of
  references, and the diagnostic's shape are defined once in [WEB_UI.md §3](WEB_UI.md#transitive-upstream-let-prepending--ds4061). Lazy
  `let` re-scans are surfaced via distinct `SourceRef` counts (`PlanNode::Scan`) per result;
  `let`-only validation suppresses `DS0004`.
- **Auth, persistence, CSP & workspace safety**: `ds ui` exchanges the launch `?token=` for an
  `HttpOnly; SameSite=Strict` cookie holding a separate random session id, named per port
  (`ds_session_<port>`) because cookies are shared across ports on `127.0.0.1` (so reloads stay
  signed in and two `ds ui` instances don't sign each other out), sends a strict CSP header, stores
  per-root preferences in the user's OS config dir (`dirs::config_dir()`) so `ds ui`'s random port
  never resets settings, saves files atomically (per-path lock + temp file + `sync_all` + rename),
  and inserts paths relative to the active notebook's parent directory. The workspace file API
  never writes or deletes data files and rejects in-root symlink components
  ([WEB_UI.md §4.1](WEB_UI.md#41-left-pane--working-directory-tree--attached-tables),
  [§6](WEB_UI.md#server-auth-csp--api-contract-datascript-server-m4-01-m4-10-m4-04--m4-05)).
- **Staged delivery & shortcuts**: M4 v1 (`M4-08`, `M4-01`, `M4-10`, `M4-02`, `M4-09`, `M4-03`,
  `M4-04`) ships a single-notebook view, read-only tree + atomic `.dsnb` creation, Scratchpad quick
  previews, grid + ECharts (plain table queries stay in Table view by default with a **Visualize**
  helper appending `| render ...`), auto-save with `409` conflicts, and a right-hand inspector
  starting collapsed by default. Multi-file tabs & pinning, atomic no-clobber rename (`rustix`) &
  per-file+`rmdir` folder delete, Outline/Scope tabs, Cell Command Mode, Command Palette
  (`Cmd/Ctrl+K` primary), and Quick Open follow in `M4-05` ([WEB_UI.md §5](WEB_UI.md#5-keyboard-shortcuts--command-mode)).
*(Web UI design session, 2026-11-27.)*

### D-063 — Multi-statement cells use compact result sub-tabs
**Accepted.** Closes Q-004. Running a DataScript cell (or `.ds` script / VS Code M3-06 run) executes
all query statements in that cell and displays compact result sub-tabs (`Result 1`, `Result 2`, …,
using each query's `render` title when present) whenever a run produces more than one output. A
secondary shortcut (`Cmd/Ctrl+Alt+Enter`, plus cell gutter action) runs only the statement at the
cursor. `let`-only cells validate bindings (suppressing `DS0004`) and display a compact confirmation
line rather than an empty grid.
*(Web UI design session, 2026-11-27.)*

### D-064 — UI result display cap of 10,000 rows with full streaming export
**Accepted.** Closes Q-007. Interactive UI runtimes (`ds ui`, VS Code M3-06 results panel, and WASM
M5) cap streamed results at `10_000` rows by default (configurable per workspace root in the user's
OS config dir for `ds ui` or `localStorage` for WASM, plus per-run override), enforced on streaming
runs via a row-limiting helper in `datascript-engine` that slices the final batch and drops the
stream to cancel execution. The helper is extracted from `ResultStream::collect`, which then calls
it, so there is one truncation implementation (D-043). When truncated, the runtime emits `DS4060`, client-side grid
sorting and Result Profiler stats label `"of first {N} rows"` using the run's actual `rowLimit`,
column headers offer **Sort in query** (`| sort by <col> desc/asc`, inserted before any trailing
`| render ...` or updating an existing `sort` in place), and the footer offers one-click **Export
full result**. Exports send the immutable run-time cell snapshot (re-executing the cell-relative
`statementIndex` with the original `now_micros` clock per D-015 and upstream `let` context against
current disk files) over a two-step cookie+ticket download (`POST /api/export` → one-shot
`GET /api/export/<ticket>`, [WEB_UI.md §6](WEB_UI.md#server-auth-csp--api-contract-datascript-server-m4-01-m4-10-m4-04--m4-05))
so `datascript_engine::writers` streams straight to disk without buffering in browser memory.
v1 export formats are CSV, NDJSON, JSON (a new streaming array writer) and Arrow IPC (a new
streaming writer), all added or reused in M4-10; Parquet and `.xlsx` appear once M2-27 and M2-28
land, and the menu lists only what the server can write.
*(Web UI design session, 2026-11-27.)*

## Proposed (open questions)

### Q-003 — `dynamic` representation
JSON-encoded strings (simple, works everywhere) versus Arrow structs/unions or the emerging Arrow
Variant type (faster, more complex). Decide in M2-03.

### Q-005 — Quoted identifiers vs path strings
`['order id']` (KQL bracket syntax) is a column name, while `'orders.csv'` after `from` is a
path (`from 'orders.csv'`, D-036). This is unambiguous to the parser, but check with users that it is not confusing.

*Deferred (M1-13, 2026-11-22):* nothing to decide without users. M1 keeps both forms; the
`DS1001` help already says "a column name is written without string quotes" when a string is
used where a column is expected. Revisit with the first user feedback, carried over to M6-01.

### Q-006 — Hive-style partitioned directories
DuckDB users expect `'data/year=*/month=*/*.parquet'` to expose `year` and `month` as columns.
DataFusion's `ListingTable` supports this. Decide whether it is automatic, opt-in
(`read_parquet(..., hive = true)`), or later. Not needed for M1.

### Q-008 — Does KQL error on integer overflow and integer division by zero?
D-014 says these are runtime errors and calls that "following KQL". Check real KQL behaviour in
M1-09. If KQL wraps or returns null, either keep the rule and mark it ≠ KQL in LANGUAGE.md §4.1,
or follow KQL. Either way, record the answer here.

*M1-09 (2026-11-21):* Microsoft's KQL documentation does not say; the numerical operators page
covers truncating integer division and null propagation only, and no Kusto cluster was available
to try it. DataScript keeps the rule (errors, `DS4003`), which the DataFusion engine now
implements without deviations. Still open: whoever can run KQL should check `print
9223372036854775807 + 1` and `print 1 / 0`, then either mark the rule ≠ KQL in LANGUAGE.md §4.1 or
close this question.

*Deferred (M1-13, 2026-11-22):* still no Kusto cluster, and the documentation still says nothing
about scalar integer overflow or division by zero (only `series_divide` documents IEEE behaviour
for its element-wise division). The rule stays, LANGUAGE.md §4.1 keeps its "not yet verified"
note, and the check is carried over to M6-01, whose reference docs carry the ≠ KQL marks.
