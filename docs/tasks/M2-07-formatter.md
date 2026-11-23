# M2-07 — Formatter (`ds fmt`)

**Status:** todo
**Depends on:** M1-13
**Crates:** `datascript-syntax`, `datascript-cli`

## Goal
An opinionated, whitespace-only formatter over the CST: keep short single-line queries on one line
when they fit comfortably, preserve already-split pipelines and expand longer ones to one operator
per line with leading `|`, normalize spacing with 2-space indentation, and preserve all tokens and
comments verbatim.

## Acceptance criteria
- [ ] `ds fmt [--check] <files>`.
- [ ] Idempotent and comment-preserving over the whole golden corpus (an automated test).
- [ ] Refuses to format files with syntax errors, saying why.

## Carried over
- [ ] (from planning, 2026-11-22) If M3-03 is done, add `textDocument/formatting` to the language server (M3-03 left it for after this task), with an LSP test.
- [ ] (from M1-24) When a pipeline is multi-line, `from <source>` stays on the first line, with operators on the following lines: `from orders⏎| where …`, never `from⏎orders`.
- [ ] (from M2-19) Never add or remove a trailing `,` in comma-separated lists (preserve an existing trailing `,` verbatim, since D-059 makes trailing commas optional and the formatter is whitespace-only); treat a trailing `,` as a magic trailing comma that formats its list multi-line (never collapsing it onto one line) and keeps the `,` attached to the last item's line before `)`, `;`, `by`, or the next `|`.

## Formatting rules

- **Whitespace-only contract:**
  - The formatter **only** edits whitespace trivia between tokens — it never adds, removes, or
    rewrites non-trivia tokens.
  - Semicolons (including any trailing `;` at EOF), optional trailing commas in lists (D-059 —
    never added or removed), string quote styles (`'...'` vs `"..."`),
    verbatim strings (`@"..."`, `@'...'`), escape sequences, number spellings (`1_000_000`, `.5`,
    `1e-3`), single-token literal text (including interior spaces inside `datetime( 2024-01-31 )`
    or `timespan( 1d )`), quoted identifiers (`['order id']`, `["it's"]`), and operator aliases
    (`sort` vs `order`, `take` vs `limit`, explicit `kind = inner`) are all preserved verbatim.
  - DataScript strings are single-line (`DS0002` on unescaped newlines). Multiline `/* ... */`
    block comments preserve their interior lines verbatim (only leading whitespace before the
    opening `/*` is adjusted to match surrounding indentation).
- **Line width, indentation & line endings:**
  - 100-column soft target; **2 spaces** per indentation level (spaces, never tabs); LF (`\n`) line
    endings; no trailing spaces; a single trailing newline at EOF (whitespace-only files format to
    empty).
  - All `+N` indents below are relative to the enclosing stage's `|` column (or the enclosing
    statement's start column). Because multi-line tabular `let` stages sit at column 0, wrapped
    constructs inside them use the exact same indentation as top-level queries.
- **Pipelines & leading `|`:**
  - **Single-line when written on one line and it fits:** If a pipeline (a query statement or a
    tabular `let`) has no newline between its stages in the input, fits within 100 columns
    (including any `;` on that line), has no internal comments, and has no multi-line stages or
    sub-expressions (such as a list with a trailing `,`, or a wrapped `case(...)`, `read_csv(...)`,
    `render ... with (...)`, or wrapped subquery), keep it on one line:
    `from 'orders.csv' | where amount > 10 | take 5`.
  - **Already-split pipelines stay split:** If the input already has a newline before any `|` stage
    of a pipeline, keep the pipeline multi-line (one `| <operator>` per line), even if it would
    fit within 100 columns.
  - **`from <source>` stays on line 1 (M1-24):** Never break between `from` and `<source>`.
    `from\n  orders\n  | count` pulls `orders` onto line 1 while keeping `| count` on line 2
    (because the input already had a newline before `|`). If `<source>` is a long reader call
    (`read_csv(...)`), `from read_csv(` starts line 1 and its arguments wrap inside `(...)`.
  - **Multi-line layout:** Put each `| <operator>` on its own line with a leading `| ` (one space
    after `|`), at **column 0** for both top-level queries and tabular `let`s (`let name = from
    <source>` on line 1, `| <operator>` at column 0 — never indented).
  - **Parenthesized pipelines:** Inside `join`, `union`, `from (...)`, or a statement starting with
    `(from ... | ...)`, keep the parenthesized pipeline inline if it had no newline between its
    stages, its enclosing line fits within 100 columns, and it has no internal comments or
    multi-line sub-expressions. Otherwise break after `(` and before `)` and format the inner
    pipeline multi-line (`from` and each `|` stage indented +2 spaces relative to the enclosing
    line).
  - Hyphenated operator names (`project-away`, `project-rename`) have no spaces around `-`.
