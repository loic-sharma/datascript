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
script's directory. Chosen by the maintainer.

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
2026-11-21.)*

## Proposed (open questions)

### Q-002 — Web front-end stack
Proposal: TypeScript + Vite + **CodeMirror 6** (smaller than Monaco, easier to embed in WASM
builds) + a light UI layer (Svelte or Preact) + **Vega-Lite** or **ECharts** for charts. Decide at
the start of M4 (task M4-02). The VS Code results panel should reuse the same components.

### Q-003 — `dynamic` representation
JSON-encoded strings (simple, works everywhere) versus Arrow structs/unions or the emerging Arrow
Variant type (faster, more complex). Decide in M2-03.

### Q-004 — Multiple result statements in the web UI
Show tabs per result, or stack them? Decide in M4-02.

### Q-005 — Quoted identifiers vs path strings
`['order id']` (KQL bracket syntax) is a column name, while `'orders.csv'` at a pipe head is a
path. This is unambiguous to the parser, but check with users that it is not confusing.

### Q-006 — Hive-style partitioned directories
DuckDB users expect `'data/year=*/month=*/*.parquet'` to expose `year` and `month` as columns.
DataFusion's `ListingTable` supports this. Decide whether it is automatic, opt-in
(`read_parquet(..., hive = true)`), or later. Not needed for M1.

### Q-007 — Result size limits in UI runtimes
The CLI streams; the web UI, VS Code panel and WASM must not buffer a 10 GB result. Proposal: UI
runtimes fetch the first N rows (default 10 000) with a "results truncated" info diagnostic and an
option to export the full result. Decide in M4-01.

### Q-008 — Does KQL error on integer overflow and integer division by zero?
D-014 says these are runtime errors and calls that "following KQL". Check real KQL behaviour in
M1-09. If KQL wraps or returns null, either keep the rule and mark it ≠ KQL in LANGUAGE.md §4.1,
or follow KQL. Either way, record the answer here.
