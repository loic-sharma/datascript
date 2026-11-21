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

## Severity

- `error`: the program cannot run.
- `warning`: it runs, but probably not as intended (e.g. `take` before `sort`, a predicate that
  is always true). These lints are task M1-14.
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
| DS0001–DS0019 | M1-01 lexer |
| DS0100–DS0101 | M1-02 parser (generic errors) |
| DS0102–DS0149 | M1-03 syntax diagnostics |
| DS1001–DS1029, DS2001–DS2029 | M1-05 binder core (and M1-04 for DS2002, DS2003, DS2006, DS2009) |
| DS1030–DS1049, DS2030–DS2039 | M1-16 operator schemas and joins (also implements the registered DS1005, DS1007) |
| DS2040–DS2049 | M1-17 source resolution and capability pass (also implements the registered DS2008, DS3006, DS4001) |
| DS2100–DS2149 | M1-14 lints |
| DS3001–DS3049 | M1-08 sources (DS3007 reserved for M4-01) |
| DS4001–DS4049 | M1-07 / M1-09 engine |
| DS5001–DS5049 | M1-10 render |
| DS0004, DS9001 | M0-02 diagnostics core |

**Retired codes**: DS2007 (D-020). List them here when a code is retired.

| Range | Area |
|---|---|
| DS0001–DS0099 | Lexical (bad characters, unterminated strings, bad literals) |
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
| DS0104 | error | unknown operator (with did-you-mean; also `project - away` with spaces, D-026) |
| DS0105 | error | SQL keyword used (`SELECT ... FROM`, `GROUP BY`, `WHERE` and `AND`/`OR`/`NOT` in caps, `DESC`, `IS NULL`, `LIKE`, `AS`, …), with the DataScript equivalent |
| DS0106 | error | missing `;` between statements (machine-applicable fix, except after an operator that takes a list, where `,` is offered too and neither is certain) |
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
| DS0125 | error | query does not start with a table (`where a > 1`, or a pipe with nothing before it) |
| DS0126 | error | reserved word used as a name (help: quote it, `['null']`) |
| DS0127 | error | trailing comma at the end of a list |
| DS0128 | error | list after `in` written in square brackets |
| DS0129 | error | `in ~` / `!in ~` written with a space (machine-applicable fix to `in~`) |
| DS1001 | error | unknown column |
| DS1002 | error | unknown name (let/table) |
| DS1003 | error | name already defined |
| DS1004 | error | unknown function |
| DS1005 | error | duplicate output column name |
| DS1007 | info | `join` renamed colliding non-key columns (says how to rename explicitly; `on key` columns are merged, D-019) |
| DS2001 | error | predicate is not `bool` |
| DS2002 | error | wrong number of arguments |
| DS2003 | error | operator not defined for these types |
| DS2004 | error | aggregate function used outside `summarize` |
| DS2005 | error | non-aggregate expression in `summarize` aggregate list |
| DS2006 | error | function argument has the wrong type (points at the argument, lists signatures) |
| DS2007 | — | *retired, never implemented* (was: name used both as a table and as a scalar, D-010; superseded by D-020) |
| DS2008 | error | column type has no DataScript equivalent and can only be projected |
| DS2009 | error | untyped `null` with no context (suggest `long(null)`, …) |
| DS2101 | warning | `take`/`limit` before `sort`: the rows kept are arbitrary *(planned, M1-14)* |
| DS2102 | warning | predicate is always true or always false *(planned, M1-14)* |
| DS2103 | warning | `sort` key without `asc`/`desc` (default is asc, ≠ KQL); machine-applicable fix inserts `asc` *(planned, M1-14, D-005)* |
| DS3001 | error | file not found (with did-you-mean over nearby files) |
| DS3002 | error | unknown file format |
| DS3003 | error | unknown reader option |
| DS3004 | error | glob matched no files |
| DS3005 | error | incompatible schemas across glob matches |
| DS3006 | error | source path or reader option is not a constant |
| DS3007 | error | path is outside the UI root directory *(planned, M4-01, D-016)* |
| DS3008 | error | file could not be read (permission denied, malformed CSV/JSON row at runtime) |
| DS4001 | error | function not supported by this engine |
| DS4002 | error | engine runtime error |
| DS4003 | error | arithmetic error: integer division by zero, overflow, or out-of-range value (D-014) |
| DS5001 | info | chart not drawn in this runtime |
| DS5002 | error | chart needs a column of a kind the result lacks |
| DS5003 | error | unknown render kind (did-you-mean) |
| DS5004 | error | unknown or repeated render property (did-you-mean) |
| DS9001 | error | internal compiler error (also used for caught panics) |