- **Spacing:**
  - Single space around `|` on single-line pipelines, around binary operators (`+`, `-`, `*`, `/`,
    `%`, `==`, `!=`, `<`, `<=`, `>`, `>=`, `=~`, `!~`, `and`, `or`, `in`, `!in`, `in~`, `!in~`,
    `between`, `!between`, `has`, `!has`, `contains`, `startswith`, `endswith`, `_cs` variants,
    `matches regex`), around `..` in `between (lo .. hi)`, and around `=` everywhere (`let`, column
    aliases, `project-rename`, `join kind = ...`, named function arguments, `render ... with (...)`
    properties).
  - No space after unary `-` (`-x`, `-(a + b)`). Keep a space between binary `-` and unary `-`
    (`a - -b`).
  - Prefix `not`: single space before a non-parenthesized operand (`not active`, `not a == b`); no
    space before `(` in the call-like form (`not(active)`), which binds like a call (D-029).
  - No space before `,` or `;`; single space after `,` on single-line lists. On wrapped lists, a
    trailing `,` stays attached to the last item's line (with no space before `,`, and immediately
    before any trailing `;` on an operator list, e.g. `customerId,;`).
  - No space inside `(...)` (except inside single-token literals like `datetime( 2024-01-31 )`,
    where verbatim token text wins), between a function name and `(`, around `.` in member access
    (`$left.a`, `record.field`), or before `[` in indexing (`items[0]`).
  - **Bracket indexing vs quoted identifiers:** Non-string index expressions have no spaces inside
    `[...]` (`items[0]`, `items[i + 1]`). When `[` is followed by a string literal (`INDEX_EXPR`),
    normalize to a single space inside `[` and `]` (`payload[ 'key' ]`, `payload[ "key" ]`) —
    **never** remove the space between `[` and a quote, which would re-lex as a `QUOTED_IDENT`
    token (`['key']`, `LANGUAGE.md` §1 / D-034).
  - Single space before `(` in `in (...)` / `!in (...)` / `in~ (...)` / `!in~ (...)` and
    `between (...)` / `!between (...)`.
- **Wrapping order, lists & clauses (magic trailing commas):**
  - **Wrapping order & reflow:** Wrap the **outermost** wrappable construct first (e.g. pipeline
    stages before operator lists, operator lists before call arguments), then re-measure each inner
    construct at its new indentation and wrap it only if it still exceeds 100 columns (or contains
    comments, a trailing `,`, or multi-line children). Unlike pipelines (which preserve user-split
    `|` lines), non-pipeline constructs — operator lists, `summarize`, `case(...)`, function calls,
    `render ... with (...)`, and binary/boolean expressions — **reflow back to one line** when they
    fit within 100 columns and contain no comments, **no trailing comma**, and no multi-line
    children.
  - **Trailing commas (magic trailing comma, D-059):** DataScript accepts an optional trailing `,`
    in every comma-separated list (D-059). Because the formatter is whitespace-only, it **never**
    adds or removes a trailing `,`. Instead, an existing trailing `,` acts as a **magic trailing
    comma**: any list ending with a trailing `,` (delimited `(...)` lists, operator lists,
    `summarize` aggregate or `by` lists, and `render ... with (...)` property lists) **always
    formats multi-line** and never reflows back onto one line, even when it fits within 100
    columns. The trailing `,` stays attached to the last item's line before `)`, `;`, `by`, or the
    next `|`. Lists without a trailing `,` never have one added and reflow when short.
  - **Single-item hugging:** When a list-taking operator (`project`, `extend`, `union`, etc.) has
    **exactly one item**, **no trailing comma on the operator list itself**, and its trailing
    construct wraps (e.g. `| extend tier = case(`, `| union (`), that single item stays on the
    operator line rather than breaking onto a new line. If the inner construct has a trailing `,`
    (e.g. `| extend size = case(a, b, c,)`), the single item still hugs `| extend` while the inner
    construct wraps with `,` attached to its last item before `)`. If the outer operator list itself
    has a trailing `,` (e.g. `| extend tier = case(...),`), the magic trailing comma breaks the
    outer operator list so the item indents at +2 with the trailing `,` attached to its end.
  - **List-taking operators** (`project`, `extend`, `project-away`, `project-rename`, `distinct`,
    `sort by`, `union`, `join ... on`): stay on one line when the full line fits within 100 columns
    and contains no comments, no trailing `,`, and no multi-line children; otherwise break after the
    operator keyword (`| project`, `| sort by`, `| union`, `) on`, etc.) with one item per line
    indented +2 spaces. For `join` with a parenthesized right-hand subquery, break the subquery
    `(...)` **before** wrapping the `on` condition list due to width; wrap `on` if `) on ...` still
    exceeds 100 columns, contains comments, or has a trailing `,`.
  - **`summarize`:** Stays on one line if it fits and has no comments or trailing `,` in either
    list; when wrapped:
    - Aggregates indent **+4 spaces** (+2 under `summarize`), whether or not a `by` clause is
      present.
    - When both aggregates and `by` are present, `by` aligns under `summarize` at **+2 spaces**, and
      `by` keys stay on the `  by ...` line if it fits within 100 columns (with no comments, no
      trailing `,`, and no multi-line expressions), or wrap to **+4 spaces** (+2 under `by`)
      otherwise. (Per D-059, `by = expr` is a named aggregate rather than a `by` clause, so
      `summarize sum(amount), by = count()` has no trailing comma and stays single-line when it
      fits.)
    - A `by`-only `summarize` (`| summarize by ...`) exceeding 100 columns, containing comments, or
      ending with a trailing `,` breaks after `by` with keys at **+2 spaces** (matching `| sort by`).
  - **`top N by`:** Stays on one line if it fits; if the sort key exceeds 100 columns, break after
    `by` and indent the sort key +2 spaces (wrapping inside the key expression if it still exceeds
    100 columns).
  - **`render <kind> with (...)`:** Stays on one line if it fits and has no comments or trailing
    `,`; when wrapped, each property sits on its own line at +2 indent with `)` aligned with `|`.
    Multi-value properties such as `ycolumns = a, b` stay on the property's line unless they exceed
    100 columns (preserving any trailing `,` at the end of the last property, e.g.
    `ycolumns = revenue, margin,`).
  - **Function calls & `in (...)`:** Wrap after `(` with one item per line at +2 indent and `)` at
    the outer indent when they exceed 100 columns, have a trailing `,`, or contain
    comments/multi-line arguments.
  - **`case(...)`:** Stays on one line if it has at most 2 condition/value pairs, contains no
    comments, has no trailing `,`, and fits within 100 columns; otherwise wraps after `(` with each
    `condition, value,` pair on its own line at +2 indent and the final `else` value on its own line
    (keeping any trailing `,` attached to that `else` line before `)`).
  - **Boolean & binary chains (`and` / `or`, arithmetic, comparisons) and parenthesized
    sub-expressions:** In `where` and other expressions (`extend`, `project`, `case`, `iff`,
    `countif`, `sumif`, etc.), when a chain exceeds 100 columns, break at the **lowest-precedence**
    operator first with leading operators: wrap before `or` at +2 indent, keeping each `and` group
    together unless that `and` group itself still exceeds 100 columns — in which case **every**
    wrapped `and` group inside that broken `or` chain (including the **first** `or` branch) wraps
    before `and` at +2 under `or` (+4 from `|`), so `and` never lands in the same column as `or`.
    Pure `and` or pure `or` chains wrap before each operator at +2 indent. Non-boolean binary chains
    (`+`, `-`, `*`, `/`, `%`, etc.) follow the same lowest-precedence-first, leading-operator rule.
    When a parenthesized sub-expression (such as `a and (b or c or ...) and d`) exceeds 100 columns
    or contains comments, break after `(` and before `)` with its body indented +2 and `)` aligned
    with the enclosing indent.
