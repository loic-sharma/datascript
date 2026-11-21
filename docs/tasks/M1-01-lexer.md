# M1-01 — Lexer

**Status:** todo
**Depends on:** M0-02, M0-03
**Crates:** `datascript-syntax`
**Diagnostic codes:** DS0001–DS0019 (D-018)

## Goal
Turn source text into tokens (including trivia) for the parser, with good lexical diagnostics.

## Scope
- Token kinds as a `SyntaxKind` enum shared with the rowan tree (M1-02): punctuation, operators
  (`==`, `!=`, `=~`, `!~`, `<=`, …), keywords, identifiers, quoted identifiers `['x']`/`["x"]`,
  literals (long, real, string with escapes, verbatim `@"..."`, timespan suffixes, `datetime(...)`
  and `timespan(...)` bodies lexed as a single literal token when the name is directly followed by
  `(`), comments (`//`, `/* */`), whitespace.
- The lexer is **context-free** (D-026): it never looks at the previous token. `project-away` lexes
  as `project` `-` `away`; the parser joins them (M1-02).
- Lexer never fails. Bad input becomes an `ERROR` token plus a diagnostic (DS0001–DS0003).
- Literal value parsing (number ranges, timespan units, ISO datetimes) with precise spans for errors.

Design-review additions:
- Keywords are contextual (LANGUAGE.md §1.1): emit `IDENT` for everything except the reserved
  `let`, `true`, `false`, `null`, `and`, `or`, `not`.
- Tokens `..`, `$left`, `$right`, `<>` (lexed so M1-03 can give DS0107), `&&`, `||`, `!`, `~`.
- `!` forms per LANGUAGE.md §1: read the whole identifier after `!`, and emit a negated-operator
  token only for the listed names (`!has`, `!in`, `!contains_cs`, …), so `!has_error` is `!` then
  an identifier. `in~` / `!in~` likewise.
- Numbers: `1..10` is `1` `..` `10`; `1.` is not a real.
- No negative number literals: `-` is always its own token.
- String escapes per LANGUAGE.md §1; unknown escapes are DS0005. Unterminated block comment is DS0006.

## Acceptance criteria
- [ ] Every lexical form in LANGUAGE.md §1 has a unit test.
- [ ] Unterminated string: diagnostic spans from the opening quote to end of line, with help on
      closing it.
- [ ] `1x` (bad timespan unit) names the valid units.
- [ ] Round trip: concatenating token texts reproduces the input exactly (property test with `proptest`).
- [ ] Golden cases under `tests/cases/lexer/` (mode `syntax`).
- [ ] `'C:\data\x.csv'` produces DS0005 with help suggesting `@'...'` or `/`.
- [ ] Unit tests for `1..10`, `datetime(2024-01-31)`, `datetime(null)`, `!has`, `!has_error`, `in~`,
      and `project-away` lexing as three tokens.

## Follow-ups
