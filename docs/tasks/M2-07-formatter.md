# M2-07 — Formatter (`ds fmt`)

**Status:** todo
**Depends on:** M1-13
**Crates:** `datascript-syntax`, `datascript-cli`

## Goal
An opinionated formatter over the CST: keep short queries on one line when they fit comfortably,
expand longer pipelines to one operator per line with leading `|`, normalize spacing with 2-space
indentation, and preserve all comments.

## Acceptance criteria
- [ ] `ds fmt [--check] <files>`.
- [ ] Idempotent and comment-preserving over the whole golden corpus (an automated test).
- [ ] Refuses to format files with syntax errors, saying why.

## Carried over
- [ ] (from planning, 2026-11-22) If M3-03 is done, add `textDocument/formatting` to the language server (M3-03 left it for after this task), with an LSP test.
- [ ] (from M1-24) The formatter keeps `from <source>` on the first line of a pipeline, with operators on the following lines: `from orders⏎| where …`, never `from⏎orders`.

## Formatting rules

- **Line width & indentation:** 100-column soft target for keeping statements on one line and
  wrapping long constructs; **2 spaces** per indentation level (spaces, never tabs); LF (`\n`) line
  endings; no trailing spaces; a single trailing newline at EOF (whitespace-only files format to
  empty).
- **Pipelines & leading `|`:**
  - **Single-line when it fits:** If an entire query statement (including its `;` when another
    statement follows) fits within 100 columns, contains no comments, and contains no multi-line
    sub-expressions (such as a wrapped `case(...)`, `read_csv(...)`, `render ... with (...)`, or
    wrapped subquery), format it on one line: `from 'orders.csv' | where amount > 10 | take 5`.
  - **Multi-line otherwise:** Break to one `| <operator>` per line with a leading `| ` (one space
    after `|`), keeping `from <source>` on the first line — never break between `from` and
    `<source>` (M1-24). If `<source>` is a long reader call (`read_csv(...)`), `from read_csv(`
    starts line 1 and its arguments wrap inside `(...)`.
  - Top-level multi-line pipelines keep `from` and `|` at column 0.
  - **Tabular `let`:** If the entire `let name = from ...;` statement fits within 100 columns (with
    no comments or multi-line sub-expressions), keep it on one line. Otherwise keep
    `let name = from <source>` on the first line and put each `| <operator>` on its own line
    indented 2 spaces.
  - **Parenthesized pipelines:** Inside `join`, `union`, or `(from ... | ...)`, keep the
    parenthesized pipeline inline if its enclosing line fits within 100 columns (and it has no
    comments or multi-line inner expressions). Otherwise break after `(` and before `)` and format
    the inner pipeline multi-line (`from` and each `|` stage indented +2 spaces).
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
    (`| project`, `| sort by`, `| union`, `) on`, etc.) with one item per line indented +2 spaces.
    When `union` has a single parenthesized pipeline that wraps, `(` stays on the `| union (` line.
  - `summarize`: stays on one line if it fits; when wrapped, aggregates sit at +2 indent, `by` sits
    on its own line at +2 indent (aligned with `summarize`), and `by` keys stay on the `by` line if
    `  by ...` fits within 100 columns (with no comments or multi-line expressions) or wrap to +4
    indent (+2 under `by`) otherwise.
  - `render <kind> with (...)`: stays on one line if it fits; when wrapped, each property sits on
    its own line at +2 indent with `)` aligned with `|`. Multi-value properties such as
    `ycolumns = a, b` stay on the property's line unless they exceed 100 columns.
  - Function calls and `in (...)` lists wrap after `(` with one item per line at +2 indent and `)`
    at the outer indent when they exceed 100 columns or contain comments/multi-line arguments.
  - `case(...)`: stays on one line if it has at most 2 condition/value pairs, contains no comments,
    and fits within 100 columns; otherwise wraps after `(` with each `condition, value,` pair on
    its own line at +2 indent and the final `else` value on its own line.
  - Long `where` boolean chains (`and` / `or`) exceeding 100 columns wrap before `and` / `or` at
    +2 indent.
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
  - Any comment inside a pipeline forces the pipeline to format multi-line.
  - Standalone comments before a pipeline stage stay directly above the following `| <operator>` at
    the same indentation level.
  - Trailing end-of-line comments stay on their line separated by a single space (` // ...`).
  - Comments inside a list, `case(...)`, `summarize`, or `with (...)` force the enclosing construct
    to wrap and remain attached to their item or branch.
