# M1-24 — Require `from` at the start of a query

**Status:** in-progress
**Depends on:** M1-22
**Crates:** `datascript-syntax`, `datascript-diagnostics`, `datascript-compiler` (only where the
AST shape changes), `datascript-tests`
**Diagnostic codes:** DS0150–DS0151 (D-018)

## Goal
A query reads its table with `from`, as in PRQL and SQL, instead of starting with a bare expression:

```ds
// Before
'../tests/data/orders.csv' | where amount > 100
my_table | take 5

// After
from '../tests/data/orders.csv' | where amount > 100
from my_table | take 5
```

Today any expression can start a query (D-029), and the binder decides afterwards whether it was a
table (D-020). With `from`, the parser knows where a table starts, so:
- a `let`'s kind (scalar or table) can be read from the syntax alone;
- the missing-`;` fix (DS0106) and recovery from unclosed brackets can treat a `from` keyword after a
  complete statement as a certain sign of a new statement, instead of guessing from line breaks and
  indentation (M1-25);
- completion after `from ` can offer only tables and paths (M3-02, M4-04);
- the SQL habit `from` is correct, not something DS0104 has to correct.

The design was settled with the maintainer; see Q-009 in [DECISIONS.md](../DECISIONS.md). Record it
as a decision (D-036) when this task lands.

**Split.** This task changes the grammar, makes `from` required with a plain missing-`from` error,
migrates the tests and updates the docs. That is what the binder (M1-05) needs. The targeted
diagnostics for other `from` mistakes, and the statement-start rule for DS0106 and recovery, are
[M1-25](M1-25-from-diagnostics.md). Until M1-25 lands, those mistakes get generic errors, and DS0106
keeps today's layout rule (a `from` first on its line, outside brackets, already counts as a new
statement under it).

## Design
**Where `from` is required.** Every pipeline head: a query statement, the value of a table `let`,
a parenthesized pipeline, and a pipeline passed as a call argument.

```ds
let big = from 'orders.csv' | where amount > 100;
from big | summarize count() by customerId;
from read_csv('x.txt', delimiter = ';') | take 5;
let p = 'data/orders.csv';         // scalar string, unchanged
from p | take 5                    // constant string after `from` is a path (D-020 still holds)
```

**What follows `from`.** A path string, a name (a table `let`, a host table, or a scalar `let` bound to
a constant string), a reader call, or a parenthesized pipeline. As today, the parser accepts any
expression there and the binder rejects the ones that are not tables (D-029).

**`from x` is always a pipeline.** Today `pipeline_or_expr` creates a `PIPELINE` node only when a `|`
follows. From now on `from x` produces a `PIPELINE` node, with zero operators if none follow, in
every position: query statements, `let` values, parentheses and call arguments. This is what makes
a `let`'s kind syntactic (below). Inside the `PIPELINE`, the `from` token and the source go in a
`FROM_CLAUSE` node (or the `from` token is a direct child of `PIPELINE`; pick one and record it in
D-036). Either way, update `Pipeline::source()`, which today is `child::<Expr>` and would silently
return `None` if the source moved into a child node, and add `Pipeline::from_token()`.

**Where `from` is not written.**
- The right side of `join`, `union` and `lookup` is already a table position: `join customers on id`,
  `union 'b.csv'`. A pipeline there is parenthesized and starts with `from`:
  `join (from 'c.csv' | where active) on id`. (`join from customers on id` gets a targeted error in
  M1-25; a generic one is fine here.)
- Row generators start a query on their own because they read nothing: `range`, `datatable`,
  `print` (M2-01). Make the head check accept them by name, so M2-01 only adds their grammar.
  Until M2-01, a generator at a query head gets the "not available yet" error that later operators
  get (`print` is already in `LATER_OPERATORS`; add `range` and `datatable` to the head check), not
  the missing-`from` error.
- A parenthesized pipeline can start a query without another `from`, since it already starts with
  one: `(from t | where a > 1) | take 5`. `from (from t | where a > 1)` is also valid.

