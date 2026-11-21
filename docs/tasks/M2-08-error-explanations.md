# M2-08 — Long-form error explanations

**Status:** todo
**Depends on:** M1-13
**Crates:** `datascript-diagnostics`, `datascript-cli`

## Goal
`docs/errors/DSxxxx.md` for every code (what it means, examples of wrong and right code), and
`ds explain-error DSxxxx`. Diagnostics link to the online page once docs are hosted (M6-01).

## Acceptance criteria
- [ ] A test that fails if any registered code lacks an explanation file.
- [ ] Each explanation's "wrong" example produces that code (checked automatically).

## Follow-ups
