# M1-24 — Require `from` at the start of a query

**Status:** todo
**Depends on:** M1-22
**Crates:** `datascript-syntax`, `datascript-diagnostics`, `datascript-compiler` (only where the
AST shape changes), `datascript-tests`
**Diagnostic codes:** DS0150–DS0159 (D-018)

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
- the missing-`;` fix (DS0106) and recovery from unclosed brackets can use `from` at the start of a
  line as a clear sign of a new statement, instead of guessing from indentation;
- completion after `from ` can offer only tables and paths (M3-02, M4-04);
- the SQL habit `from` is correct, not something DS0104 has to correct.

The design was settled with the maintainer; see Q-009 in [DECISIONS.md](../DECISIONS.md). Record it
as a decision (D-036) when this task lands.

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

**Where `from` is not written.**
- The right side of `join`, `union` and `lookup` is already a table position: `join customers on id`,
  `union 'b.csv'`. A pipeline there is parenthesized and starts with `from`:
  `join (from 'c.csv' | where active) on id`. `join from customers on id` is an error with a
  machine-applicable fix that removes `from`.
- Row generators start a query on their own because they read nothing: `range`, `datatable`,
  `print` (M2-01). Make the head check accept them by name, so M2-01 only adds their grammar.
- A parenthesized pipeline can start a query without another `from`, since it already starts with
  one: `(from t | where a > 1) | take 5`. `from (from t | where a > 1)` is also valid.

**Strict.** A bare source (`orders | take 5`) is an error, not a warning. Keeping both forms would
keep all of the ambiguity this task removes.

**`from` stays contextual** (D-013): it is a keyword at a pipeline head when an expression start
follows it, and an ordinary name everywhere else (`| where from > 3` still works). A table named
`from` is `from ['from']`. Keep it an `IDENT` token in the tree (D-029); add a node (for example
`FROM_CLAUSE` or a `from` token child of `PIPELINE`) so the AST can say where the source is.

**Table `let`s.** A `let` whose value starts with `from`, a generator, or a parenthesized pipeline is
a table; anything else is a scalar. This replaces D-020's "a tabular name on the right side makes
the `let` tabular": `let alias = orders;`, where `orders` is a table, is no longer a table `let`.
The binder should report it with a hint to write `let alias = from orders;` (a binder code from
M1-05's block, carried over to M1-05, not from this task's block).

## Scope
- **Parser** (`grammar.rs`): `query_stmt`, `pipeline_or_expr` (`let` values, parentheses, call
  arguments), `right_table`, `at_new_statement` and `statement_after_unclosed_at` (a `from` at the
  start of a line starts a statement), and DS0106.
- **Targeted diagnostics** (`targeted.rs`), each with golden cases:
  - Missing `from`: a query or table `let` that starts with a string, a name or a reader call
    followed by `|`, or a string or reader call on its own. Machine-applicable fix inserting
    `from `. A bare name on its own (`orders`) gets the fix as `MaybeIncorrect`, since it may be a
    scalar typed by mistake. One error per statement.
  - `from` with nothing after it (`from | take 5`, `from;`).
  - `from` after a pipe (`t | from x`) and on the right of `join`/`union`/`lookup`, with a fix that
    removes it.
  - SQL order `from t select a` or `from t where x` without a pipe: reuse DS0102 (missing pipe) if it
    already covers this; otherwise a code from this block.
- **Existing diagnostics that change:**
  - DS0104's `from` alias and `FROM_HELP` (`targeted.rs`) go away: `from` is now right.
  - DS0105's `SELECT ... FROM` rewrite produces `from t | project a` (`targeted.rs`, `sql_select`).
  - DS0125's help text ("start the query with `from 'data.csv'`…").
  - DS0106 tests and DS0112 recovery tests that rely on the "expression at the start of a line"
    rule. Where `from` makes a fix certain, make it `MachineApplicable` (D-035).
- **AST** (`ast.rs`): `Pipeline::source()` keeps returning the source expression; expose the `from`
  token for spans. Update the existing small `binder.rs` and the walking skeleton to match.
- **Tests:** about 209 `.dstest` files contain a query. Add `from` with a script, rebless with
  `DS_BLESS=1 cargo test -p datascript-tests`, and review the diff: only spans and trees should
  move, never which errors are reported, except where this task changes them on purpose.
- **Docs**, in the same change:
  - LANGUAGE.md §1.1 (contextual keywords), §2 (statements, `let` kinds, the `path` example), §3,
    §3.1 (sources table and the "constant string at the start of a pipe" paragraph), §8 grammar.
  - README.md and `examples/*.ds`.
  - DECISIONS.md: add D-036, and note at D-006, D-020 and D-029 which parts D-036 changes. Remove
    Q-009 from the open questions.
  - DIAGNOSTICS.md registry entries for the new codes.

## Acceptance criteria
- [ ] Every example in LANGUAGE.md, README.md and `examples/` starts its queries with `from` (or a
      generator) and passes `ds check`.
- [ ] Golden cases for each new diagnostic, each with its fix, checked by `mode: syntax` (the fix
      applied parses).
- [ ] Golden cases for: `from` with each source form (path, glob, name, scalar `let` path, reader
      call, parenthesized pipeline); a table `let`; `join`/`union` with a bare table and with a
      parenthesized `from` pipeline; a column named `from`; `from ['from']`.
- [ ] A missing `;` before a line starting with `from` gets a machine-applicable DS0106 fix.
- [ ] DS0104 no longer mentions `from`; the DS0105 rewrite produces valid `from` syntax.
- [ ] Core crates still build for `wasm32-unknown-unknown`.
- [ ] Downstream tasks updated with a "Carried over" item each: M1-05 (table `let` kind is syntactic;
      hint for `let alias = orders`), M1-17 (table positions are the `from` operand and the right
      side of `join`/`union`/`lookup`), M2-01 (generators start a query without `from`), M2-07
      (formatter keeps `from <source>` on the first line), M3-02 (completion after `from`).

## Follow-ups