**Strict.** A bare source (`orders | take 5`) is an error, not a warning. Keeping both forms would
keep all of the ambiguity this task removes.

**`from` stays contextual** (D-013). It is a keyword at a pipeline head when an expression start
follows it, and an ordinary name everywhere else (`| where from > 3` still works). Keep it an
`IDENT` token in the tree (D-029). The edge cases:
- `-` and `not` after `from` do not make it a keyword, so `let y = from - 1` (a scalar `let` named
  `from`) stays a scalar. Nobody writes a table source that starts with `-` or `not`.
- `from(` is the keyword followed by a parenthesized source, never a call to a function named
  `from` (there is none).
- A table named `from` is `from ['from']`. `from from` also works under this rule (the second
  `from` is a name); say so in LANGUAGE.md §1.1.
- The match is case-sensitive like every contextual keyword (`at_kw`). `FROM` in capitals is not
  accepted (M1-25 reports it as SQL).

**Table `let`s.** A `let` whose value *is* a `PIPELINE` (it starts with `from`, a generator, or is a
parenthesized pipeline) is a table; anything else is a scalar. "Is", not "starts with":
`let x = (from t | count) + 1` is a scalar expression, and the binder rejects it. This replaces
D-020's "a tabular name on the right side makes the `let` tabular": `let alias = orders;`, where
`orders` is a table, is no longer a table `let`. The binder should report it with a hint to write
`let alias = from orders;` (a binder code from M1-05's block, carried over to M1-05, not from this
task's block).

**Reader names in the syntax crate.** The missing-`from` diagnostic must tell a reader call
(`read_csv('x') | take 5`, `let r = read_csv('x');`) from any other call. The catalog lives in
`datascript-compiler`, above `datascript-syntax`, so add a small `READERS` constant
(`read_csv`, `read_parquet`, `read_json`) in the syntax crate, and a unit test in the compiler
that checks it matches the catalog's readers.

## Scope
- **Parser** (`grammar.rs`): `query_stmt`, `pipeline_or_expr` (`let` values, parentheses, call
  arguments), `right_table`, and the head check for generators.
- **Diagnostics** (`targeted.rs`), each with golden cases:
  - **DS0150, missing `from`.** One error per pipeline, with a fix that inserts `from `:
    - A query statement that starts with a string, a name or a call followed by `|`, or with a
      string or a reader call on its own: `MachineApplicable`. A bare name on its own (`orders`)
      gets the fix as `MaybeIncorrect`, since it may be a scalar typed by mistake.
    - A `let` value that is `name | …`, `'str' | …`, `call(…) | …`, or a reader call on its own
      (`let r = read_csv('x');`): `MachineApplicable`. **Never** a bare string or a bare name on
      its own: `let p = 'data/orders.csv';` and `let n = other;` are scalars (D-020) and get no
      error from the parser.
    - A pipeline in parentheses or in a call argument without `from`: `(t | where a)`,
      `f(t | count)`. Same fix. The node is still a `PIPELINE` so nothing cascades.
  - **DS0151, `from` with nothing after it**: `from | take 5`, `from;`, `from` at the end.
- **Existing diagnostics whose text or fix would now be wrong.** Change them here, because a fix
  that produces a bare source no longer parses (D-035):
  - DS0104's `from` alias and `FROM_HELP` (`targeted.rs`) go away: `from` is now right. The
    `"from" => d.help(FROM_HELP)` arm of DS0105 (`| FROM` after a pipe) loses its help text too;
    M1-25 gives both cases a targeted diagnostic.
  - DS0105's `SELECT ... FROM` rewrite produces `from t | project a` (`sql_rewrite` starts from
    `from <table>`), and the fallback help in `sql_select` (`'orders.csv' | where amount > 10 | …`)
    starts with `from`.
  - DS0125: the help in `operator_without_source` (`'data.csv' | {query}`) and the `|`-at-start
    help in `query_stmt` become "start the query with `from 'data.csv'`…".