- **Semicolons & blank lines:**
  - Semicolons are preserved verbatim (never added or removed). A statement following `;` starts on
    a new line.
  - **Never insert a blank line after `;` automatically:** consecutive statements with no blank line
    between them in the input stay on consecutive lines; an intentional single blank line between
    statements is preserved.
  - **Collapse 2+ consecutive blank lines to 1** anywhere in the file.
  - Remove stray blank lines without comments inside a pipeline; preserve at most one blank line
    before a standalone comment introducing a new section of a pipeline.
- **Comments:**
  - Preserve every `//` line comment and `/* ... */` block comment in place relative to its
    surrounding tokens.
  - A **trailing end-of-line comment** after a statement's last token (`from orders | count // note`,
    `let cutoff = ago(30d); // rolling window`) stays on that line separated by a single space and
    does **not** force a single-line statement to become multi-line.
  - Any comment *inside* a pipeline (between stages or on an intermediate stage) forces the pipeline
    to format multi-line; comments inside a list, `case(...)`, `summarize`, or `with (...)` force
    the enclosing construct to wrap and stay attached to their item or branch.
  - Standalone comments before a pipeline stage stay directly above the following `| <operator>` at
    the same indentation level.
  - Inline `/* ... */` comments mid-expression stay in place separated by a single space from
    adjacent tokens (`where a /* note */ > 1`) without forcing a line break unless the comment is
    multi-line.
- **Syntax errors:** If the lexer or parser produces any **error-severity** diagnostic, refuse to
  format, report the diagnostics, and leave the file untouched. Non-error diagnostics (such as
  warning `DS0004` on a `let`-only script) do not block formatting.

## Examples (before and after)

### 1. Short single-line queries, already-split pipelines, and long pipelines

A short query written on one line (≤ 100 columns) stays on one line with normalized spacing:

Before:
```ds
from   'orders.csv'|where   amount>10  |  take   5
```

After:
```ds
from 'orders.csv' | where amount > 10 | take 5
```

A short query that is **already split across lines** stays split — while `from <source>` is pulled
onto line 1 (M1-24):

Before:
```ds
from
    orders
  |  where   amount > 10
  |  count
```

After:
```ds
from orders
| where amount > 10
| count
```

A single-line pipeline exceeding 100 columns expands to one operator per line with leading `|`,
keeping `from <source>` on line 1:

Before:
```ds
from
  orders
  | where   amount>100   and  status=='paid'|project orderId ,customerId,amount,timestamp|sort by timestamp desc|take   10
```

After:
```ds
from orders
| where amount > 100 and status == 'paid'
| project orderId, customerId, amount, timestamp
| sort by timestamp desc
| take 10
```

### 2. File sources, globs, and reader function calls

Short single-line file queries stay on one line; longer glob pipelines expand. Consecutive
statements with no blank line between them stay consecutive (no blank line added after `;`):

Before:
```ds
from   'orders.csv'|where amount>0|count;
from 'events/*.ndjson' | where level == "error" and service == "checkout" | project timestamp, message, traceId | take 20
```

After:
```ds
from 'orders.csv' | where amount > 0 | count;
from 'events/*.ndjson'
| where level == "error" and service == "checkout"
| project timestamp, message, traceId
| take 20
```

Reader calls (short single-line queries stay on one line; long options wrap inside `read_csv(...)`;
intentional blank lines are preserved while consecutive statements stay adjacent):

Before:
```ds
from read_csv('orders.csv',delimiter=';',header=true)|take 5;
from   read_parquet( 'a.parquet' )  |  getschema;

from read_csv('data/2024/quarterly_orders.tsv', delimiter='\t', header=true, null_value='NULL', infer_rows=5000, compression='gzip') | where amount > 0
```

After:
```ds
from read_csv('orders.csv', delimiter = ';', header = true) | take 5;
from read_parquet('a.parquet') | getschema;

from read_csv(
  'data/2024/quarterly_orders.tsv',
  delimiter = '\t',
  header = true,
  null_value = 'NULL',
  infer_rows = 5000,
  compression = 'gzip'
)
| where amount > 0
```

### 3. Scalar and tabular `let` bindings

Short single-line tabular `let`s stay on one line; multi-line tabular `let`s put every `|` stage at
**column 0** (aligned with `let`, not indented):

Before:
```ds
let  minAmount=100;
let   cutoff = ago(30d) ;
let recent=from 'orders.parquet'|where timestamp>=cutoff;

let topCustomers=from recent|where amount>minAmount|summarize revenue=sum(amount) by customerId|top 10 by revenue desc;

from recent|join kind=inner topCustomers on customerId|summarize monthlyRevenue=sum(amount) by customerId, month=startofmonth(timestamp)
```

