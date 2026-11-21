# M1-01 — Lexer

**Status:** todo
**Depends on:** M0-02, M0-03
**Crates:** `datascript-syntax`

## Goal
Turn source text into tokens (including trivia) for the parser, with good lexical diagnostics.

## Scope
- Token kinds as a `SyntaxKind` enum shared with the rowan tree (M1-02): punctuation, operators
  (`==`, `!=`, `=~`, `!~`, `<=`, …), keywords, identifiers, quoted identifiers `['x']`/`["x"]`,
  literals (long, real, string with escapes, verbatim `@"..."`, timespan suffixes, `datetime(...)`
  and `timespan(...)` bodies lexed as a single literal token), comments (`//`, `/* */`), whitespace.
- Hyphenated operator names (`project-away`, `project-rename`, `mv-expand`, `make-series`,
  `project-reorder`) lexed as one token **only immediately after `|`** (see LANGUAGE.md §8).
- Lexer never fails. Bad input becomes an `ERROR` token plus a diagnostic (DS0001–DS0003).
- Literal value parsing (number ranges, timespan units, ISO datetimes) with precise spans for errors.

## Acceptance criteria
- [ ] Every lexical form in LANGUAGE.md §1 has a unit test.
- [ ] Unterminated string: diagnostic spans from the opening quote to end of line, with help on
      closing it.
- [ ] `1x` (bad timespan unit) names the valid units.
- [ ] Round trip: concatenating token texts reproduces the input exactly (property test with `proptest`).
- [ ] Golden cases under `tests/cases/lexer/` (mode `syntax`).

## Follow-ups
