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
  ┌─ query.ds:2:20
  │
2 │ | extend total = price + 1
  │                  ^^^^^   - this is `long`
  │                  │
  │                  this is `string` (column `price` was read from orders.csv)
  │
  = help: if `price` holds numbers, convert it first: `toreal(price) + 1`
  = help: or declare the column type: read_csv('orders.csv', schema = (price: real))
```

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

## Severity

- `error`: the program cannot run.
- `warning`: it runs, but probably not as intended (e.g. `take` before `sort`, a `join` producing
  renamed columns, a predicate that is always true).
- `info`: neutral facts (e.g. "render ignored: this runtime cannot draw charts").

## Error code ranges

Codes are stable. They are never reused or renumbered, and retired codes stay reserved. Every code
gets a long-form explanation (`ds explain-error DS1001`, M2) in `docs/errors/DSxxxx.md`.

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

## Initial code registry

Add codes here when you introduce them. Keep the table sorted.

| Code | Severity | Summary |
|---|---|---|
| DS0001 | error | unexpected character |
| DS0002 | error | unterminated string literal |
| DS0003 | error | invalid number/timespan/datetime literal |
| DS0004 | warning | program has no query to run |
| DS0101 | error | expected X, found Y (generic parse error, used as a last resort) |
| DS0102 | error | missing `|` between operators |
| DS0103 | error | pipe with no operator after it |
| DS0104 | error | unknown operator (with did-you-mean) |
| DS0105 | error | SQL keyword used (`SELECT`, `FROM`, `GROUP BY`, …), with the DataScript equivalent |
| DS0110 | error | `=` used for comparison |
| DS0111 | error | `&&`/`||`/`!` used instead of `and`/`or`/`not` |
| DS0120 | error | `render` is not the last operator |
| DS1001 | error | unknown column |
| DS1002 | error | unknown name (let/table) |
| DS1003 | error | name already defined |
| DS1004 | error | unknown function |
| DS1005 | error | duplicate output column name |
| DS2001 | error | predicate is not `bool` |
| DS2002 | error | wrong number of arguments |
| DS2003 | error | operator not defined for these types |
| DS2004 | error | aggregate function used outside `summarize` |
| DS2005 | error | non-aggregate expression in `summarize` aggregate list |
| DS3001 | error | file not found (with did-you-mean over nearby files) |
| DS3002 | error | unknown file format |
| DS3003 | error | unknown reader option |
| DS3004 | error | glob matched no files |
| DS3005 | error | incompatible schemas across glob matches |
| DS4001 | error | function not supported by this engine |
| DS4002 | error | engine runtime error |
| DS5001 | info | chart not drawn in this runtime |
| DS5002 | error | chart needs a column of a kind the result lacks |
| DS9001 | error | internal compiler error |
