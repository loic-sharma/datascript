# M2-07 — Formatter (`ds fmt`)

**Status:** todo
**Depends on:** M1-13
**Crates:** `datascript-syntax`, `datascript-cli`

## Goal
An opinionated formatter over the CST: one operator per line with leading `|`, normalized spacing,
and comments preserved.

## Acceptance criteria
- [ ] `ds fmt [--check] <files>`.
- [ ] Idempotent and comment-preserving over the whole golden corpus (an automated test).
- [ ] Refuses to format files with syntax errors, saying why.

## Carried over
- [ ] (from planning, 2026-11-22) If M3-03 is done, add `textDocument/formatting` to the language server (M3-03 left it for after this task), with an LSP test.
- [ ] (from M1-24) The formatter keeps `from <source>` on the first line of a pipeline, with operators on the following lines: `from orders⏎| where …`, never `from⏎orders`.

## Formatting rules

- **Line width & indentation:** 100-column soft target for wrapping; 4 spaces per indentation level
  (spaces, never tabs); LF (`\n`) line endings; no trailing spaces; a single trailing newline at
  EOF (whitespace-only files format to empty).
- **Pipelines & leading `|`:**
  - Keep `from <source>` on the first line of a pipeline — never break between `from` and
    `<source>` (M1-24). If `<source>` is a long reader call (`read_csv(...)`), `from read_csv(`
    starts line 1 and the call arguments wrap inside `(...)`.
  - Every `| <operator>` starts on a new line with a leading `| ` (one space after `|`).
  - Top-level pipelines keep `from` and `|` at column 0.
  - Inside a tabular `let` (`let name = from ...`), `from <source>` stays on the `let` line and
    each `| <operator>` is indented 4 spaces. A source-only tabular `let` (`let raw = from 'a.csv';`)
    stays on one line.
  - Inside a parenthesized pipeline `(from ... | ...)` that has one or more operators, break after
    `(` and before `)`, indenting `from` and all `|` stages by +4 spaces. A bare parenthesized
    source `(from orders)` stays on one line.
  - Hyphenated operator names (`project-away`, `project-rename`) have no spaces around `-`.
- **Spacing:**
  - Single space around binary operators (`+`, `-`, `*`, `/`, `%`, `==`, `!=`, `<`, `<=`, `>`,
    `>=`, `=~`, `!~`, `and`, `or`, `in`, `!in`, `in~`, `!in~`, `between`, `!between`, `has`,
    `!has`, `contains`, `startswith`, `endswith`, `_cs` variants, `matches regex`), around `..` in
    `between (lo .. hi)`, and around `=` everywhere (`let`, column aliases, `project-rename`,
    `join kind = ...`, named function arguments, `render ... with (...)` properties).
  - No space after unary `-` (`-x`, `-(a + b)`). Keep a space between binary `-` and unary `-`
    (`a - -b`).
  - Prefix `not`: single space before a non-parenthesized operand (`not active`, `not a == b`); no
    space before `(` in the call-like form (`not(active)`), which binds like a call (D-029).
  - No space before `,` or `;`; single space after `,` on single-line lists.
  - No space inside `(...)` or `[...]`, between a function name and `(`, around `.` in member
    access (`$left.a`, `record.field`), or before `[` in indexing (`items[0]`).
  - Single space before `(` in `in (...)` / `!in (...)` / `in~ (...)` / `!in~ (...)` and
    `between (...)` / `!between (...)`.