After:
```ds
let minAmount = 100;
let cutoff = ago(30d);
let recent = from 'orders.parquet' | where timestamp >= cutoff;

let topCustomers = from recent
| where amount > minAmount
| summarize revenue = sum(amount) by customerId
| top 10 by revenue desc;

from recent
| join kind = inner topCustomers on customerId
| summarize monthlyRevenue = sum(amount) by customerId, month = startofmonth(timestamp)
```

### 4. Multiple queries in a script

Statements after `;` start on the next line without inserting a blank line. Because the formatter
only edits whitespace, a trailing `;` is preserved verbatim if present (and never added if absent):

Before:
```ds
from 'orders.csv' | summarize total = sum(amount) by region; from 'orders.csv' | where status == "refunded" and amount > 50 | project orderId, customerId, amount | sort by amount desc;
```

After:
```ds
from 'orders.csv' | summarize total = sum(amount) by region;
from 'orders.csv'
| where status == "refunded" and amount > 50
| project orderId, customerId, amount
| sort by amount desc;
```

A `let`-only script (warning `DS0004` does not block formatting; missing trailing `;` is kept as
written, and intentional blank lines are preserved):

Before:
```ds
let taxRate=0.08;
let recent = from 'orders.csv' | where timestamp >= ago(7d);

let highValue = from recent | where amount * (1.0 + taxRate) > 500 | project orderId, customerId, amount
```

After:
```ds
let taxRate = 0.08;
let recent = from 'orders.csv' | where timestamp >= ago(7d);

let highValue = from recent
| where amount * (1.0 + taxRate) > 500
| project orderId, customerId, amount
```

### 5. Expression spacing, tricky operators, bracket indexing & mixed `and`/`or`

Unary vs binary minus, `not` vs `not(...)`, string operators, `in` / `between`, nested calls,
member access, numeric indexing (`items[0]`), and string-literal indexing (`payload[ 'key' ]`,
keeping spaces inside `[ ... ]` so it cannot fuse into quoted identifier `['key']`):

Before:
```ds
from events
| where not   archived and not ( status== "deleted" ) and delta == - 1 and diff == a--b and total == -(x+y)
| where msg contains 'timeout' and path !startswith_cs '/health' and log has 'error' and tag =~ 'prod' and env !~ 'dev' and uri matches   regex '^/api/v[0-9]+'
| where code in(200,201,204) and region !in~ ( 'test' , 'staging' ) and latency between(10ms..500ms) and score !between ( -10 .. 10 )
| extend label = strcat( toupper( meta.env ) , ':' , items[ 0 ] , ':' , payload[  'key'  ] )
```

After:
```ds
from events
| where not archived
  and not(status == "deleted")
  and delta == -1
  and diff == a - -b
  and total == -(x + y)
| where msg contains 'timeout'
  and path !startswith_cs '/health'
  and log has 'error'
  and tag =~ 'prod'
  and env !~ 'dev'
  and uri matches regex '^/api/v[0-9]+'
| where code in (200, 201, 204)
  and region !in~ ('test', 'staging')
  and latency between (10ms .. 500ms)
  and score !between (-10 .. 10)
| extend label = strcat(toupper(meta.env), ':', items[0], ':', payload[ 'key' ])
```

Mixed `and`/`or` chains wrap at the lowest-precedence operator (`or`) first; an `and` group only
wraps (at +2 under `or`, i.e. +4 from `|`) if that branch itself exceeds 100 columns — including
when it is the **first** `or` branch, so `and` never shares a column with `or`:

Before:
```ds
from orders
| where status == "paid" and amount >= 500 or status == "pending" and amount >= 1000 and customerTier == "enterprise" and timestamp >= ago(7d) or isPriorityAccount and region == "US"
```

After:
```ds
from orders
| where status == "paid" and amount >= 500
  or status == "pending"
    and amount >= 1000
    and customerTier == "enterprise"
    and timestamp >= ago(7d)
  or isPriorityAccount and region == "US"
```

When the first `or` branch wraps, its `and` continuations still sit at +4 while `or` sits at +2.
When a parenthesized sub-expression exceeds 100 columns, it breaks after `(` and before `)` with
its body indented +2:

Before:
```ds
from orders
| where status == "pending" and amount >= 1000 and customerTier == "enterprise" and timestamp >= ago(7d) or isPriorityAccount and region == "US";

from orders
| where not archived and (status == "paid" or status == "invoiced" or status == "settled" or customerTier == "enterprise") and region == "US"
```

After:
```ds
from orders
| where status == "pending"
    and amount >= 1000
    and customerTier == "enterprise"
    and timestamp >= ago(7d)
  or isPriorityAccount and region == "US";

from orders
| where not archived
  and (
    status == "paid"
    or status == "invoiced"
    or status == "settled"
    or customerTier == "enterprise"
  )
  and region == "US"
```

### 6. Long `project`, `extend`, `project-rename`, `project-away`, and `distinct` lists

In a multi-line pipeline, short lists and `distinct *` stay on their operator's line, and short
already-wrapped lists or expressions without comments reflow back onto one line (while the
already-split pipeline itself stays multi-line):

Before:
```ds
from orders | project orderId , customerId , amount | extend net=amount-discount | project-rename id=orderId | project-away tmp* , debug_flag | distinct *;

from orders
| where
  amount > 100
  and status == "paid"
| project
  orderId,
  customerId,
  amount
```

After:
```ds
from orders
| project orderId, customerId, amount
| extend net = amount - discount
| project-rename id = orderId
| project-away tmp*, debug_flag
| distinct *;

from orders
| where amount > 100 and status == "paid"
| project orderId, customerId, amount
```

