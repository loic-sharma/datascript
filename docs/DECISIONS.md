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
and `ds ui` invalidate when files may have changed (M1-12, M3-01, M4-01). The cache key is the
base directory, the sandbox and the `SourceRef`, because the sandbox can refuse a path that is
fine without it. **The row limit is enforced by the session**, not the backend: `ResultStream::
collect` stops after `row_limit` rows, drops the stream (cancelling the query) once one more row
arrives, and sets `QueryResult::truncated`. How frontends word that is Q-007's (M4-01).
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
**Globs are read file by file.** The engine infers each file's schema on its own and merges them
by name. When every file has the same schema, one `ListingTable` reads them all; otherwise each
file is its own table, projected to the merged columns (typed nulls where a file lacks one) and
combined with `UNION ALL` in path order, because DataFusion applies a table schema to CSV files by
position. **The engine remembers what `schema_of` found** (files and per-file schemas, keyed like
the session cache: base directory, sandbox, `SourceRef`), and the scan uses it instead of
inferring again; every `schema_of` call reads afresh and replaces the entry, so a session that
invalidates also refreshes the engine. **Columns with only nulls** (an all-empty CSV column, Arrow
`Null`) read as `string`, and in a glob take the other files' type; LANGUAGE.md §5.1 has the row.
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

*M1-09 (2026-11-21):* Microsoft's KQL documentation does not say; the numerical operators page
covers truncating integer division and null propagation only, and no Kusto cluster was available
to try it. DataScript keeps the rule (errors, `DS4003`), which the DataFusion engine now
implements without deviations. Still open: whoever can run KQL should check `print
9223372036854775807 + 1` and `print 1 / 0`, then either mark the rule ≠ KQL in LANGUAGE.md §4.1 or
close this question.