- **Wrapping lists & clauses (no trailing commas):**
  - DataScript rejects trailing commas (`DS0127`), so wrapped lists **never** emit a trailing comma.
  - List-taking operators (`project`, `extend`, `project-away`, `project-rename`, `distinct`,
    `sort by`, `union`, `join ... on`) stay on one line when the full line fits within 100 columns
    and contains no comments or multi-line children; otherwise break after the operator keyword
    (`| project`, `| sort by`, `| union`, `) on`, etc.) with one item per line indented +4 spaces.
    When `union` has a single parenthesized pipeline, `(` stays on the `| union (` line.
  - `summarize`: stays on one line if it fits; when wrapped, aggregates sit at +4 indent, `by` sits
    on its own line at +2 indent (aligned with `summarize`), and `by` keys stay on the `by` line if
    short or wrap to +4 indent if long (or if they contain comments or multi-line expressions).
  - `render <kind> with (...)`: stays on one line if it fits; when wrapped, each property sits on
    its own line at +4 indent with `)` aligned with `|`. Multi-value properties such as
    `ycolumns = a, b` stay on the property's line unless they exceed 100 columns.
  - Function calls and `in (...)` lists wrap after `(` with one item per line at +4 indent and `)`
    at the outer indent when they exceed 100 columns or contain comments/multi-line arguments.
  - `case(...)`: stays on one line if it has at most 2 condition/value pairs, contains no comments,
    and fits within 100 columns; otherwise wraps after `(` with each `condition, value,` pair on
    its own line at +4 indent and the final `else` value on its own line.
  - Long `where` boolean chains (`and` / `or`) exceeding 100 columns wrap before `and` / `or` at
    +4 indent.
- **Semicolons & blank lines:**
  - Separate statements with `;` attached to the end of the preceding statement (no space before `;`).
  - Every `let` statement ends with `;` (including in a `let`-only script). Omit a redundant
    trailing `;` on the final query statement of a file.
  - Consecutive single-line `let` statements stay adjacent (or preserve a single intentional blank
    line if the author grouped them). Separate multi-line tabular `let`s and query statements from
    adjacent statements with one blank line.
  - Collapse 2+ consecutive blank lines anywhere to a single blank line. Remove blank lines inside
    a pipeline unless a blank line precedes a standalone comment introducing a new section of the
    pipeline.
- **Literals, identifiers & keywords preserved verbatim:**
  - Never rewrite string quote style (`'...'` vs `"..."`), verbatim strings (`@"..."`, `@'...'`),
    escape sequences, number spellings (`1_000_000`, `.5`, `1e-3`), timespan/datetime literal text,
    quoted identifier quoting (`['order id']`, `["it's"]`), or operator aliases (`sort` vs `order`,
    `take` vs `limit`, explicit `kind = inner`).
  - DataScript strings are single-line (`DS0002` on unescaped newlines); multiline `/* ... */`
    block comments preserve their interior text and indentation verbatim.
- **Comments:**
  - Preserve every `//` line comment and `/* ... */` block comment in place relative to its
    surrounding tokens.
  - Standalone comments before a pipeline stage stay directly above the following `| <operator>` at
    the same indentation level.
  - Trailing end-of-line comments stay on their line separated by a single space (` // ...`).
  - Comments inside a list, `case(...)`, `summarize`, or `with (...)` force the enclosing construct
    to wrap and remain attached to their item or branch.
- **Syntax errors:** If parsing produces any lexical or syntax error (`DS0xxx`), refuse to format,
  report the diagnostics, and leave the file untouched.

## Examples (before and after)

### 1. Compact single-line pipeline & messy whitespace

Before:
```ds
from   orders|where   amount>100   and  status=='paid'|project orderId ,customerId,amount|take   10
```

After:
```ds
from orders
| where amount > 100 and status == 'paid'
| project orderId, customerId, amount
| take 10
```

Keeping `from <source>` on line 1 even when the input split it across lines (M1-24):

Before:
```ds
from
    orders
  |  where   amount > 100
    |  take  10
```

After:
```ds
from orders
| where amount > 100
| take 10
```

### 2. File sources, globs, and reader function calls

Before:
```ds
from   'orders.csv'|where amount>0|count;
from 'events/*.ndjson' | where level == "error" | take 20
```

After:
```ds
from 'orders.csv'
| where amount > 0
| count;

from 'events/*.ndjson'
| where level == "error"
| take 20
```

Reader calls (short stays on line 1; long options list wraps inside `read_csv(...)`):

