# M2-08 — Long-form error explanations

**Status:** todo
**Depends on:** M1-13, M2-13
**Crates:** `datascript-diagnostics`, `datascript-cli`

## Goal
`docs/errors/DSxxxx.md` for every code (what it means, examples of wrong and right code), and
`ds explain-error DSxxxx`. Diagnostics link to the online page once docs are hosted (M6-01).

## Acceptance criteria
- [ ] A test that fails if any registered code lacks an explanation file.
- [ ] Each explanation's "wrong" example produces that code (checked automatically).

## Carried over
- [ ] (from planning, 2026-11-22) If M3-01 is done, switch the LSP's code description links from the DIAGNOSTICS.md registry to the new explanation pages (one function: `datascript_diagnostics::codes::docs_url`), and use the same links in the web UI if M4-02 shows them.

## Follow-ups
