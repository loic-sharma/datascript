# Diagnostics

Clear, human-readable errors are DataScript's top priority. Every surface (CLI, REPL, LSP, web UI,
WASM) renders the same `Diagnostic` values from `datascript-diagnostics`. This document is the style
guide and the registry of error codes.

## What a good diagnostic looks like

```
error[DS1001]: unknown column `amout`
  ┌─ top_customers.ds:3:27
  │
3 │ | summarize revenue = sum(amout) by customerId
  │                           ^^^^^ no column named `amout` here
  │
  = help: a column with a similar name exists: `amount`
  = note: columns available at this point: orderId, customerId, amount, timestamp
```

```
error[DS0110]: `=` is assignment, not comparison
  ┌─ query.ds:2:16
  │
2 │ | where status = "shipped"
  │                ^ assignment is not allowed here
  │
  = help: use `==` to compare values: `status == "shipped"`
```

```
error[DS2003]: cannot add `string` and `long`
  ┌─ query.ds:2:18
  │
2 │ | extend total = price + 1
  │                  ^^^^^   - this is `long`
  │                  │
  │                  this is `string` (column `price` was read from products.csv)
  │
  = help: if `price` holds numbers, convert it first: `toreal(price) + 1`
```

(From M2-09 on, a second help also suggests declaring the type:
`read_csv('products.csv', schema = (price: real))`. Never suggest syntax that does not exist yet.)

## Style rules

1. **Message**: one line, starts lowercase, no trailing period, says *what* is wrong in user terms.
   Name the user's things in backticks: `` unknown column `amout` ``.
2. **Primary label** points at the smallest span that explains the error. Its text adds information
   and does not repeat the message.
3. **Secondary labels** show *why*, e.g. where a type came from or where a name was defined.
4. **Help** says what to do. Give a concrete rewrite when we can. Attach a machine-applicable
   `Suggestion` when the fix is certain (`=` → `==`, a typo with one close match). The LSP turns
   these into quick fixes.
5. **Notes** give context: available columns, the expected signature, a link to the docs.
6. **No jargon.** Say "expected a column name after `by`", not "unexpected token IDENT". Never
   expose parser internals, Rust types, DataFusion plan nodes, or Arrow type names unless the user
   wrote them.
7. **Help people coming from other languages.** Recognize SQL and KQL habits and say what the
   DataScript equivalent is: `SELECT` → `project`, `GROUP BY` → `summarize ... by`, `&&` → `and`,
   `innerunique` → explain the divergence.
8. **One root cause, one error.** Do not cascade: after an unknown column, give the expression an
   error type that silently matches anything downstream.
9. **Did-you-mean** uses edit distance over the names that are in scope (columns, lets, functions,
   operators, render kinds, reader options). Suggest only when there is a single clear winner, or list up to three.
10. **Engine errors** (runtime) are wrapped. The message is ours, the engine's text goes in a note,
    and the span points at the IR node's source when we can trace it.
11. **Every diagnostic has a primary span** unless there is truly no location (`DS0004`, an engine
    error that cannot be traced). Then `primary` is `None` and renderers show the file name only.
12. **Stable output.** Lists in messages and notes (available columns, nearby files, did-you-mean
    candidates) are sorted deterministically. Paths are shown relative to the script's directory
    when they are inside it.

### Recurring patterns (M1 review)

The M1-13 review read every golden diagnostic against the rules above. What came up more than
once, and is now the house style:

- **Only suggest a fix that works.** A conversion help must produce a value: `toreal(timestamp)`
  is null (LANGUAGE.md §7), so it is never offered. When the real problem is likely something
  else, say that instead: swapped arguments (`datetime_part(ts, 'year')`), a number used as a
  condition (`x != 0`), a bare number where a `timespan` is needed (`1d`).
- **Never use `DS9001` for a user's mistake.** "Not implemented yet" with an internal-error code
  was used for `take amount`; a mistake the user can fix gets a real code and a help.
- **Check constants early.** If a value is known before the query runs, report it from `check`,
  at the argument, with the same code the engine would use at run time.
- **Code spans hold code.** Do not wrap prose that itself contains backticks in a code span
  (`` `min(x: T) -> T`, where T is a number or `string` ``). Quoted values in messages are code:
  `` `'ndjson'` ``, not `'ndjson'`.
- **Short labels are fine when the message says it all**, such as "this is SQL" or "not a chart
  kind". A label must not restate the message in other words; when there is nothing to add, say
  where with a few words ("given again here").
