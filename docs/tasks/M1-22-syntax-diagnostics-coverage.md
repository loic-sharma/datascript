# M1-22 — Syntax diagnostics: more habits and style fixes

**Status:** todo
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
- [ ] A golden case for each item above, including an empty program and a comments-only program
      (DS0004).
- [ ] New codes added to DIAGNOSTICS.md's registry and `codes.rs`, from this block only.
- [ ] A golden case with tabs in the source and in help text.

## Carried over

## Follow-ups