Lists exceeding 100 columns wrap with one item per line and 2-space indent (never adding a trailing
comma if the input did not have one):

Before:
```ds
from orders
| extend grossAmount = unitPrice * qty, discountAmount = round(unitPrice * qty * discountRate, 2), netAmount = unitPrice * qty * (1.0 - discountRate)
| project orderId, customerId, orderMonth = startofmonth(timestamp), grossAmount, discountAmount, netAmount
| project-rename order_id = orderId, customer_id = customerId, order_month = orderMonth, net_amount = netAmount
| project-away internal_*, debug_*, raw_payload, temp_notes, migration_flag, audit_trace_id, legacy_code
| distinct order_id, customer_id, order_month, net_amount, shipping_region, sales_channel, payment_method
```

After:
```ds
from orders
| extend
  grossAmount = unitPrice * qty,
  discountAmount = round(unitPrice * qty * discountRate, 2),
  netAmount = unitPrice * qty * (1.0 - discountRate)
| project
  orderId,
  customerId,
  orderMonth = startofmonth(timestamp),
  grossAmount,
  discountAmount,
  netAmount
| project-rename
  order_id = orderId,
  customer_id = customerId,
  order_month = orderMonth,
  net_amount = netAmount
| project-away
  internal_*,
  debug_*,
  raw_payload,
  temp_notes,
  migration_flag,
  audit_trace_id,
  legacy_code
| distinct
  order_id,
  customer_id,
  order_month,
  net_amount,
  shipping_region,
  sales_channel,
  payment_method
```

### 7. `summarize` with aggregates and `by` keys

Short single-line queries (aggregates + `by`, aggregates only, and `by` only) stay on one line,
with no blank line inserted between consecutive statements:

Before:
```ds
from orders | summarize orders=count(),revenue=sum(amount) by customerId;
from orders | summarize totalOrders=count(), totalRevenue=sum(amount);
from orders | summarize   by   customerId , region
```

After:
```ds
from orders | summarize orders = count(), revenue = sum(amount) by customerId;
from orders | summarize totalOrders = count(), totalRevenue = sum(amount);
from orders | summarize by customerId, region
```

Long `summarize` indents aggregates at **+4** (+2 under `summarize`), aligns `by` under `summarize`
at **+2**, and indents wrapped `by` keys at **+4** (+2 under `by`) when `  by ...` exceeds 100
columns (keeping short `by` keys on the `  by ...` line):

Before:
```ds
from orders
| summarize orderCount = count(), paidCount = countif(status == "paid"), totalRevenue = sum(amount), avgOrder = avg(amount), p95Order = percentile(amount, 95) by region, country, customerSegment, salesChannel, paymentMethod, fulfillmentCenter, day = bin(timestamp, 1d);

from orders
| summarize orderCount = count(), totalRevenue = sum(amount), avgOrder = avg(amount), maxOrder = max(amount) by region, bin(timestamp, 1d)
```

After:
```ds
from orders
| summarize
    orderCount = count(),
    paidCount = countif(status == "paid"),
    totalRevenue = sum(amount),
    avgOrder = avg(amount),
    p95Order = percentile(amount, 95)
  by
    region,
    country,
    customerSegment,
    salesChannel,
    paymentMethod,
    fulfillmentCenter,
    day = bin(timestamp, 1d);

from orders
| summarize
    orderCount = count(),
    totalRevenue = sum(amount),
    avgOrder = avg(amount),
    maxOrder = max(amount)
  by region, bin(timestamp, 1d)
```

Aggregates-only `summarize` still indents aggregates at **+4** (+2 under `summarize`), while
`by`-only `summarize` breaks after `by` at **+2** (matching `sort by`):

Before:
```ds
from orders
| summarize orderCount = count(), paidCount = countif(status == "paid"), totalRevenue = sum(amount), avgOrder = avg(amount);

from orders
| summarize by region, country, customerSegment, salesChannel, paymentMethod, fulfillmentCenter, day = bin(timestamp, 1d)
```

After:
```ds
from orders
| summarize
    orderCount = count(),
    paidCount = countif(status == "paid"),
    totalRevenue = sum(amount),
    avgOrder = avg(amount);

from orders
| summarize by
  region,
  country,
  customerSegment,
  salesChannel,
  paymentMethod,
  fulfillmentCenter,
  day = bin(timestamp, 1d)
```

### 8. `sort by` / `order by`, `top N by`, and `take` / `limit`

Short single-line queries stay on one line; `order by` and `limit` are preserved verbatim:

Before:
```ds
from orders | sort  by  region  asc , amount  desc  nulls  last | limit   25;
from orders | order by timestamp desc nulls first | top  10  by  amount  desc  nulls  last
```

After:
```ds
from orders | sort by region asc, amount desc nulls last | limit 25;
from orders | order by timestamp desc nulls first | top 10 by amount desc nulls last
```

Long `sort by` lists and long `top N by` keys break after `by` at +2 indent:

Before:
```ds
from orders
| sort by region asc nulls last, customerSegment asc, orderMonth desc, totalRevenue desc nulls last, customerId asc;

from orders
| top 10 by coalesce(discountedAmount, grossAmount) + shippingCost + handlingFee - credit desc nulls last
```

After:
```ds
from orders
| sort by
  region asc nulls last,
  customerSegment asc,
  orderMonth desc,
  totalRevenue desc nulls last,
  customerId asc;

from orders
| top 10 by
  coalesce(discountedAmount, grossAmount) + shippingCost + handlingFee - credit desc nulls last
```

### 9. `case(...)` expressions

Short inline `case(...)` (≤ 2 branches, query fits within 100 columns):