- **Syntax errors:** If parsing produces any lexical or syntax error (`DS0xxx`), refuse to format,
  report the diagnostics, and leave the file untouched.

## Examples (before and after)

### 1. Short single-line queries vs multi-line pipelines

Short queries that fit within 100 columns stay on one line — including queries that were pointlessly
split across lines:

Before:
```ds
from   'orders.csv'|where   amount>10  |  take   5;
from
  orders
  | count
```

After:
```ds
from 'orders.csv' | where amount > 10 | take 5;

from orders | count
```

Once a pipeline exceeds 100 columns, it expands to one operator per line with leading `|`, keeping
`from <source>` on line 1 even if the input broke after `from` (M1-24):

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

Short file-source queries stay on one line; longer glob pipelines expand:

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

Reader calls (short queries stay on one line; long options wrap inside `read_csv(...)`):

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

Short tabular `let`s stay on one line; longer tabular `let`s put `|` stages on indented lines
(2 spaces):

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

Separates queries with a blank line, preserves `;` between statements, keeps short queries on one
line, and drops a redundant trailing `;` after the final query:

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
| sort by amount desc
```

In a `let`-only script, every `let` statement keeps a trailing `;`:

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
  | project orderId, customerId, amount;
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

In a multi-line pipeline, short lists stay on their operator's line:

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

Lists exceeding 100 columns wrap with one item per line, 2-space indent, and no trailing comma:

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

Short single-line queries (aggregates + `by`, aggregates only, and `by` only) stay on one line:

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

Long `summarize` wraps aggregates at +2 indent, aligns `by` under `summarize` at +2 indent, and
wraps `by` keys at +4 indent when `  by ...` exceeds 100 columns (or keeps short `by` keys on the
`by` line):

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

### 8. `sort by` / `order by` and `top N by`

Short multi-key `sort by`, `order by` (preserved verbatim), and `top N by` stay on one line:

Before:
```ds
from orders | sort  by  region  asc , amount  desc  nulls  last;
from orders | order by timestamp desc nulls first | top  10  by  amount  desc  nulls  last
```

After:
```ds
from orders | sort by region asc, amount desc nulls last;

from orders | order by timestamp desc nulls first | top 10 by amount desc nulls last
```

Long `sort by` list wraps one key per line at +2 indent:

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
at +2 indent, followed by the final `else` value:

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

Short `join` with an inline subquery fits on one line; a longer `join` with `kind = leftouter` and
multi-key `$left` / `$right` conditions expands the subquery with 2-space indent:

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

### 11. `union`

Short inline table `union` stays on one line:

Before:
```ds
from 'orders_2023.parquet'|union  'orders_2024.parquet' , 'orders_2025.parquet'
```

After:
```ds
from 'orders_2023.parquet' | union 'orders_2024.parquet', 'orders_2025.parquet'
```

When a `union` stage exceeds 100 columns, a single parenthesized pipeline hugs `| union (` and
multiple targets wrap one per line with 2-space indent:

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

### 12. `render` operator

Short `render` queries stay on one line; once the query exceeds 100 columns it breaks per operator,
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

A section comment above a short query does not force that query onto multiple lines:

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

Any comment inside a list, subquery, or `case(...)` forces multi-line layout (2-space indent) and
stays with its item:

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

3+ blank lines collapse to 1; stray blank lines without comments inside a multi-line pipeline are
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

### 15. Edge cases & non-changes

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

Input with syntax errors (here: `=` instead of `==` (`DS0110`) and a trailing comma (`DS0127`)):

```ds
from 'orders.csv' | where amount = 100 | project orderId,
```

`ds fmt` reports the syntax diagnostics, exits non-zero, and leaves the file untouched. Running
`ds fmt --check` on the same file also reports the syntax diagnostics rather than a formatting diff.

## Follow-ups