Before:
```ds
from read_csv('orders.csv',delimiter=';',header=true)|take 5;
from   read_parquet( 'a.parquet' )  |  getschema;
from read_csv('data/2024/quarterly_orders.tsv', delimiter='\t', header=true, null_value='NULL', infer_rows=5000, compression='gzip') | where amount > 0
```

After:
```ds
from read_csv('orders.csv', delimiter = ';', header = true)
| take 5;

from read_parquet('a.parquet')
| getschema;

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

Before:
```ds
let  minAmount=100;
let   cutoff = ago(30d) ;
let raw=from 'orders.parquet';
let topCustomers=from raw|where timestamp>=cutoff and amount>minAmount|summarize revenue=sum(amount) by customerId|top 10 by revenue desc;
from raw|join kind=inner topCustomers on customerId|take 50
```

After:
```ds
let minAmount = 100;
let cutoff = ago(30d);
let raw = from 'orders.parquet';

let topCustomers = from raw
    | where timestamp >= cutoff and amount > minAmount
    | summarize revenue = sum(amount) by customerId
    | top 10 by revenue desc;

from raw
| join kind = inner topCustomers on customerId
| take 50
```

### 4. Multiple queries in a script

Separates queries with a blank line, preserves `;` between statements, and drops a redundant
trailing `;` after the final query:

Before:
```ds
from 'orders.csv' | summarize total = sum(amount) by region; from 'orders.csv' | where status == "refunded" | count;
```

After:
```ds
from 'orders.csv'
| summarize total = sum(amount) by region;

from 'orders.csv'
| where status == "refunded"
| count
```

In a `let`-only script, every `let` statement keeps a trailing `;`:

Before:
```ds
let taxRate=0.08;
let   recent = from 'orders.csv' | where timestamp >= ago(7d)
```

After:
```ds
let taxRate = 0.08;

let recent = from 'orders.csv'
    | where timestamp >= ago(7d);
```

### 5. Expression spacing & tricky operators

Unary vs binary minus, `not` vs `not(...)`, string operators, `in` / `between`, nested calls,
member access, and indexing:

Before:
```ds
from events
| where not   archived and not ( status== "deleted" ) and delta == - 1 and diff == a--b and total == -(x+y)
| where msg contains 'timeout' and path !startswith_cs '/health' and log has 'error' and tag =~ 'prod' and env !~ 'dev' and uri matches   regex '^/api/v[0-9]+'
| where code in(200,201,204) and region !in~ ( 'test' , 'staging' ) and latency between(10ms..500ms) and score !between ( -10 .. 10 )
| extend label = strcat( toupper( meta.env ) , ':' , items[ 0 ] )
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
| extend label = strcat(toupper(meta.env), ':', items[0])
```

### 6. Long `project`, `extend`, `project-rename`, and `project-away` lists

Short lists stay on one line:

Before:
```ds
from orders | project orderId , customerId , amount | extend net=amount-discount | project-rename id=orderId | project-away tmp* , debug_flag
```

After:
```ds
from orders
| project orderId, customerId, amount
| extend net = amount - discount
| project-rename id = orderId
| project-away tmp*, debug_flag
```

Lists exceeding 100 columns wrap with one item per line, 4-space indent, and no trailing comma:

Before:
```ds
from orders
| extend grossAmount = unitPrice * qty, discountAmount = round(unitPrice * qty * discountRate, 2), netAmount = unitPrice * qty * (1.0 - discountRate)
| project orderId, customerId, orderMonth = startofmonth(timestamp), grossAmount, discountAmount, netAmount
| project-rename order_id = orderId, customer_id = customerId, order_month = orderMonth, net_amount = netAmount
| project-away internal_*, debug_*, raw_payload, temp_notes, migration_flag, audit_trace_id, legacy_code
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
```

### 7. `summarize` with aggregates and `by` keys

Short single-line forms (aggregates + `by`, aggregates only, and `by` only):

Before:
```ds
from orders | summarize orders=count(),revenue=sum(amount) by customerId;
from orders | summarize totalOrders=count(), totalRevenue=sum(amount);
from orders | summarize   by   customerId , region
```

After:
```ds
from orders
| summarize orders = count(), revenue = sum(amount) by customerId;