Before:
```ds
from orders | extend size=case( amount>=500 , "large" , "small" )
```

After:
```ds
from orders | extend size = case(amount >= 500, "large", "small")
```

Multi-branch `case(...)` (3+ branches or > 100 columns) wraps one `condition, value` pair per line
at +2 indent, followed by the final `else` value. Because `extend` has only one item, `tier = case(`
hugs the `| extend` line (single-item hugging rule):

Before:
```ds
from orders
| extend tier = case(amount >= 10000, "enterprise", amount >= 1000, "mid-market", amount >= 100, "standard", "starter")
```

After:
```ds
from orders
| extend tier = case(
  amount >= 10000, "enterprise",
  amount >= 1000, "mid-market",
  amount >= 100, "standard",
  "starter"
)
```

When a multi-line `case(...)` appears alongside other items in `extend` or `project`, the enclosing
operator wraps first (outermost first) at +2 indent and the `case` branches indent to +4:

Before:
```ds
from orders
| extend netAmount = amount - discount, tier = case(amount >= 10000, "enterprise", amount >= 1000, "mid-market", amount >= 100, "standard", "starter")
```

After:
```ds
from orders
| extend
  netAmount = amount - discount,
  tier = case(
    amount >= 10000, "enterprise",
    amount >= 1000, "mid-market",
    amount >= 100, "standard",
    "starter"
  )
```

### 10. `join` with parenthesized subqueries and complex `on` clauses

Short single-line `join` with an inline subquery stays on one line; a longer `join` breaks its
parenthesized subquery at +2 indent **before** wrapping `on`, keeping `) on ...` on one line when
it fits:

Before:
```ds
from orders | join (from customers | where active) on customerId;

from orders | join kind=leftouter (from customers | where active | project id, name, region) on $left.customerId==$right.id, $left.region==$right.region
```

After:
```ds
from orders | join (from customers | where active) on customerId;

from orders
| join kind = leftouter (
  from customers
  | where active
  | project id, name, region
) on $left.customerId == $right.id, $left.region == $right.region
```

When the `on` clause exceeds 100 columns, its conditions wrap at +2 indent:

Before:
```ds
from orders
| join kind = leftouter (from customers | where active | project customerId, region, effectiveDate, tier) on $left.customerId == $right.customerId, $left.region == $right.region, $left.orderDate == $right.effectiveDate
```

After:
```ds
from orders
| join kind = leftouter (
  from customers
  | where active
  | project customerId, region, effectiveDate, tier
) on
  $left.customerId == $right.customerId,
  $left.region == $right.region,
  $left.orderDate == $right.effectiveDate
```

### 11. `union` and parenthesized head pipelines

Short inline table `union` stays on one line:

Before:
```ds
from 'orders_2023.parquet'|union  'orders_2024.parquet' , 'orders_2025.parquet'
```

After:
```ds
from 'orders_2023.parquet' | union 'orders_2024.parquet', 'orders_2025.parquet'
```

When a `union` stage exceeds 100 columns, a single parenthesized pipeline hugs `| union (`
(single-item hugging) and multiple targets wrap one per line at +2 indent:

Before:
```ds
from 'orders_2024.parquet' | where amount > 0 | union (from 'orders_2025.parquet' | where amount > 0 and status == "paid" | project-away legacy_notes);

from 'orders_2023.parquet' | where amount > 0 | union 'orders_2024.parquet', (from 'orders_2025.parquet' | where amount > 0 and status == "paid" | project-away notes)
```

After:
```ds
from 'orders_2024.parquet'
| where amount > 0
| union (
  from 'orders_2025.parquet'
  | where amount > 0 and status == "paid"
  | project-away legacy_notes
);

from 'orders_2023.parquet'
| where amount > 0
| union
  'orders_2024.parquet',
  (
    from 'orders_2025.parquet'
    | where amount > 0 and status == "paid"
    | project-away notes
  )
```

Parenthesized pipelines at the head of a query follow the same outermost-first rule (short stays
inline; when the outer query wraps, each parenthesized pipeline stays inline if its own line fits,
or wraps inside `(...)` if its line still exceeds 100 columns):

Before:
```ds
(from 'a.csv' | take 5) | union (from 'b.csv' | take 5);

(from 'orders_2024.parquet' | where status == "paid" | top 10 by amount desc) | union (from 'orders_2025.parquet' | where status == "paid" | top 10 by amount desc);

(from 'orders_2024.parquet' | where status == "paid" and amount > 100 | project orderId, customerId, amount) | union (from 'orders_2025.parquet' | where status == "paid" and amount > 100 | project orderId, customerId, amount)
```

After:
```ds
(from 'a.csv' | take 5) | union (from 'b.csv' | take 5);

(from 'orders_2024.parquet' | where status == "paid" | top 10 by amount desc)
| union (from 'orders_2025.parquet' | where status == "paid" | top 10 by amount desc);

(
  from 'orders_2024.parquet'
  | where status == "paid" and amount > 100
  | project orderId, customerId, amount
)
| union (
  from 'orders_2025.parquet'
  | where status == "paid" and amount > 100
  | project orderId, customerId, amount
)
```

### 12. `render` operator

Short `render` queries stay on one line; once a query exceeds 100 columns it breaks per operator,
while a short `render ... with (...)` stage stays on its line:

Before:
```ds
from orders | summarize revenue = sum(amount) by region | render   barchart;
from orders | summarize revenue = sum(amount) by region | render barchart with(title="Revenue by Region")
```

After:
```ds
from orders | summarize revenue = sum(amount) by region | render barchart;
from orders
| summarize revenue = sum(amount) by region
| render barchart with (title = "Revenue by Region")
```

Long `render ... with (...)` wraps one property per line at +2 indent, keeping multi-column values
together on their property line:

