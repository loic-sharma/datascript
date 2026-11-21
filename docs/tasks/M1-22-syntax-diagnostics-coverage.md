# M1-22 — Syntax diagnostics: more habits and style fixes

**Status:** done — targeted diagnostics for SQL habits, a missing table, reserved names, trailing commas, `in [...]`, `in ~` and `1.2.3`; DS0004 from `ds run`; label, tab and long-line fixes
**Depends on:** M1-21, M1-19
**Crates:** `datascript-syntax`, `datascript-diagnostics`
**Diagnostic codes:** DS0102–DS0149 (D-018); also implements the registered DS0004 (owned by the
done task M0-02)

## Goal
Common mistakes get a targeted message instead of a bare DS0101 ("expected `|` or `;`, found …"),
and existing messages follow DIAGNOSTICS.md consistently. Found in a review of M1-03.

## Scope
Missing targeted diagnostics (each currently a bare DS0101 with no label text or help):
- SQL habits: `AND`/`OR`/`NOT` in caps, `a is null` / `is not null`, `name like 'a%'`,
  `project a as b`, `count(*)`, `project *`, SQL quote doubling `'it''s'`.
- `SELECT a, count(*) FROM t GROUP BY a` gets an unrelated generic example. Build the concrete
  rewrite (`t | summarize count() by a`) when possible.
- A query with no source (`where a > 1`) should say a query starts with a table.
- A reserved word used as a name (`extend null = 1`) should suggest `['null']`; only `let` has a
  case today.
- Trailing comma before a pipe (`project a, b, | take 5`), `a in [1,2]` (suggest parentheses),
  `1.2.3` (says "found `.3`").
- **DS0004 is never emitted.** An empty or comments-only program prints nothing and exits 0.
- "expected a table before `|`" is targeted text under the generic DS0101; give it its own code.

Style fixes:
- `Where` (title case) is reported as "SQL keyword" (DS0105) while `Take` gets DS0104
  "operator names are case-sensitive". Only all-caps should count as SQL (`targeted.rs`, SQL_WORDS).
- Empty primary labels (`.primary(span, "")`) for DS0111 `&&`/`||`, DS0104 did-you-mean and DS0105.
  Give them labels that add information (rule 2).
- Help text echoes raw tabs; expand them as the snippet does.
- Stale doc comment on `DS0105` in `codes.rs` versus the registry summary.
- Very long source lines (e.g. 300+ nested parens with DS0100) render untruncated. Truncate around
  the span.

## Acceptance criteria
- [x] A golden case for each item above, including an empty program and a comments-only program
      (DS0004).
- [x] New codes added to DIAGNOSTICS.md's registry and `codes.rs`, from this block only.
- [x] A golden case with tabs in the source and in help text.

## Carried over
- [x] (from M1-20) `x !in ~ ('a')` gives a bare DS0101 ("expected `(` ... found `~`") with no help. Recognize `!in ~` / `in ~` with a space and suggest `!in~` / `in~` (machine-applicable). Golden: `tests/cases/lexer/negated_in_tilde_with_space.dstest`.

## Follow-ups
Notes (no owner needed):
- New codes: DS0117 (SQL forms: `count(*)`, `project *`, `''` in a string), DS0125 (query with no
  table, including the old "expected a table before `|`"), DS0126 (reserved word as a name, with a
  machine-applicable quote fix; replaces the help on DS0101 for `let`), DS0127 (trailing comma),
  DS0128 (`in [...]`, also `in ['a']`, which lexes as a quoted name), DS0129 (`in ~` / `!in ~`).
  SQL keyword habits (`AND`/`OR`/`NOT`/`IN` in capitals, `IS [NOT] NULL`, `[NOT] LIKE`, `AS`,
  `NOT IN`) use DS0105, whose summary now lists them.
- `1.2.3` reuses the registered DS0003 (invalid number literal) rather than a new code: the
  parser emits it, because the lexer reads `1.2` and `.3` as two valid tokens.
- DS0004 is emitted by `Session::run` (engine), not the parser: LANGUAGE.md says a program with
  no query is valid for `ds check`. The harness rejects an empty `---- query` section, so the
  "empty program" golden is `;`; the engine unit test `run_warns_when_there_is_no_query` covers
  truly empty and whitespace-only text.
- The `SELECT` rewrite now handles `WHERE` (with `=`/`<>`/`AND`/`OR` translated), `GROUP BY`,
  `ORDER BY`, `LIMIT`, `x AS y` and `count(*)`; anything else (`HAVING`, `LIKE`, joins) falls
  back to the generic example.
- Only all-caps SQL words are DS0105. Title case (`Where`, `Select`) is DS0104 with the
  case-sensitive or alias help; title-case `And`/`Or` are not recognized as habits.
- Long source lines (over 120 columns) are cut to a window of about 100 columns around the
  labels, with `...` (`…` in the terminal) where text was left out.
