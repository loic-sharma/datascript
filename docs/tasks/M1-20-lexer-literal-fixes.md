# M1-20 — Lexer and literal fixes

**Status:** todo
**Depends on:** M1-03
**Crates:** `datascript-syntax` (`lexer.rs`, `literal.rs`)
**Diagnostic codes:** DS0001–DS0019 (D-018)

## Goal
Fix edge-case bugs and unsafe suggestions in the lexer and literal parsing, found in a review of
M1-01/M1-03.

## Scope
Bugs:
- **Span splits a UTF-8 character.** `datetime(2024-é1-01)` gives a DS0003 with span 14..15,
  ending inside `é`. `DateTimeParser::number` uses `.max(start + 1)` without
  `ceil_char_boundary_compat`, which `expect()` already uses.
- **Tiny timespans reported as too large.** `0.0000000000000000000000000000000000000001s` says
  "timespan is too large": `10i128.checked_pow(scale)` fails past 38 fractional digits. Drop digits
  beyond nanosecond precision before parsing.
- **Decomposed accents split a word.** `nai\u{0308}ve` (i + combining diaeresis) lexes as `nai`,
  ERROR, `ve`. `is_word_char` should accept combining marks (Mn/Mc) after the first character, so
  LANGUAGE.md §1's single DS0001 suggesting `['naïve']` applies.
- **Wrong field blamed.** `datetime(2024-01-0é)` reports "January has 31 days" on `0`. Check for a
  non-digit right after a numeric field before its range check.

Suggestions marked `MachineApplicable` that produce broken code:
- **Backtick names.** `` `a\b` `` suggests `['a\b']` (a DS0005), and `` `it's "x"` `` suggests
  `["it's "x""]`. Escape `\` and the chosen quote, or downgrade to `MaybeIncorrect`.
- **Unterminated string ending in `\`.** For `'abc\`, inserting `'` gives `'abc\'`, still
  unterminated. Handle an odd number of trailing backslashes.
- **Windows paths.** `'C:\data\new.csv'` gets "double it: `'C:\\data\new.csv'`", which leaves
  `\n` as a newline; the path branch is skipped when `has_valid_escape`. Whenever the text
  contains `:\` (or starts with `\`), suggest `@'…'` or forward slashes.

Spec gaps (decide, implement, record in LANGUAGE.md):
- `5μs` with Greek mu (U+03BC) says "unknown timespan unit" with no hint. Accept it or hint at `µs`/`us`.
- `timespan(-90m)` is rejected; only the clock form takes `-`.
- `['']` is accepted with no diagnostic. Decide whether the lexer or the binder rejects it.
- `0x1F` says "unknown timespan unit `x1F`". Hint that hex literals are not supported.

Nits:
- Diagnostics within one string are not in source order: for `'\q\u{zz}'` the `\u` error comes
  first, breaking the promise on `Lexed`.
- `literal::string("é", …)` panics (assumes an ASCII first char). Guard it or document `# Panics`.
- Dead code: "strings cannot span lines" branch in `literal.rs`, the `'` branch in `bad_name`.
- `lex` casts offsets with `as u32`, which wraps past 4 GiB. Assert or cap input size
  (coordinate with whatever limit M1-18 adds).

## Acceptance criteria
- [ ] Golden cases for every bug and suggestion above; each fix is applied and re-parsed by
      `mode: syntax`.
- [ ] A golden case with CRLF line endings covering an unterminated string, a line comment and an
      unclosed `datetime(`.
- [ ] Golden cases for: unclosed `datetime(`/`timespan(`, `1e3s`, `1_`, `$foo`, a lone `&`, `@x`,
      a BOM or no-break space, U+200B, typographic quotes, `timespan(1.25:00)`, `timespan(:)`,
      `['a\'b']`, `[ 'x' ]`, `!in ~`.
- [ ] `tests/lexer_props.rs` checks secondary spans and suggestion edit spans for char
      boundaries too, and its fragments include `datetime(2024-` + a multi-byte character.

## Carried over

## Follow-ups