from orders
| summarize totalOrders = count(), totalRevenue = sum(amount);

from orders
| summarize by customerId, region
```

Long `summarize` wraps aggregates at +4 indent, puts `by` at +2 indent (aligned with `summarize`),
and wraps `by` keys at +4 indent when they exceed 100 columns (or keeps short `by` keys on the `by`
line):

Before:
```ds
from orders
| summarize orderCount = count(), paidCount = countif(status == "paid"), totalRevenue = sum(amount), avgOrder = avg(amount), p95Order = percentile(amount, 95) by region, country, customerSegment, salesChannel, paymentMethod, day = bin(timestamp, 1d);
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
    day = bin(timestamp, 1d);

from orders
| summarize
    orderCount = count(),
    totalRevenue = sum(amount),
    avgOrder = avg(amount),
    maxOrder = max(amount)
  by region, bin(timestamp, 1d)
```

### 8. `sort by` / `order by` and `top N by`

Short multi-key `sort by`, `order by` (preserved verbatim), and `top N by`:

Before:
```ds
from orders | sort  by  region  asc , amount  desc  nulls  last;
from orders | order by timestamp desc nulls first | top  10  by  amount  desc  nulls  last
```

After:
```ds
from orders
| sort by region asc, amount desc nulls last;

from orders
| order by timestamp desc nulls first
| top 10 by amount desc nulls last
```

Long `sort by` list wraps one key per line:

Before:
```ds
from orders
| sort by region asc nulls last, customerSegment asc, orderMonth desc, totalRevenue desc nulls last, customerId asc
```

After:
```ds
from orders
| sort by
    region asc nulls last,
    customerSegment asc,
    orderMonth desc,
    totalRevenue desc nulls last,
    customerId asc
