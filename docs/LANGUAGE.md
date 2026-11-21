# DataScript language (draft specification)

Status: **draft**. This is the contract that parser, binder, and tests are written against.
Change it deliberately, in the same change as the code, and log notable decisions in
[DECISIONS.md](DECISIONS.md).

DataScript is *KQL-like, free to diverge*. It keeps KQL's feel, operator names, and function names.
It departs from KQL where KQL is surprising or where a change gives clearer errors. Each divergence
is marked **≠ KQL**.

## 1. Lexical structure

- **Encoding**: UTF-8. Identifiers and keywords are **case-sensitive** (as in KQL).
- **Comments**: `// to end of line`. (≠ KQL: block comments `/* */` are also allowed.)
- **Whitespace** is insignificant except as a separator. Newlines carry no meaning.
- **Identifiers**: `[A-Za-z_][A-Za-z0-9_]*`. Names with other characters, or that clash with a
  keyword, are quoted: `['order id']` or `["order id"]`.
- **Literals**

| Kind | Examples | Notes |
|---|---|---|
| long | `42`, `-7`, `1_000_000` | 64-bit signed |
| real | `3.14`, `1e-3`, `.5` | 64-bit float |
| string | `"text"`, `'text'`, `@"C:\raw"` | `\` escapes in normal strings; `@` = verbatim |
| bool | `true`, `false` | |
| null | `null` | ≠ KQL: an untyped `null` literal, coerced by context |
| timespan | `1d`, `2h`, `30m`, `10s`, `250ms`, `1.5h`, `timespan(1.02:03:04)` | |
| datetime | `datetime(2024-01-31)`, `datetime(2024-01-31T12:00:00Z)` | ISO 8601 |
| dynamic | `dynamic([1, 2])`, `dynamic({"a": 1})` | M2 |

## 2. Program structure

A program is a sequence of **statements** separated by `;`. A trailing `;` is optional.

```ds
let threshold = 100;                         // scalar let
let big = 'orders.csv' | where amount > threshold;   // tabular let
big | summarize count() by customerId        // query statement (produces a result)
```

- `let name = <expr>` binds a scalar or tabular expression. Lets are immutable and lexically
  scoped to the rest of the program. Re-binding a name is an error (`DS1003`) (≠ KQL, which allows
  shadowing).
- Each **query statement** (a statement that is not a `let`) produces one result. A program may
  contain several. They are shown in order (≠ KQL's multi-result behaviour is engine-specific).
- A program with no query statement is valid for `ds check`. `ds run` warns that there is nothing
  to run (`DS0004`).

## 3. Tabular expressions

```
tabular_expr := source ( '|' operator )*
```

### 3.1 Sources

| Source | Example | Notes |
|---|---|---|
| File path literal | `'data/orders.csv'` | Format from extension (`.csv`, `.tsv`, `.parquet`, `.json`, `.ndjson`/`.jsonl`). Paths are relative to the **script's directory** (or cwd in the REPL) |
| Glob | `'logs/2024-*/*.parquet'` | All matches must share a format. Schemas are merged by name |
| Reader function | `read_csv('x.txt', delimiter = ';', header = false)` | Explicit format and options. Also `read_parquet`, `read_json` |
| Name | `orders` | A tabular `let`, or a table registered by the host (web UI drag-drop, `--table orders=path` on the CLI) |
| Inline data | `datatable(a: long, b: string) [1, "x", 2, "y"]` | M2 |
| Range | `range x from 1 to 10 step 1` | M2 |

DuckDB-style path sources are the main way to read files. A string literal at the start of a pipe
is always a source, never a scalar.

Reader options (M1 set):

| Function | Options |
|---|---|
| `read_csv` | `delimiter`, `header` (bool), `quote`, `escape`, `null_value`, `schema` (M2), `infer_rows` |
| `read_parquet` | none in M1 |
| `read_json` | `format = "ndjson" \| "array"` |

### 3.2 Operators

M1 operators are marked ●, M2 operators ○.

| Operator | Syntax | Notes |
|---|---|---|
| ● `where` | `where <predicate>` | Predicate must be bool. `=` in a predicate gives a targeted error suggesting `==` |
| ● `project` | `project a, b, c = a + b` | Select, rename, and compute |
| ● `project-away` | `project-away a, b` | Wildcards `col*` allowed |
| ● `project-rename` | `project-rename new = old` | |
| ● `extend` | `extend c = a * 2` | Adds or replaces columns. Unnamed expressions get `Column1`, … |
| ● `summarize` | `summarize [name =] agg(...) , ... [by key [, ...]]` | Keys may be expressions: `by bin(ts, 1h)`. Unnamed aggregates get names like `sum_amount` |
| ● `sort` / `order` | `sort by a desc, b asc nulls last` | ≠ KQL: default direction is **asc**. KQL defaults to desc, a common surprise. Decision D-005 |
| ● `top` | `top 10 by revenue desc` | Default desc, as in KQL (top-N is about largest) |
| ● `take` / `limit` | `take 100` | No ordering guarantee |
| ● `count` | `count` | One row, column `Count` |
| ● `distinct` | `distinct a, b` or `distinct *` | |
| ● `join` | `join kind = inner (right) on key` or `on $left.a == $right.b` | Kinds: `inner` (default), `leftouter`, `rightouter`, `fullouter`, `leftsemi`, `leftanti`, `rightsemi`, `rightanti`. ≠ KQL: no `innerunique`, and the default is `inner`. Decision D-004 |
| ● `union` | `union t2, t3` | By column name. Missing columns become null |
| ● `render` | `render timechart [with (title = "...")]` | Must be last. See §6 |
| ● `getschema` | `getschema` | Returns the column name/type table |
| ○ `lookup` | `lookup (dim) on key` | |
| ○ `mv-expand` | `mv-expand tags` | Needs `dynamic` |
| ○ `parse` | `parse msg with "user=" user " id=" id:long` | |
| ○ `make-series` | `make-series sum(x) on ts step 1h` | |
| ○ `sample` | `sample 100` | |
| ○ `as` | `as name` | Names a sub-result for later reference |
| ○ `project-reorder` | | |

A right-hand table in `join`/`union`/`lookup` is a name or a parenthesized tabular expression.

### 3.3 Column naming

- Output column names must be unique. Collisions after `join` are renamed `name1`, `name2` (as in
  KQL) and the binder emits an info diagnostic that says how to rename them explicitly.
- `summarize sum(amount)` produces `sum_amount`. `count()` produces `count_`.

## 4. Scalar expressions

Precedence, from lowest to highest:

| Level | Operators |
|---|---|
| 1 | `or` |
| 2 | `and` |
| 3 | `not` (prefix, also `not(x)`) |
| 4 | `==` `!=` `<` `<=` `>` `>=` `=~` `!~` `in` `!in` `in~` `between` `!between` and string operators |
| 5 | `+` `-` |
| 6 | `*` `/` `%` |
| 7 | unary `-` |
| 8 | call `f(x)`, member `a.b`, index `a[0]` (M2 for dynamic) |

String operators (case-insensitive unless the name ends in `_cs`): `contains`, `!contains`, `has`,
`!has`, `startswith`, `endswith`, `matches regex`, and their `_cs` variants.

- `=` is **never** comparison. Using it where a predicate is expected gives `DS0110` with a
  machine-applicable fix to `==`.
- `and`/`or` are words. `&&`/`||` give `DS0111` with a suggestion.
- Null semantics follow SQL three-valued logic: comparisons with null are null, and `where` keeps
  only `true` rows.
- Mixing `long` and `real` widens to `real`. No other implicit conversions. `"1" + 1` is a type
  error suggesting `toint()` or `tostring()`.

## 5. Types

| DataScript type | Arrow type | Notes |
|---|---|---|
| `bool` | Boolean | |
| `int` | Int32 | |
| `long` | Int64 | default integer |
| `real` | Float64 | |
| `decimal` | Decimal128(38, 18) | M2 |
| `string` | Utf8 / LargeUtf8 | |
| `datetime` | Timestamp(µs, UTC) | |
| `timespan` | Duration(µs) | |
| `guid` | FixedSizeBinary(16) | M2 |
| `dynamic` | JSON-encoded Utf8 (M2, may become a Variant type) | |

Source columns with Arrow types that have no DataScript type (e.g. nested structs before M2) are
readable but can only be projected. Using them in an expression gives `DS2xxx` naming the Arrow type.

## 6. Render

```ds
... | render <kind> [with ( <prop> = <value>, ... )]
```

Kinds: `table`, `timechart`, `linechart`, `areachart`, `barchart` (horizontal), `columnchart`
(vertical), `piechart`, `scatterchart`, `card` (single value).

Properties: `title`, `xcolumn`, `ycolumns`, `series`, `xtitle`, `ytitle`, `legend` (`visible`/`hidden`),
`ymin`, `ymax`, `kind` (`default`/`stacked`/`stacked100`).

Defaults when properties are omitted (validated by the compiler):

- `timechart`: x = first datetime column; y = all numeric columns; series = remaining string columns.
- `barchart`/`columnchart`/`piechart`: x = first string column; y = first numeric column.
- `scatterchart`: x = first numeric column; y = second numeric column.

Runtimes without a UI ignore the chart, show the data, and print an info diagnostic once (`DS5001`).

## 7. Functions (M1 core set)

The authoritative list is the function catalog in `datascript-compiler` (it generates the reference docs).

- **Aggregates**: `count()`, `countif(p)`, `sum(x)`, `sumif(x, p)`, `avg(x)`, `min(x)`, `max(x)`,
  `dcount(x)` (exact in M1), `make_list(x)` (M2), `make_set(x)` (M2), `percentile(x, p)`,
  `stdev(x)`, `variance(x)`, `arg_max(x, *)`/`arg_min(x, *)` (M2), `take_any(x)`.
- **Conditional**: `iff(p, a, b)`, `case(p1, v1, p2, v2, ..., else)`, `coalesce(a, b, ...)`,
  `isnull(x)`, `isnotnull(x)`, `isempty(x)`, `isnotempty(x)`.
- **Conversion**: `tostring`, `toint`, `tolong`, `toreal`, `tobool`, `todatetime`, `totimespan`.
- **String**: `strlen`, `tolower`, `toupper`, `trim`, `substring(s, start [, len])`, `strcat(...)`,
  `replace_string(s, find, repl)`, `split(s, sep)` (M2, returns dynamic), `indexof`, `extract(regex, group, s)`.
- **Datetime**: `now()`, `ago(ts)`, `bin(x, size)` (also numbers), `startofday`, `startofweek`,
  `startofmonth`, `startofyear`, `endof*`, `datetime_part(part, dt)`, `dayofweek`, `format_datetime`, `datetime_diff`.
- **Math**: `abs`, `round(x [, digits])`, `floor`, `ceiling`, `sqrt`, `pow`, `log`, `log10`, `exp`.

`now()` is evaluated **once per query**, from a clock the host provides (needed for WASM and for
deterministic tests).

## 8. Grammar sketch (EBNF)

```ebnf
program        = { statement ";" } [ statement ] ;
statement      = let_stmt | tabular_expr ;
let_stmt       = "let" ident "=" ( tabular_expr | scalar_expr ) ;
tabular_expr   = source { "|" operator } ;
source         = string_lit | ident | call | "(" tabular_expr ")" ;
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
render_op      = "render" ident [ "with" "(" prop { "," prop } ")" ] ;
```

Operator names such as `project-away` are single tokens. The lexer recognizes them only right
after `|`, so `a - b` and `project-away` never conflict.

## 9. Not planned (for now)

Database/cluster functions (`cluster()`, `database()`), management commands (`.show`), ingestion,
`materialize()` (may come later as an engine hint), and query parameters (`declare query_parameters`).