Before:
```ds
from orders
| summarize revenue = sum(amount), margin = sum(profit) by region, month = startofmonth(ts)
| render linechart with (title="Monthly Revenue and Margin", xcolumn=month, ycolumns=revenue,margin, series=region, xtitle="Month", ytitle="USD", ymin=0, legend=visible)
```

After:
```ds
from orders
| summarize revenue = sum(amount), margin = sum(profit) by region, month = startofmonth(ts)
| render linechart with (
  title = "Monthly Revenue and Margin",
  xcolumn = month,
  ycolumns = revenue, margin,
  series = region,
  xtitle = "Month",
  ytitle = "USD",
  ymin = 0,
  legend = visible
)
```

### 13. Literals, quoted identifiers & block comments preserved verbatim

Quote styles (`'...'`, `"..."`), verbatim strings (`@"..."`, `@'...'`), escape sequences, number
spellings, single-token `datetime(...)` / `timespan(...)` text (including any interior spaces such
as `datetime( 2024-01-31 )` and `timespan( 1d )`), quoted identifiers (`['...']`, `["..."]`), and
multiline `/* ... */` block comment interiors are kept verbatim. By contrast, `long( null )` is four
separate tokens (`long`, `(`, `null`, `)`) and normalizes to `long(null)`. (DataScript strings are
always single-line; an unescaped newline inside a string is `DS0002`.)

Before:
```ds
/*  Quarterly report
      keeps exact
    internal indentation  */
let winPath = @"C:\data\orders.csv";
let rawTag = @'a\b\c';

from   'orders.csv'
| where ['order id'] != "" and amount > 1_000_000 and rate >= .5 and eps == 1e-3
| where timestamp >= datetime( 2024-01-31 ) and wait < timespan( 1d ) and jitter <= 5µs and window == timespan(1.02:03:04) and offset == timespan(-90m)
| extend note = "line1\nline2", ["it's"] = long( null ), ['let'] = string(null)
| project ['order id'], ["it's"], ['let'], note
```

After:
```ds
/*  Quarterly report
      keeps exact
    internal indentation  */
let winPath = @"C:\data\orders.csv";
let rawTag = @'a\b\c';

from 'orders.csv'
| where ['order id'] != "" and amount > 1_000_000 and rate >= .5 and eps == 1e-3
| where timestamp >= datetime( 2024-01-31 )
  and wait < timespan( 1d )
  and jitter <= 5µs
  and window == timespan(1.02:03:04)
  and offset == timespan(-90m)
| extend note = "line1\nline2", ["it's"] = long(null), ['let'] = string(null)
| project ['order id'], ["it's"], ['let'], note
```

### 14. Comments & blank lines

#### File-leading comments and section comments

A section comment above a short single-line query does not force that query onto multiple lines,
and blank lines are preserved only where present:

Before:
```ds
// Monthly revenue report
// Owner: analytics team

// Shared inputs
let orders='orders.parquet';
let   cutoff = ago(90d);

// Main query
from orders | where timestamp >= cutoff | summarize revenue = sum(amount) by region
```

After:
```ds
// Monthly revenue report
// Owner: analytics team

// Shared inputs
let orders = 'orders.parquet';
let cutoff = ago(90d);

// Main query
from orders | where timestamp >= cutoff | summarize revenue = sum(amount) by region
```

#### Comments between pipeline stages

Comments inside a pipeline always keep it multi-line and attach above the following `| operator`.
An intentional blank line before a section comment inside a pipeline is preserved:

Before:
```ds
from orders
  // Exclude cancelled and test orders
| where status != "cancelled" and not isTest

// Aggregate by month
| summarize revenue = sum(amount) by month = startofmonth(timestamp)
// Most recent first
| sort by month desc
```

After:
```ds
from orders
// Exclude cancelled and test orders
| where status != "cancelled" and not isTest

// Aggregate by month
| summarize revenue = sum(amount) by month = startofmonth(timestamp)
// Most recent first
| sort by month desc
```

#### Trailing end-of-line comments

A trailing end-of-line comment after a statement's last token does **not** force a single-line
statement onto multiple lines; trailing comments on intermediate pipeline stages keep the pipeline
multi-line:

Before:
```ds
let cutoff=ago(30d);    // rolling window
from 'orders.csv'|count;   // quick sanity check

from 'orders.csv'   // raw export
| where amount > 0     // drop refunds
| summarize revenue = sum(amount) by customerId   // per customer
| top 10 by revenue desc // leaderboard
```

After:
```ds
let cutoff = ago(30d); // rolling window
from 'orders.csv' | count; // quick sanity check

from 'orders.csv' // raw export
| where amount > 0 // drop refunds
| summarize revenue = sum(amount) by customerId // per customer
| top 10 by revenue desc // leaderboard
```

#### Inline block comments and comments inside multi-line constructs

Inline `/* ... */` comments stay in place with a single space on each side without forcing line
breaks. Any comment inside a list, subquery, `summarize`, or `case(...)` forces multi-line layout
and stays attached to its item (with `summarize` using +4/+2/+4 indentation):

Before:
```ds
from orders
| where amount /* exclude zero */ > 0
| project
  orderId, // primary key
  // Net amount after discount
  net = amount - discount
| join kind=leftouter (
  // Only active accounts
  from customers
  | where active
) on customerId
| extend tier = case(
  // Enterprise threshold
  net >= 1000, "enterprise",
  "standard" // fallback
)
| summarize
    // Total revenue
    revenue = sum(net)
  by
    tier // grouping key
```

