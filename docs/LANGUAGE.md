# DataScript language (draft specification)

Status: **draft**. This is the contract that parser, binder, and tests are written against.
Change it deliberately, in the same change as the code, and log notable decisions in
[DECISIONS.md](DECISIONS.md).

DataScript is *KQL-like, free to diverge*. It keeps KQL's feel, operator names, and function names.
It departs from KQL where KQL is surprising or where a change gives clearer errors. Each divergence
is marked **≠ KQL**. Where this document and the function catalog are silent, behaviour follows KQL
(D-014).

## 1. Lexical structure

- **Encoding**: UTF-8. Identifiers and keywords are **case-sensitive** (as in KQL).
- **Comments**: `// to end of line`. (≠ KQL: block comments `/* */` are also allowed. They do not
  nest. An unterminated block comment is `DS0006`.)
- **Whitespace** is insignificant except as a separator. Newlines carry no meaning.
- **Identifiers**: `[A-Za-z_][A-Za-z0-9_]*`. Names with other characters, or that clash with a
  reserved word, are quoted: `['order id']` or `["order id"]`.
- **Punctuation and operators**: `|` `;` `,` `(` `)` `[` `]` `{` `}` `.` `..` `=` `==` `!=` `<`
  `<=` `>` `>=` `=~` `!~` `+` `-` `*` `/` `%` `:` `!` `~`. `$left` and `$right` are single tokens
  (used in `join ... on`). `<>`, `&&` and `||` are lexed as tokens only so that the parser can
  suggest `!=`, `and` and `or` (`DS0107`, `DS0111`).
- **Negated and case-insensitive operator forms**: `!` directly followed (no trivia) by one of
  `in`, `between`, `has`, `contains`, `startswith`, `endswith` or their `_cs` variants is the
  negated operator (`!has`, `!in`). `in~` and `!in~` are `in`/`!in` directly followed by `~`. The
  lexer reads the whole identifier after `!` before deciding, so `!has_error` is `!` followed by
  the name `has_error`. A `!` that does not start one of these forms (`!x`, `!has_error`) is `DS0111`
  with a suggestion to use `not(...)`.