- **The same situation gets the same words.** "remove one of them" for repeated properties and
  options; "did you mean `x`?" with a machine-applicable fix for one close match; "a ... with a
  similar name exists: `x`" for names.
- **Measure text in terminal columns** (`text_width`), not characters, wherever output is
  aligned.

## Severity

- `error`: the program cannot run.
- `warning`: it runs, but probably not as intended (e.g. a `where` condition that is always
  true, a `sort` key without a direction). These lints run on the bound program; their codes and
  default severities are listed in one table, `datascript_compiler::lints::LINTS` (M1-14).
- `info`: neutral facts (e.g. "render ignored: this runtime cannot draw charts", "join renamed
  `customerId` to `customerId1`").

At most 50 errors are reported per file; then one note says how many were omitted.

## Error code ranges

Codes are stable. They are never reused or renumbered, and retired codes stay reserved. Every code
gets a long-form explanation (`ds explain-error DS1001`, M2) in `docs/errors/DSxxxx.md`.

**Allocation (D-018).** Parallel tasks must not pick the same "next free" code. Each task owns a
block, listed below and in its task file. Allocate only inside your block. If you run out, reserve
a new block in this table in a separate small commit to `main` before using it.
| Block | Owner |
|---|---|
| DS0001–DS0019 except DS0004 | M1-01 lexer (M1-20 added DS0007 from this block) |
| DS0100–DS0101 | M1-02 parser (generic errors) |
| DS0102–DS0149 | M1-03 syntax diagnostics |
| DS0150–DS0151 | M1-24 `from` keyword |
| DS0152–DS0159 | M1-25 `from` mistakes and statement starts |
| DS1001–DS1029, DS2001–DS2029 | M1-05 binder core (and M1-04 for DS2002, DS2003, DS2006, DS2009) |
| DS1030–DS1049, DS2030–DS2039 | M1-16 operator schemas and joins (also implements the registered DS1005, DS1007) |
| DS2040–DS2049 | M1-17 source resolution and capability pass (also implements the registered DS2008, DS3006, DS4001, and builds DS3002 and DS3003 in the binder, D-041) |
| DS2050–DS2059 | M1-13 M1 acceptance and polish |
| DS2100–DS2149 | M1-14 lints |
| DS3001–DS3049 | M1-08 sources (DS3007 reserved for M4-01) |
| DS4001–DS4049 | M1-07 / M1-09 engine |
| DS5001–DS5049 | M1-10 render |
| DS0004, DS9001 | M0-02 diagnostics core |
| DS0200–DS0219, DS1050–DS1059, DS2200–DS2219 | M2-01 remaining tabular operators |
| DS0220–DS0229, DS2220–DS2229, DS4050–DS4054 | M2-02 `parse` operator |
| DS0230–DS0239, DS2230–DS2249 | M2-03 `dynamic` type, JSON, `mv-expand` (also rewords the registered DS2012) |
| DS0240–DS0249, DS2250–DS2259, DS5050–DS5054 | M2-04 `make-series` |
| DS0250–DS0269, DS1060–DS1079, DS2260–DS2279 | M2-05 user-defined functions |
| DS0270–DS0279, DS2280–DS2299, DS4055–DS4059 | M2-06 function library |
| DS0280–DS0289, DS1080–DS1084, DS2300–DS2309, DS3050–DS3059 | M2-09 reader schema overrides |
| DS2310–DS2319 | M2-10 `decimal` and `guid` |
| DS2320–DS2329 | M2-20 conversions that can never succeed |
| DS3060–DS3064 | M2-12 runtime read errors (none expected; DS3008 is reused) |
| DS3065–DS3069, DS4060–DS4064 | M4-01 local web UI server (Q-007's "results truncated" info diagnostic; DS3007 above) |
| DS3070–DS3074 | M2-22 CSV sniffing |
| DS0290–DS0294 | M2-23 `describe` operator |
| DS3080–DS3089 | M2-24 Excel reader and writer |
| DS0295–DS0299, DS3090–DS3094, DS4065–DS4069 | M2-25 `output` operator |
| DS2330–DS2339 | M2-26 `output` writer functions and options |
| DS3095–DS3099 | M2-27 `output` partitioned output |
| DS3100–DS3119 | M7-01 HTTPS sources |
| DS3120–DS3139 | M7-02 S3, GCS and Azure sources |

**Retired codes**: DS0127 (D-059), DS2007 (D-020), DS2101 (D-050). List them here when a code is retired.

| Range | Area |
|---|---|
| DS0001–DS0099 | Lexical (bad characters, unterminated strings, bad literals), except `DS0004` (no query to run), which M0-02 registered before the ranges were set and which stays where it is |
| DS0100–DS0999 | Syntax (parse errors, misplaced operators, foreign-language habits) |
| DS1000–DS1999 | Names (unknown column/let/function/table, duplicate definitions) |
| DS2000–DS2999 | Types (mismatches, wrong arity, non-bool predicates, aggregate misuse) |
| DS3000–DS3999 | Sources and I/O (file not found, unknown format, bad reader options, schema conflicts in globs) |
| DS4000–DS4999 | Engine (unsupported feature in this backend, runtime failures) |
| DS5000–DS5999 | Render (bad chart kind/properties, incompatible columns, unsupported in runtime) |
| DS9000–DS9999 | Internal errors (bugs; the message asks the user to report it) |

## Code registry

Add codes here when you introduce them. Keep the table sorted. "Planned" codes are reserved by a
design decision and implemented by the task named in the allocation table.

| Code | Severity | Summary |
|---|---|---|
| DS0001 | error | unexpected character |
| DS0002 | error | unterminated string literal |
| DS0003 | error | invalid number/timespan/datetime literal |
| DS0004 | warning | program has no query to run (no span) |
| DS0005 | error | unknown escape sequence in string (help: `@"..."` or `/` for paths) |
| DS0006 | error | unterminated block comment |
| DS0007 | error | empty quoted name `['']` |
| DS0100 | error | expression nested too deeply, or an operator chain too long, for the parser |
| DS0101 | error | expected X, found Y (generic parse error, used as a last resort) |
| DS0102 | error | missing `|` between operators |
| DS0103 | error | pipe with no operator after it |
| DS0104 | error | unknown operator (with did-you-mean; also `project - away` with spaces, D-026, and a row generator such as `range` before M2-01) |
| DS0105 | error | SQL keyword used (`SELECT ... FROM`, rewritten as a `from` pipeline; `GROUP BY`, `WHERE` and `AND`/`OR`/`NOT` in caps, `DESC`, `IS NULL`, `LIKE`, `AS`, …), with the DataScript equivalent |
| DS0106 | error | missing `;` between statements (machine-applicable fix, except after an operator that takes a list, where `,` is offered too and neither is certain, and before a `from` on the same line; a `from` first on its line always makes `;` certain, D-037) |
| DS0107 | error | `<>` used instead of `!=` |
| DS0110 | error | `=` used for comparison (also `===` and `= =`, reported once with a fix to `==`) |
| DS0111 | error | `&&`/`||`/`!` used instead of `and`/`or`/`not` |
| DS0112 | error | unbalanced parenthesis or bracket (points at the opening one) |
| DS0113 | error | method-style call `a.f(x)` (DataScript has no methods) |
| DS0114 | error | prefix `not` without parentheses as the operand of a tighter operator (`1 + not x`) |
| DS0115 | error | `where` with conditions separated by commas |
| DS0116 | error | `distinct *` with other columns |
| DS0117 | error | SQL form with no DataScript equivalent: `count(*)`, `project *`, `''` inside a string |
| DS0120 | error | `render` is not the last operator |
| DS0121 | error | missing `by` in `sort`/`top` |
| DS0122 | error | missing `on` in `join`/`lookup` |
| DS0123 | error | unknown join kind (did-you-mean) |
| DS0124 | error | `innerunique` is not supported (explains D-004) |
| DS0125 | error | query starts with an operator or a pipe instead of `from` and a table (`where a > 1`, `| take 5`) |
| DS0126 | error | reserved word used as a name (help: quote it, `['null']`) |
| DS0127 | — | *retired* (was: trailing comma at the end of a list; D-059 allows them) |
| DS0128 | error | list after `in` written in square brackets |
| DS0129 | error | `in ~` / `!in ~` written with a space (machine-applicable fix to `in~`) |
| DS0150 | error | query or pipeline does not start with `from` (machine-applicable fix inserts it when the head is clearly a table, D-036) |
| DS0151 | error | `from` with no table after it |
| DS0152 | error | `from` where it is not written: after `|`, or on the right of `join` or `union` (fix removes it, or turns the `|` into `;` when a new query follows, D-037) |
| DS0153 | error | SQL clause order: a SQL keyword after the table without `|` (`from t select a`), or `FROM` after the column list (`| project a from t`) |
| DS1001 | error | unknown column |
| DS1002 | error | unknown name (let/table) |
| DS1003 | error | name already defined |
| DS1004 | error | unknown function |
| DS1005 | error | duplicate output column name |
| DS1006 | error | a table name used where a value is expected (`let alias = orders;`) |
| DS1007 | info | `join` renamed colliding non-key columns (says how to rename explicitly; `on key` columns are merged, D-019) |
| DS1030 | error | `join` condition that is not a column name or `$left.a == $right.b` |
| DS1031 | error | `$left`/`$right` used outside a `join` condition |
| DS1032 | error | `distinct` given an expression instead of a column name |
| DS2001 | error | predicate is not `bool` |
| DS2002 | error | wrong number of arguments |
| DS2003 | error | operator not defined for these types |
| DS2004 | error | aggregate function used outside `summarize` |
| DS2005 | error | non-aggregate expression in `summarize` aggregate list |
| DS2006 | error | function argument has the wrong type (points at the argument, lists signatures) |
| DS2007 | — | *retired, never implemented* (was: name used both as a table and as a scalar, D-010; superseded by D-020) |
| DS2008 | error | column type has no DataScript equivalent and can only be projected |
| DS2009 | error | untyped `null` with no context (suggest `long(null)`, …) |
| DS2010 | error | a pipeline (a table) used where a value is expected |
| DS2011 | error | named argument in a call to a function that takes none |
| DS2012 | error | field access or indexing on a value that is not `dynamic` (`dynamic` is M2) |
| DS2030 | error | `summarize` with no aggregate and no `by` key |
| DS2040 | error | a value that is not a table or a path used as a source (`from 5`, a `let` bound to a number) |
| DS2041 | error | reader option value has the wrong type or is not allowed |
| DS2042 | error | reader call without exactly one path argument |
| DS2043 | error | reader option given twice |
| DS2050 | error | `take`/`top` row count that is not a constant whole number of 0 or more |
| DS2101 | — | *retired, never implemented* (was: `take`/`limit` before `sort`, D-050) |
| DS2102 | warning | `where` condition that is always true or always false after folding literals (D-051) |
| DS2103 | warning | `sort` key without `asc`/`desc` (default is asc, ≠ KQL); machine-applicable fix inserts `asc` (D-005) |
| DS3001 | error | file not found (with did-you-mean over nearby files) |
| DS3002 | error | unknown file format |
| DS3003 | error | unknown reader option |
| DS3004 | error | glob matched no files |
| DS3005 | error | incompatible schemas across glob matches |
| DS3006 | error | source path or reader option is not a constant |
| DS3007 | error | path is outside the UI root directory *(planned, M4-01, D-016)* |
| DS3008 | error | file could not be read (permission denied, malformed CSV/JSON row at runtime) |
| DS3009 | error | invalid glob pattern |
| DS4001 | error | function, `join` kind or file format not supported by this engine (reported by `check`, from the engine's capabilities) |
| DS4002 | error | engine runtime error |
| DS4003 | error | arithmetic error: integer division by zero, overflow, or out-of-range value (D-014) |
| DS4004 | error | invalid argument value: a regular expression that does not compile, an unknown datetime part or format, a percentile outside 0–100 (reported by `check` when the argument is a constant, D-049) |
| DS5001 | info | chart not drawn in this runtime (once per program run, not for `render table`) |
| DS5002 | error | chart role cannot be filled: no column of the type the kind needs, or an explicit column of the wrong type (names the role and lists the result's columns) |
| DS5003 | error | unknown render kind (did-you-mean; a KQL kind DataScript does not support yet says so instead) |
| DS5004 | error | unknown or repeated render property (did-you-mean; a KQL property DataScript does not support yet says so instead) |
| DS5005 | error | bad value for a render property: wrong form or count, unknown `legend`/`kind` word (did-you-mean), `ymin` not below `ymax` |
| DS5006 | error | render property does not apply to this chart kind (says which properties the kind accepts; `kind = map` says map charts are not supported yet) |
| DS5007 | error | one column used in two chart roles (x, y, series) |
| DS5008 | error | `render` outside a query statement (in a tabular `let`, a parenthesized pipeline, or the right side of `join`/`union`) |
| DS9001 | error | internal compiler error (also used for caught panics) |