After:
```ds
from orders
| where amount /* exclude zero */ > 0
| project
  orderId, // primary key
  // Net amount after discount
  net = amount - discount
| join kind = leftouter (
  // Only active accounts
  from customers
  | where active
) on customerId
| extend tier = case(
  // Enterprise threshold
  net >= 1000, "enterprise",
  "standard" // fallback
)
| summarize
    // Total revenue
    revenue = sum(net)
  by
    tier // grouping key
```

#### Collapsing excessive blank lines vs removing stray blank lines

2+ consecutive blank lines collapse to 1; stray blank lines without comments inside a pipeline are
removed, while intentional single blank lines between statements are preserved:

Before:
```ds
let minAmount = 100;


let maxAmount = 5000;



from 'orders.parquet'

| where amount > minAmount and amount < maxAmount


| project orderId, customerId, amount, timestamp

| take 10
```

After:
```ds
let minAmount = 100;

let maxAmount = 5000;

from 'orders.parquet'
| where amount > minAmount and amount < maxAmount
| project orderId, customerId, amount, timestamp
| take 10
```

### 15. Trailing commas (magic trailing comma)

#### Delimited lists and operator lists with trailing commas

Any comma-separated list ending with a trailing `,` (D-059) is preserved verbatim and formats
multi-line rather than reflowing onto one line, even when short (and expands its enclosing pipeline
to multi-line). Lists without a trailing `,` still reflow back to one line when short:

Before:
```ds
from read_csv('orders.csv', delimiter = ';',) | where code in (200, 201,) | project orderId , customerId , | take 5;

from orders
| extend
  net = amount - discount,
  tax = amount * 0.08
| project
  orderId,
  customerId,;
```

After:
```ds
from read_csv(
  'orders.csv',
  delimiter = ';',
)
| where code in (
  200,
  201,
)
| project
  orderId,
  customerId,
| take 5;

from orders
| extend net = amount - discount, tax = amount * 0.08
| project
  orderId,
  customerId,;
```

#### `summarize` with trailing commas (and a column named `by`)

A trailing `,` on aggregates wraps aggregates at +4 while keeping short `by` keys on `  by ...`; a
trailing `,` on `by` keys wraps the keys at +4 (+2 for `by`-only `summarize`). Remember that
`by = expr` names an aggregate column rather than starting a `by` clause (D-059), so
`summarize sum(amount), by = count()` has no trailing comma and stays single-line when it fits:

Before:
```ds
from orders | summarize revenue = sum(amount), by region;
from orders | summarize revenue = sum(amount) by region, country,;
from orders | summarize by customerId, region,;
from orders | summarize sum(amount), by = count()
```

After:
```ds
from orders
| summarize
    revenue = sum(amount),
  by region;
from orders
| summarize
    revenue = sum(amount)
  by
    region,
    country,;
from orders
| summarize by
  customerId,
  region,;
from orders | summarize sum(amount), by = count()
```

#### Single-item hugging, `case(...)`, `join ... on`, and `render ... with (...)`

When a single-item operator has no trailing `,` on its own list, a trailing `,` inside `case(...)`
wraps `case(...)` while still hugging `| extend`. When the outer operator list itself has a
trailing `,`, the magic trailing comma breaks the outer operator list instead of hugging. In
`join`, a trailing `,` on `on` wraps `on` without forcing a short inline subquery to expand. In
`render ... with (...)`, a trailing `,` (including after a multi-value property like
`ycolumns = revenue, margin,`) wraps `with (...)` with `,` attached to the last property line:

Before:
```ds
from orders
| extend size = case(amount >= 500, "large", "small",)
| extend tier = case(amount >= 500, "large", "small",),
| join (from customers | where active) on customerId,
| render linechart with (title = "Monthly", xcolumn = month, ycolumns = revenue, margin,)
```

After:
```ds
from orders
| extend size = case(
  amount >= 500, "large",
  "small",
)
| extend
  tier = case(
    amount >= 500, "large",
    "small",
  ),
| join (from customers | where active) on
  customerId,
| render linechart with (
  title = "Monthly",
  xcolumn = month,
  ycolumns = revenue, margin,
)
```

#### Multi-line lists with trailing commas and comments

Comments inside trailing-comma lists stay attached to their items (at +2 indent inside both
operator lists and delimited `(...)` lists, including a commented-out final item before `)`), while
the trailing `,` stays directly attached to the preceding item:

Before:
```ds
from orders
| project
    orderId , // primary key
    // amount,
    customerId , // trailing comma after last item
| render barchart with (
    title="Orders",
    // xtitle = "Customer",
)
```

After:
```ds
from orders
| project
  orderId, // primary key
  // amount,
  customerId, // trailing comma after last item
| render barchart with (
  title = "Orders",
  // xtitle = "Customer",
)
```

### 16. Edge cases & non-changes

#### Already-formatted query (idempotency)

Formatting an already-formatted script produces byte-for-byte identical output:

Before (and After — unchanged):
```ds
// Top customers over time
let orders = 'orders.parquet';
let recent = from orders | where timestamp >= ago(90d);

let topCustomers = from recent
| where status == "paid"
| summarize revenue = sum(amount) by customerId
| top 10 by revenue desc;

from recent
| join kind = inner topCustomers on customerId
| summarize revenue = sum(amount) by customerId, month = startofmonth(timestamp)
| render timechart with (title = "Top Customers")
```

#### Syntax error — formatter refuses to modify invalid input

Input with error-severity syntax diagnostics (here: `=` instead of `==` (`DS0110`) and a trailing
comma on a non-list operator (`DS0101`); trailing commas are only valid in comma-separated lists
per D-059):

```ds
from 'orders.csv' | where amount = 100 | take 5,
```

`ds fmt` reports the diagnostics, exits non-zero, and leaves the file untouched. Running
`ds fmt --check` on the same file also reports the syntax diagnostics rather than a formatting diff.

## Follow-ups