- **AST** (`ast.rs`): `Pipeline::source()` keeps returning the source expression; add
  `Pipeline::from_token()` for spans. Update the doc comments on `LetStmt`, `LET_STMT` and
  `PIPELINE` that say the binder decides a `let`'s kind. Update the existing small `binder.rs` and
  the walking skeleton to match.
- **Tests.** About 211 `.dstest` files contain a query. Work in this order, one commit each, so that
  every commit passes the tests and the migration diff contains nothing else:
  1. **Accept `from`.** The parser accepts `from <source>` (with the `PIPELINE` shape above) while
     still accepting bare sources. Add the new golden cases for the `from` forms.
  2. **Migrate.** Run a one-off script over the existing cases, then rebless with
     `DS_BLESS=1 cargo test -p datascript-tests`. With `from ` in front, every column in every
     error moves; in this commit only spans and trees may change, never which errors are reported.
  3. **Require `from`.** Bare sources become DS0150, plus DS0151 and the help text changes above.

  The migration script must:
  - insert `from ` at the head of each query statement, after a leading BOM if there is one (one
    case starts with `\u{FEFF}t | take`);
  - skip cases whose head is deliberately wrong: the DS0125 cases (`| take 5`, `where amount …`),
    the `SELECT` cases (`SELECT …`, `| SELECT *`), and the punctuation case (`| ; , ( ) [ …`);
  - handle `let` values and parenthesized pipelines, not only the first line.

  The `datetime(…)` lexer cases would become `from datetime(…)`, which parses but reads oddly; move
  the literal into `| where` or `| extend` instead.
- **Docs**, in the same change:
  - LANGUAGE.md §1.1 (contextual keywords, `from from`), §2 (statements, `let` kinds, the `path`
    example), §3, §3.1 (sources table and the "constant string at the start of a pipe" paragraph),
    §8 grammar.
  - README.md and `examples/*.ds`.
  - DECISIONS.md: add D-036 (including the node shape and the `-`/`not`/`from(` rules), and note at
    D-006, D-020 and D-029 which parts D-036 changes. Remove Q-009 from the open questions.
  - DIAGNOSTICS.md registry entries for DS0150 and DS0151, and the changed DS0104, DS0105 and
    DS0125 summaries.

## Acceptance criteria
- [ ] Every example in LANGUAGE.md, README.md and `examples/` starts its queries with `from` (or a
      generator) and passes `ds check`.
- [ ] Golden cases for DS0150 (each form above) and DS0151, each with its fix, checked by
      `mode: syntax` (the fix applied parses).
- [ ] Golden cases for: `from` with each source form (path, glob, name, scalar `let` path, reader
      call, parenthesized pipeline); `from x` alone as a query, a `let` value, in parentheses and as
      a call argument (each a `PIPELINE` in the tree); a table `let`; a scalar `let` bound to a
      string and to a bare name (no error); `join`/`union` with a bare table and with a
      parenthesized `from` pipeline; a column named `from`; `from ['from']`; `from from`;
      `let y = from - 1`; a generator at a head before M2-01.
- [ ] DS0104 no longer mentions `from`; the DS0105 rewrite and every help that quotes a query
      produce valid `from` syntax.
- [ ] Accepting `from`, migrating the `.dstest` files, and requiring `from` are separate commits;
      the migration's blessed diff changes no diagnostic code or message.
- [ ] The syntax crate's `READERS` list is checked against the catalog by a test.
- [ ] Core crates still build for `wasm32-unknown-unknown`.
- [ ] Downstream tasks updated with a "Carried over" item each: M1-05 (table `let` kind is syntactic:
      the value is a `PIPELINE`; hint for `let alias = orders`), M1-17 (table positions are the
      `from` operand and the right side of `join`/`union`/`lookup`), M2-01 (generators start a
      query without `from`), M2-07 (formatter keeps `from <source>` on the first line), M3-02
      (completion after `from`).

## Follow-ups
