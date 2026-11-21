# M1-03 — Syntax diagnostics quality pass

**Status:** done — targeted DS0102–DS0124 diagnostics (forgotten/extra pipes, unknown operators with did-you-mean and foreign aliases, SQL habits, `=`/`<>`/`&&`/`||`/`!`, brackets, `render` placement, missing `by`/`on`, join kinds) with machine-applicable fixes that `mode: syntax` applies and re-parses; 44 new golden cases.
**Depends on:** M1-02
**Crates:** `datascript-syntax`
**Diagnostic codes:** DS0102–DS0149 (D-018)

## Goal
Replace generic parse errors with targeted, helpful ones. This task is all about error quality.

## Scope
Implement and golden-test DS0102–DS0120 from [DIAGNOSTICS.md](../DIAGNOSTICS.md), including:
- Missing `|` between operators (`where x > 1 project a`), with a suggestion to insert it.
- Trailing `|` with nothing after it.
- Unknown operator, with did-you-mean (`summerize`, `select`, `filter` → `where`, `orderby`), and
  `project - away` with spaces suggesting `project-away` (D-026).
- SQL habits: `SELECT ... FROM`, `GROUP BY`, `ORDER BY`, `LIMIT` as an operator name (we do accept
  `limit`), `WHERE` in caps (case-sensitive keywords), `<>` → `!=`.
- `=` used as comparison in `where`/`iff`/`join on`, with a machine-applicable fix.
- `&&`, `||`, `!x` → `and`, `or`, `not(x)`, including `!has_error` (LANGUAGE.md §1).
- `render` not last.
- Unbalanced parentheses: point at the opening one.
- Missing `by` in `sort`/`top`, missing `on` in `join`, `kind=innerunique` (D-004 explanation).

Design-review additions: DS0106 missing `;` between statements (machine-applicable insert), DS0107
`<>`, DS0112 unbalanced brackets, DS0121 missing `by`, DS0122 missing `on`, DS0123 unknown join
kind, DS0124 `innerunique`. All codes are already in the registry.

## Acceptance criteria
- [x] Each diagnostic has at least one golden case with the full rendered output.
      (`tests/cases/errors/syntax/`, 44 cases, plus the re-blessed `tests/cases/syntax/` ones.)
- [x] Machine-applicable suggestions are tested by applying them and re-parsing without errors
      (a harness helper `apply_suggestions`). `datascript_tests::apply_suggestions` (with unit
      tests); `mode: syntax` prints the fixed query and the result of re-parsing it, so every
      case with a fix shows `which parses without errors` (TESTING.md).
- [x] A review pass over all syntax golden outputs against the style rules in DIAGNOSTICS.md.
      Every message is lowercase-first with no trailing period; labels add to the message.

## Notes
- Design choices are recorded in D-030: habit tokens (`&&`, `||`, `<>`, `=` in predicates, `!x`)
  parse as the operator meant and the AST reads them that way; a certain fix for an unknown
  operator name parses the rest as that operator; `||` before an operator name is two pipes;
  DS0112 only when no closing bracket follows in the statement.
- `=` is a comparison (DS0110) in `where`, `join ... on`, and the arguments of `iff`, `iif`,
  `case`, `countif` and `sumif`. Inside other calls `name = value` stays a named argument.
- Only DS0102–DS0124 (the registered codes) were needed; DS0108–DS0109, DS0113–DS0119 and
  DS0125–DS0149 stay free in this block.
- The parser fuzz fragments now include the habit tokens; run once with 100k cases per property.
  The 10k-line benchmark is unchanged (7.0 ms).

## Follow-ups
- Generic DS0101 remains for chained comparisons (`a < b < c`), missing expressions and token
  soups. Chained comparisons could move to a code from this block if they need one.
- Uppercase `NULLS FIRST` / `NULLS LAST` and SQL's `LEFT JOIN` written as an operator have no
  targeted help yet.