```

### 9. `case(...)` expressions

Short inline `case(...)` (≤ 2 branches, fits within 100 columns):

Before:
```ds
from orders | extend size=case( amount>=500 , "large" , "small" )
```

After:
```ds
from orders
| extend size = case(amount >= 500, "large", "small")
```

Multi-branch `case(...)` (3+ branches or > 100 columns) wraps one `condition, value` pair per line
followed by the final `else` value:

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
operator also wraps:

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

Subquery inside `(...)` with `kind = leftouter` and multi-key `$left` / `$right` conditions:

Before:
```ds
from orders | join kind=leftouter (from customers | where active | project id, name, region) on $left.customerId==$right.id, $left.region==$right.region
```

After:
```ds
from orders
| join kind = leftouter (
    from customers
    | where active
    | project id, name, region
) on $left.customerId == $right.id, $left.region == $right.region
```

When the `on` clause exceeds 100 columns, its conditions wrap at +4 indent:

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

### 11. `union`

Inline table sources stay on one line:

Before:
```ds
from 'orders_2023.parquet'|union  'orders_2024.parquet' , 'orders_2025.parquet'
```

After:
```ds
from 'orders_2023.parquet'
| union 'orders_2024.parquet', 'orders_2025.parquet'
```

A single parenthesized pipeline hugs `| union (`; multiple targets with any multi-line pipeline wrap
one target per line:

Before:
```ds
from 'orders_2024.parquet' | where amount > 0 | union (from 'orders_2025.parquet' | where amount > 0);
from 'orders_2023.parquet' | where amount > 0 | union 'orders_2024.parquet', (from 'orders_2025.parquet' | where amount > 0 | project-away notes)
```

After:
```ds
from 'orders_2024.parquet'
| where amount > 0
| union (
    from 'orders_2025.parquet'
    | where amount > 0
);

from 'orders_2023.parquet'
| where amount > 0
| union
    'orders_2024.parquet',
    (
        from 'orders_2025.parquet'
        | where amount > 0
        | project-away notes
    )
```

### 12. `render` operator

Short `render` and short `render ... with (...)`:

Before:
```ds
from orders | summarize revenue = sum(amount) by region | render   barchart;
from orders | summarize revenue = sum(amount) by region | render barchart with(title="Revenue by Region")
```

After:
```ds
from orders
| summarize revenue = sum(amount) by region
| render barchart;

from orders
| summarize revenue = sum(amount) by region
| render barchart with (title = "Revenue by Region")
```

Long `render ... with (...)` wraps one property per line, keeping multi-column values together on
their property line:

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
spellings, timespans, datetimes, typed nulls, quoted identifiers (`['...']`, `["..."]`), and
multiline `/* ... */` block comment interiors are kept verbatim. (DataScript strings are always
single-line; an unescaped newline inside a string is `DS0002`.)

Before:
```ds
/*  Quarterly report
      keeps exact
    internal indentation  */
let winPath = @"C:\data\orders.csv";
let rawTag = @'a\b\c';
from   'orders.csv'
| where ['order id'] != "" and amount > 1_000_000 and rate >= .5 and eps == 1e-3
| where timestamp >= datetime(2024-01-31T12:00:00Z) and wait < 1.5h and jitter <= 5µs and window == timespan(1.02:03:04) and offset == timespan(-90m)
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
| where timestamp >= datetime(2024-01-31T12:00:00Z)
    and wait < 1.5h
    and jitter <= 5µs
    and window == timespan(1.02:03:04)
    and offset == timespan(-90m)
| extend note = "line1\nline2", ["it's"] = long(null), ['let'] = string(null)
| project ['order id'], ["it's"], ['let'], note
```

### 14. Comments & blank lines

#### File-leading comments and section comments

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
from orders
| where timestamp >= cutoff
| summarize revenue = sum(amount) by region
```

#### Comments between pipeline stages

Standalone comments attach above the following `| operator`. An intentional blank line before a
section comment inside a pipeline is preserved:

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

Before:
```ds
let cutoff=ago(30d);    // rolling window
from 'orders.csv'   // raw export
| where amount > 0     // drop refunds
| summarize revenue = sum(amount) by customerId   // per customer
| top 10 by revenue desc // leaderboard
```

After:
```ds
let cutoff = ago(30d); // rolling window

from 'orders.csv' // raw export
| where amount > 0 // drop refunds
| summarize revenue = sum(amount) by customerId // per customer
| top 10 by revenue desc // leaderboard
```

#### Comments inside multi-line `project`, `summarize`, `join (...)`, and `case(...)`

Any comment inside a list or `case(...)` forces multi-line layout and stays with its item:

Before:
```ds
from orders
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

3+ blank lines collapse to 1; stray blank lines without comments inside a pipeline are removed,
while intentional single blank lines between statements are preserved:

Before:
```ds
let a = 1;



let b = 2;



from orders

| where amount > a


| where amount > b

| take 10
```

After:
```ds
let a = 1;

let b = 2;

from orders
| where amount > a
| where amount > b
| take 10
```

### 15. Edge cases & non-changes

#### Already-formatted query (idempotency)

Formatting an already-formatted script produces byte-for-byte identical output:

Before (and After — unchanged):
```ds
// Top customers over time
let orders = 'orders.parquet';

let topCustomers = from orders
    | summarize revenue = sum(amount) by customerId
    | top 10 by revenue desc;

from orders
| join kind = inner topCustomers on customerId
| summarize revenue = sum(amount) by customerId, month = startofmonth(timestamp)
| render timechart with (title = "Top Customers")
```

#### Syntax error — formatter refuses to modify invalid input

Input with syntax errors (here: `=` instead of `==` (`DS0110`) and a trailing comma (`DS0127`)):

```ds
from 'orders.csv' | where amount = 100 | project orderId,
```

`ds fmt` reports the syntax diagnostics, exits non-zero, and leaves the file untouched. Running
`ds fmt --check` on the same file also reports the syntax diagnostics rather than a formatting diff.

## Follow-ups