- **Unexpected characters** (`#`, `` ` ``, `?`, `\` outside a string, invisible characters such as
  U+200B) are `DS0001`. A word with non-ASCII letters (`naïve`, also when the accent is a
  combining mark after the letter) is one `DS0001` suggesting `['naïve']`; `` `name` `` is one `DS0001` suggesting `['name']`. Unicode white space (no-break
  space, BOM) counts as white space.
- **Literals**

| Kind | Examples | Notes |
|---|---|---|
| long | `42`, `1_000_000` | 64-bit signed. There are no negative literals: `-7` is unary minus applied to `7` (the binder folds it, so `-9223372036854775808` is accepted) |
| real | `3.14`, `1e-3`, `.5` | 64-bit float. A real needs a digit after the point (`1.` is not a real), and a `.` followed by another `.` never belongs to a number: `1..10` is `1` `..` `10` |
| string | `"text"`, `'text'`, `@"C:\raw"` | Escapes in normal strings: `\\` `\"` `\'` `\n` `\r` `\t` `\0` `\u{1F600}`. Any other `\x` is `DS0005`, with help suggesting `@"..."` or `/` when the string looks like a Windows path (it contains `:\`, starts with `\\` and a name, or has several bad escapes). `@` = verbatim (no escapes) |
| bool | `true`, `false` | |
| null | `null` | ≠ KQL: an untyped `null` literal, coerced by context |
| timespan | `1d`, `2h`, `30m`, `10s`, `250ms`, `1.5h`, `timespan(1.02:03:04)` | Units: `d`, `h`, `m` (minutes), `s`, `ms`, `us`/`µs`, `tick` (100 ns, truncated to µs). `µs` may use the micro sign (U+00B5) or the Greek mu (U+03BC) |
| datetime | `datetime(2024-01-31)`, `datetime(2024-01-31T12:00:00Z)` | ISO 8601. No offset means UTC |
| dynamic | `dynamic([1, 2])`, `dynamic({"a": 1})` | M2 |
| typed null | `long(null)`, `string(null)`, `datetime(null)` | Any type name from §5 |

**Literal forms that look like calls.** `datetime(...)`, `timespan(...)`, `dynamic(...)` and the
typed forms `<type>(null)` are literals, not function calls, so their contents are not scalar
expressions (`datetime(2024-01-31)` is not a subtraction). The type names stay unreserved
(§1.1); the rules apply only when the name is directly followed by `(`:

- `datetime(` and `timespan(`: the lexer reads everything up to the next `)` (or the end of the
  line, which is `DS0003`) as **one literal token** and checks it (`DS0003`). A body of `null`
  makes a typed null. White space around the body is allowed.
  - `datetime` bodies: `YYYY-MM-DD`, optionally followed by `T` or a space, `hh:mm`, optional
    `:ss` and a fraction (truncated to µs), and an optional `Z` or `±hh:mm` offset. Month, day,
    hour, minute and second may have one or two digits.
  - `timespan` bodies: `[-][d.]hh:mm[:ss[.fffffff]]` (hours 0–23, minutes and seconds 0–59), or a
    short-form timespan such as `90m`, which may also take a leading `-` (`timespan(-90m)`).
- `<type>(null)` for the other type names (`bool`, `int`, `long`, `real`, `decimal`, `string`,
  `guid`, `dynamic`): the parser recognizes the type name, `(`, `null`, `)` sequence as a typed-null
  literal.
- `dynamic(...)` (M2): the parser reads a JSON-like literal body (arrays, objects, scalar literals).

**Lexical details** (D-028, D-034):

- `_` in a number must sit between two digits (`1_000`, not `1_` or `1__0`).
- A timespan is a number directly followed by a unit; the number cannot have an exponent (`1e3s`
  is `DS0003`). An unknown unit (`1x`, `10min`) is `DS0003` listing the valid units. Fraction
  digits past nanosecond precision are ignored, so `0.0000000001s` is zero.
- There are no hexadecimal literals: `0x1F` is `DS0003` with help giving the decimal value.
- Strings, including verbatim ones, end at the end of their line. An unterminated string
  (`DS0002`) covers the opening quote to the end of the line, and the next line lexes normally.
- `\u{...}` takes 1 to 6 hex digits naming a Unicode scalar value; anything else is `DS0005`.
- A quoted name `['x']` / `["x"]` is one token only when there is no trivia between the brackets
  and the string. `[ 'x' ]` and `['a', 'b']` are a bracket, strings and a bracket. An empty
  quoted name (`['']`) is `DS0007`.

### 1.1 Keywords

Keywords are **contextual** (D-013). Only these words are reserved and must be quoted to be used as
names: `let`, `true`, `false`, `null`, `and`, `or`, `not`.

Every other keyword (`by`, `on`, `kind`, `with`, `asc`, `desc`, `nulls`, `first`, `last`, `in`,
`between`, `has`, `contains`, `from`, `to`, `step`, and all operator names such as `where`, `count`,
`order`) is an ordinary identifier except where the grammar expects it. So a column named `count`
or `order` can be used directly: `| where order > 3`, `| summarize sum(count) by kind`.

`from` is a keyword at the head of a pipeline when something that can start an expression follows
it, other than `-` and `not`; everywhere else it is a name (D-036). So `| where from > 3` and
`let y = from - 1` use a column or `let` named `from`. `from(` is the keyword followed by a
parenthesized source, never a call. A table named `from` is read with `from ['from']`, or with
`from from` (the second `from` is a name). The match is case-sensitive: `FROM` is not the keyword.

Operator names are recognized only when the previous non-trivia token is `|`. The lexer is
context-free (D-026): `project-away` lexes as `project`, `-`, `away`. Right after `|`, the parser
joins a hyphenated operator name (`project-away`, `project-rename`, `project-reorder`, `mv-expand`,
`make-series`) into one operator when its three tokens are **adjacent, with no trivia between
them**. So `a - b` and `project-away` never conflict, and `| project-away x` works with any
whitespace or comments between `|` and the operator. `| project - away` is not an operator name;
`DS0104` suggests `project-away`.

## 2. Program structure

A program is a sequence of **statements** separated by `;`. A trailing `;` is optional.

```ds
let threshold = 100;                                      // scalar let
let big = from 'orders.csv' | where amount > threshold;   // tabular let
from big | summarize count() by customerId                // query statement (produces a result)
```

- `let name = <expr>` binds a scalar or tabular expression. Lets are immutable and lexically
  scoped to the rest of the program. Re-binding a name is an error (`DS1003`) (≠ KQL, which allows
  shadowing).
- **Scalar or tabular?** The syntax decides (D-036). A `let` whose value *is* a pipeline (it
  starts with `from` or a generator, or is a pipeline in parentheses) is tabular. Everything else,
  including a `let` bound to a string or to a bare name, is scalar: `let alias = orders;` does not
  make `alias` a table even if `orders` is one; write `let alias = from orders;`. "Is", not "starts
  with": `let x = (from t | count) + 1` is a scalar expression, which the binder rejects.
  A **constant string** in a table position (after `from`, right side of `join`/`union`/`lookup`)
  is a file source, whether it is written as a literal or as the name of a scalar `let` bound to
  one (D-020). So one name can serve as a path and as a string:

  ```ds
  let path = 'data/orders.csv';
  from path | take 5;                              // source
  from read_csv(path, delimiter = ';') | count     // reader argument
  ```
- Each **query statement** (a statement that is not a `let`) produces one result. A program may
  contain several. They are shown in order (≠ KQL's multi-result behaviour is engine-specific).
  Each statement runs as its own query. A tabular `let` used by two statements is evaluated in
  each (no materialization).
- A missing `;` between statements is the most common mistake for KQL users. When a pipeline is
  followed by something that can only start a new statement (`let`, or `from` on a new line),
  the parser reports `DS0106` with a machine-applicable fix inserting `;`. When the pipeline
  ends in an operator that takes a list (`project a⏎  b`), a forgotten `,` is as likely, so both
  fixes are offered and neither is machine-applicable.
- A program with no query statement is valid for `ds check`. `ds run` warns that there is nothing
  to run (`DS0004`).
- `now()` has one value for the whole run of a program (D-015).

## 3. Tabular expressions

```
tabular_expr := 'from' source ( '|' operator )*
              | generator ( '|' operator )*
              | '(' tabular_expr ')' ( '|' operator )*
```

Every pipeline starts with `from` (D-036): a query statement, the value of a tabular `let`, a
pipeline in parentheses and a pipeline passed as a call argument.

```ds
let big = from 'orders.csv' | where amount > 100;
from big | summarize count() by customerId;
from read_csv('x.txt', delimiter = ';') | take 5;
(from big | take 5) | union (from 'more.csv' | take 5)
```

Three forms start a pipeline without `from`:

- A **row generator** reads nothing, so it starts a query on its own: `range`, `datatable` and
  `print` (M2).
- A **parenthesized pipeline** already starts with `from`: `(from t | where a > 1) | take 5`.
  `from (from t | where a > 1)` is also valid.
- The **right side of `join`, `union` and `lookup`** is already a table position, so it takes a
  table directly: `join customers on id`, `union 'b.csv'`. A pipeline there is parenthesized:
  `join (from 'c.csv' | where active) on id`.

A pipeline without `from` (`orders | take 5`) is `DS0150`, with a fix that inserts `from` (≠ KQL).
`from` with nothing after it is `DS0151`.

### 3.1 Sources

| Source | Example | Notes |
|---|---|---|
| File path | `from 'data/orders.csv'`, or a scalar `let` bound to one | Format from extension (see below) |
| Glob | `from 'logs/2024-*/*.parquet'` | `*`, `?`, `[abc]`, and `**` (any depth). Matches are sorted by path. All matches must share a format. Schemas are merged by name |
| Reader function | `from read_csv('x.txt', delimiter = ';', header = false)` | Explicit format and options. Also `read_parquet`, `read_json` |
| Name | `from orders` | A tabular `let`, or a table registered by the host (web UI drag-drop, `--table orders=path` on the CLI) |
| Parenthesized pipeline | `from (from orders \| take 5)` | |
| Inline data | `datatable(a: long, b: string) [1, "x", 2, "y"]` | M2. A generator: no `from` |
| Range | `range x from 1 to 10 step 1` | M2. A generator: no `from` |

DuckDB-style path sources are the main way to read files. A constant string after `from` (a
literal, or the name of a scalar `let` bound to one) is always a source, never a scalar (D-020).
A string that is not constant there (`from strcat(dir, '/x.csv') | take 1`) is `DS3006`. As
everywhere in a table position, the parser accepts any expression after `from` and the binder
rejects the ones that are not tables (D-029).

**Formats by extension**: `.csv`, `.tsv`, `.parquet`, `.json` (array or NDJSON, detected),
`.ndjson`/`.jsonl`. CSV, TSV and JSON files may also be compressed, with the compression taken from
a second suffix: `.gz`, `.bz2`, `.xz`, `.zst` (e.g. `'logs/*.csv.gz'`). Anything else is `DS3002`,
with help pointing at the reader functions.

**Path resolution** depends on the host:

| Host | Relative paths resolve against |
|---|---|
| `ds run <file>`, `ds check <file>` | the script's directory |
| `ds run -e`, `ds run -` (stdin), REPL | the current working directory |
| LSP / VS Code | the document's directory (untitled documents: the first workspace folder) |
| `ds ui [dir]` | the UI root directory. Paths outside it are rejected (`DS3007`, D-016) |
| WASM | the names files were registered under (`registerFile('logs/a.csv', …)`); globs match those names |

Absolute paths are allowed except where a host forbids them. `~` is not expanded (`DS3001` help
says so). `/` works as a separator on every platform; `\` must be escaped or written in a verbatim
string. URLs are not sources in M1; in WASM, URL contents are fetched by `registerFile`.

**Glob schema merging**: columns are matched by name, using each file's own schema. For CSV and
TSV that means each file's own header row: columns are never matched by position, so files whose
headers list the same columns in a different order read correctly. A column missing from some
files is null there. The same column with different types is widened by the rules in §4.2 (e.g. `long` and
`real` → `real`); if no widening exists, `DS3005` names the two files and the column.

Reader options (M1 set):

| Function | Options |
|---|---|
| `read_csv` | `delimiter`, `header` (bool), `quote`, `escape`, `null_value`, `schema` (M2), `infer_rows` (default 1000), `compression` |
| `read_parquet` | none in M1 |
| `read_json` | `format = "ndjson" \| "array"`, `compression` |

Reader options must be constant expressions (literals or scalar `let`s bound to literals), because
schemas are fetched before the program can be fully bound (ARCHITECTURE.md § Source resolution).
A non-constant option is `DS3006`.

### 3.2 Operators

M1 operators are marked ●, M2 operators ○.

| Operator | Syntax | Notes |
|---|---|---|
| ● `where` | `where <predicate>` | Predicate must be bool. `=` in a predicate gives a targeted error suggesting `==` |
| ● `project` | `project a, b, c = a + b` | Select, rename, and compute. Output in the order written |
| ● `project-away` | `project-away a, b` | Wildcards `col*` allowed |
| ● `project-rename` | `project-rename new = old` | Keeps column position |
| ● `extend` | `extend c = a * 2` | Adds columns at the end, or replaces a column in place. Unnamed expressions get `Column1`, … |
| ● `summarize` | `summarize [name =] agg(...) , ... [by key [, ...]]` | Keys may be expressions: `by bin(ts, 1h)`. Output: keys first, then aggregates. With no aggregates it returns the distinct keys |
| ● `sort` / `order` | `sort by a desc, b asc nulls last` | ≠ KQL: default direction is **asc** (D-005). Omitting the direction is warning `DS2103`, with a fix that inserts `asc`. See §3.4 |
| ● `top` | `top 10 by revenue desc` | Default desc, as in KQL (top-N is about largest) |
| ● `take` / `limit` | `take 100` | No ordering guarantee unless it follows `sort` |
| ● `count` | `count` | One row, column `Count` |
| ● `distinct` | `distinct a, b` or `distinct *` | |
| ● `join` | `join kind = inner (right) on key` or `on $left.a == $right.b` | Kinds: `inner` (default), `leftouter`, `rightouter`, `fullouter`, `leftsemi`, `leftanti`, `rightsemi`, `rightanti`. ≠ KQL: no `innerunique` (`DS0124`), and the default is `inner`. Unknown kinds are `DS0123`. Decision D-004 |
| ● `union` | `union t2, t3` | By column name, columns in order of first appearance. Missing columns become null. Type conflicts widen per §4.2 or give `DS2003` |
| ● `render` | `render timechart [with (title = "...")]` | Must be last (`DS0120`). See §6 |
| ● `getschema` | `getschema` | Returns the table `ColumnName: string, ColumnOrdinal: long, DataType: string` |
| ○ `lookup` | `lookup (dim) on key` | |
| ○ `mv-expand` | `mv-expand tags` | Needs `dynamic` |
| ○ `parse` | `parse msg with "user=" user " id=" id:long` | |
| ○ `make-series` | `make-series sum(x) on ts step 1h` | |
| ○ `sample` | `sample 100` | |
| ○ `as` | `as name` | Names a sub-result for later reference. Follows the no-shadowing rule (`DS1003`) |
| ○ `project-reorder` | `project-reorder c, a*` | |
| ○ `print` | `print x = 1, y = now()` | A source producing one row |

A right-hand table in `join`/`union`/`lookup` is a name, a path string, or a parenthesized tabular
expression, written without `from` (D-036): `join 'customers.csv' on customerId` works, and so
does `join (from 'customers.csv' | where active) on customerId`.

### 3.3 Column naming

- Output column names must be unique.
- `join ... on key` (a bare column name) is an equi-join on a column both sides share, and
  produces **one** `key` column, like SQL `USING` (D-019, ≠ KQL, which keeps both as `key` and
  `key1`). The key keeps the left column's position. For `rightouter` it takes the right side's
  value, and for `fullouter` it is `coalesce(left.key, right.key)`. The key columns must have
  compatible types (widened per §4.2), otherwise `DS2003`.
- `join ... on $left.a == $right.b` keeps both `a` and `b`.
- `join` output is the left columns, then the right columns (minus any `on key` columns). Other
  collisions are renamed `name1`, `name2` (as in KQL), and the binder emits an info diagnostic
  (`DS1007`) that says how to rename them explicitly. Semi and anti joins return one side only.
- Unnamed aggregates: `sum(amount)` → `sum_amount`; an aggregate over anything other than a single
  column → `sum_`; `count()` → `count_`. A second collision appends `1`, `2`, ….
- Unnamed `by` keys: a key that references exactly one column, like `bin(ts, 1h)`, takes that
  column's name (`ts`). Other expressions get `Column1`, ….
- Duplicate explicit names are `DS1005`.

### 3.4 Row order

- Only `sort` and `top` define row order. `where`, `project*`, `extend`, `take` and `render`
  preserve it. `summarize`, `distinct`, `join`, `union` and `count` do not.
- `sort` is **not guaranteed stable**: rows with equal keys come out in any order. Add tie-breaking
  keys when order matters (the test fixtures contain ties on purpose).
- Null placement default (as in KQL): `asc` puts nulls first, `desc` puts nulls last. `nulls first`
  / `nulls last` override it.

## 4. Scalar expressions

Precedence, from lowest to highest:

| Level | Operators |
|---|---|
| 1 | `or` |
| 2 | `and` |
| 3 | `not` (prefix, also `not(x)`) |
| 4 | `==` `!=` `<` `<=` `>` `>=` `=~` `!~` `in` `!in` `in~` `!in~` `between` `!between` and string operators |
| 5 | `+` `-` |
| 6 | `*` `/` `%` |
| 7 | unary `-` |
| 8 | call `f(x)`, member `a.b`, index `a[0]` (M2 for dynamic) |

Comparison operators do not chain: `a < b < c` is a syntax error (`DS0101`, one per chain)
suggesting `a < b and b < c`.

`not` followed by `(` reads like a call and binds like one (as in KQL): `not(x) == y` is
`(not x) == y`. Without parentheses, `not` is the level-3 prefix: `not x == y` is `not (x == y)`
(D-029). So a bare `not` cannot be the operand of a level 4–7 operator: `1 + not x` and
`a == not b` are errors (`DS0114`, as in Python) suggesting `not(...)` (D-033).

There are no methods: `a.f(x)` is an error (`DS0113`) suggesting `f(a, x)`, or the operator
form `a contains x` for the string operators (D-033). Expressions may nest at most 256 levels deep (`DS0100`); each operator in a chain such as
`a + b + c` or `a.b.c` counts as one level (D-032).

- `x in (1, 2, 3)`: the list is scalar literals or expressions of one type (after widening).
  `in~` compares strings case-insensitively. Tabular `in (T | project a)` is M2.
- `x between (lo .. hi)`: inclusive on both ends.

String operators (case-insensitive unless the name ends in `_cs`): `contains`, `!contains`, `has`,
`!has`, `startswith`, `endswith`, `matches regex`, and their `_cs` variants. `has` matches whole
terms, where a term is a maximal run of letters and digits. Regexes use Rust `regex` syntax
(RE2-like: no backreferences or lookaround, D-014).

- `=` is **never** comparison. Using it where a predicate is expected gives `DS0110` with a
  machine-applicable fix to `==`.
- `and`/`or` are words. `&&`/`||` give `DS0111` with a suggestion. `<>` gives `DS0107` suggesting `!=`.
- Null semantics follow SQL three-valued logic: comparisons with null are null, and `where` keeps
  only `true` rows. Arithmetic with null is null.

### 4.1 Operator typing

`num` is `int`, `long` or `real`. Operands are first widened per §4.2. Integer arithmetic always
produces `long`, even when both operands are `int` (D-021), so `qty * price_cents` on two `int`
columns cannot overflow at 2³¹.

| Expression | Operand types | Result |
|---|---|---|
| `a + b`, `a - b`, `a * b` | `int`/`long`, `int`/`long` | `long` |
| `a + b`, `a - b`, `a * b` | at least one `real` | `real` |
| `a / b` | `int`/`long`, `int`/`long` | `long`, truncating toward zero (D-014) |
| `a / b` | at least one `real` | `real` |
| `a % b` | `int`/`long`, `int`/`long` | `long` |
| `a + b`, `a - b` | `timespan`, `timespan` | `timespan` |
| `a + b`, `b + a` | `datetime`, `timespan` | `datetime` |
| `a - b` | `datetime`, `timespan` | `datetime` |
| `a - b` | `datetime`, `datetime` | `timespan` |
| `a * n`, `n * a`, `a / n` | `timespan`, `num` | `timespan` |
| `a / b` | `timespan`, `timespan` | `real` |
| `-a` | `int` or `long` | `long` |
| `-a` | `real` or `timespan` | same type |
| `==` `!=` | any two types of the same kind (after widening) | `bool` |
| `<` `<=` `>` `>=` | `num`, `string` (by Unicode code point), `datetime`, `timespan`, `bool` | `bool` |
| `=~` `!~` | `string`, `string` | `bool` (case-insensitive equality) |
| string operators | `string`, `string` | `bool` |
| `and` `or` `not` | `bool` | `bool` |

Anything else is `DS2003`, naming both types. Integer overflow and integer division or modulo by
zero are runtime errors (`DS4003`, D-014). This is part of the contract: a backend that cannot
match it lists a deviation in its `Capabilities` (ARCHITECTURE.md). Whether KQL behaves the same
way is not yet verified (Q-008); if it does not, these rules get a ≠ KQL mark. Real division by
zero follows IEEE 754 (`inf`, `nan`).

### 4.2 Implicit conversions

- Numeric widening only: `int` → `long` → `real`. `decimal` (M2) widens to `real` when mixed
  with `real`.
- `null` takes the type its context needs (`iff(p, null, 1)` is `long`). Where there is no
  context (`extend x = null`), `DS2009` asks for a typed null: `long(null)`, `string(null)`, ….
- No other implicit conversions. `"1" + 1` is a type error suggesting `tolong()` or `tostring()`.

## 5. Types

| DataScript type | Canonical Arrow type | Notes |
|---|---|---|
| `bool` | Boolean | |
| `int` | Int32 | Only from `toint()` and `int(null)`. Source columns and arithmetic use `long` (D-021) |
| `long` | Int64 | default integer |
| `real` | Float64 | |
| `decimal` | Decimal128(38, 18) | M2 |
| `string` | Utf8 | |
| `datetime` | Timestamp(µs, UTC) | |
| `timespan` | Duration(µs) | |
| `guid` | FixedSizeBinary(16) with extension type `arrow.uuid` | M2 |
| `dynamic` | Utf8 holding JSON, with extension type `arrow.json` (M2, may become a Variant type, Q-003) | |

Engines return results using the canonical Arrow types. Where the storage type alone does not say
which DataScript type a column has (`dynamic` and `string` are both Utf8), the result field
carries Arrow's canonical extension type metadata (`ARROW:extension:name`), so every frontend
(table, grid, JSON output, the WASM API) can tell them apart (D-024). A Utf8 column read from a
file without that metadata is a `string`.

### 5.1 Reading Arrow types

Source columns are widened to a DataScript type at the scan (D-012):

| Arrow type in the file | DataScript type | Notes |
|---|---|---|
| Boolean | `bool` | |
| Int8, Int16, Int32, Int64, UInt8, UInt16, UInt32 | `long` | All integers widen to `long` (D-021) |
| UInt64 | `long` | Values above `i64::MAX` are a runtime error (`DS4003`) naming the column |
| Float16, Float32, Float64 | `real` | |
| Decimal32, Decimal64, Decimal128, Decimal256 | `real` in M1, `decimal` from M2 | Precision loss in M1 is documented in the `getschema` output |
| Utf8, LargeUtf8, Utf8View, Dictionary(_, Utf8 \| LargeUtf8 \| Utf8View) | `string` | |
| Timestamp(any unit, any or no timezone) | `datetime` | Converted to UTC. No timezone means UTC. Nanoseconds are truncated to µs |
| Date32, Date64 | `datetime` | Midnight UTC |
| Duration(any unit) | `timespan` | Nanoseconds truncated to µs |
| FixedSizeBinary(16) with extension type `arrow.uuid` | `guid` (M2) | Without the extension type: none |
| Utf8 with extension type `arrow.json` | `dynamic` (M2) | |
| Struct, List, LargeList, Map | `dynamic` (M2) | |
| Anything else (Binary, Time32/64, Interval, …) | none | |

Columns with no DataScript type are readable but can only be projected (`project`, `project-away`,
`take`, …). Using one in an expression is `DS2008`, which names the Arrow type (rule 6 in
DIAGNOSTICS.md allows it because it comes from the user's file).

## 6. Render

```ds
... | render <kind> [with ( <prop> = <value>, ... )]
```

Kinds: `table`, `timechart`, `linechart`, `areachart`, `barchart` (horizontal), `columnchart`
(vertical), `piechart`, `scatterchart`, `card` (single value). An unknown kind is `DS5003`.

Properties: `title`, `xcolumn`, `ycolumns`, `series`, `xtitle`, `ytitle`, `legend` (`visible`/`hidden`),
`ymin`, `ymax`, `kind` (`default`/`stacked`/`stacked100`). An unknown or repeated property is
`DS5004`.

Defaults when properties are omitted (validated by the compiler):

- `timechart`: x = first datetime column; y = all numeric columns; series = remaining string columns.
- `barchart`/`columnchart`/`piechart`: x = first string column; y = first numeric column.
- `scatterchart`: x = first numeric column; y = second numeric column.

Runtimes without a UI ignore the chart, show the data, and print an info diagnostic once (`DS5001`).

## 7. Functions (M1 core set)

The authoritative list is the function catalog in `datascript-compiler` (it generates the reference docs).
Semantics follow KQL unless noted (D-014). All datetime functions work in UTC.

- **Aggregates** (`sum` and `sumif` of integers return `long`, of reals `real`; `avg`, `stdev`
  and `variance` return `real`): `count()`, `countif(p)`, `sum(x)`, `sumif(x, p)`, `avg(x)`, `min(x)`, `max(x)`,
  `dcount(x)` (exact in M1, ≠ KQL which estimates), `make_list(x)` (M2), `make_set(x)` (M2),
  `percentile(x, p)` (exact, nearest-rank; ≠ KQL which estimates), `stdev(x)`, `variance(x)`
  (sample), `arg_max(x, *)`/`arg_min(x, *)` (M2), `take_any(x)`. Aggregates ignore nulls;
  `sum` of no non-null values is null, `count()` counts rows.
- **Conditional**: `iff(p, a, b)`, `case(p1, v1, p2, v2, ..., else)`, `coalesce(a, b, ...)`,
  `isnull(x)`, `isnotnull(x)`, `isempty(x)`, `isnotempty(x)`.
- **Conversion**: `tostring`, `toint`, `tolong`, `toreal`, `tobool`, `todatetime`, `totimespan`.
  A value that does not convert becomes null (as in KQL).
- **String**: `strlen` (in Unicode scalar values), `tolower`, `toupper`, `trim(regex, s)` (as in KQL, D-031),
  `substring(s, start [, len])` (0-based), `strcat(...)`, `replace_string(s, find, repl)`,
  `split(s, sep)` (M2, returns dynamic), `indexof` (0-based, -1 if absent),
  `extract(regex, group, s)`.
- **Datetime**: `now()`, `ago(ts)`, `bin(x, size)` (also numbers; bins datetimes from the Unix
  epoch), `startofday`, `startofweek` (Sunday), `startofmonth`, `startofyear`, `endof*`,
  `datetime_part(part, dt)`, `dayofweek` (timespan since Sunday, as in KQL),
  `format_datetime(dt, fmt)` (KQL format specifiers such as `yyyy-MM-dd HH:mm:ss`), `datetime_diff`.
- **Math**: `abs`, `round(x [, digits])` (half away from zero), `floor`, `ceiling`, `sqrt`, `pow`,
  `log`, `log10`, `exp`.

`now()` is evaluated **once per program run** (D-015), from a clock the host provides (needed for
WASM and for deterministic tests).

## 8. Grammar sketch (EBNF)

```ebnf
program        = { statement ";" } [ statement ] ;
statement      = let_stmt | tabular_expr ;
let_stmt       = "let" ident "=" ( tabular_expr | scalar_expr ) ;   (* tabular iff a tabular_expr, §2 *)
tabular_expr   = ( "from" source | generator | "(" tabular_expr ")" ) { "|" operator } ;
source         = string_lit | ident | reader_call | "(" tabular_expr ")" ;
generator      = range_gen | datatable_gen | print_gen ;                (* M2 *)
right_table    = string_lit | ident | "(" tabular_expr ")" ;
operator       = where_op | project_op | extend_op | summarize_op | sort_op | top_op
               | take_op | count_op | distinct_op | join_op | union_op | render_op | ... ;
where_op       = "where" scalar_expr ;
project_op     = "project" proj_item { "," proj_item } ;
proj_item      = [ ident "=" ] scalar_expr ;
summarize_op   = "summarize" [ agg_item { "," agg_item } ] [ "by" proj_item { "," proj_item } ] ;
sort_op        = ( "sort" | "order" ) "by" sort_key { "," sort_key } ;
sort_key       = scalar_expr [ "asc" | "desc" ] [ "nulls" ( "first" | "last" ) ] ;
top_op         = "top" scalar_expr "by" sort_key ;
join_op        = "join" [ "kind" "=" ident ] right_table "on" join_cond { "," join_cond } ;
join_cond      = ident | "$left" "." ident "==" "$right" "." ident ;
union_op       = "union" right_table { "," right_table } ;
render_op      = "render" ident [ "with" "(" prop { "," prop } ")" ] ;
in_expr        = scalar_expr ( "in" | "!in" | "in~" | "!in~" ) "(" scalar_expr { "," scalar_expr } ")" ;
between_expr   = scalar_expr ( "between" | "!between" ) "(" scalar_expr ".." scalar_expr ")" ;
typed_null     = type_name "(" "null" ")" ;                         (* §1, not a call *)
hyph_op_name   = ident "-" ident ;            (* adjacent tokens, no trivia; only right after "|" *)
```

Keywords are contextual (§1.1): the parser checks an identifier's text where the grammar expects a
keyword.

The parser accepts more than this sketch in table positions: a source after `from` and a
`right_table` may be any scalar expression, and the binder rejects the ones that are not tables
(D-029). In the tree, `from source` is always a `PIPELINE` node, even with no operators, with the
`from` token as its first child (D-036). A `join_cond` is parsed as an expression and checked by the binder.

## 9. Not planned (for now)

Database/cluster functions (`cluster()`, `database()`), management commands (`.show`), ingestion,
`materialize()` (may come later as an engine hint), and query parameters (`declare query_parameters`).
