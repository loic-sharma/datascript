# M1-13 — M1 acceptance and polish

**Status:** todo
**Depends on:** M1-03, M1-10, M1-11, M1-12, M1-14, M1-18, M1-19, M1-20, M1-21, M1-22, M1-23, M1-24
**Crates:** all

## Goal
Close out M1 against the "done when" list in [ROADMAP.md](../ROADMAP.md).

## Scope
- The README example runs end to end as a golden test and as `examples/top_customers.ds`, with one
  `customerId` column and no DS1007 (D-019).
- Audit: every ● operator and M1 function has happy-path and error cases. Fill the gaps.
- Error-quality review: read every golden diagnostic against DIAGNOSTICS.md style rules. Fix
  messages, re-bless, and note recurring patterns in DIAGNOSTICS.md.
- Resolve or explicitly defer open questions touching M1 (Q-005, Q-008). Q-001 was closed by D-005.
- Update README status and a short "Getting started" section.

## Acceptance criteria
- [ ] All ROADMAP M1 criteria checked off.
- [ ] No `skip:` golden cases without a linked task.

## Carried over
- [ ] (from M1-03) Targeted help for uppercase `NULLS FIRST` / `NULLS LAST` and for SQL's `LEFT JOIN` written as an operator.
- [ ] (from M0-02) Carets count characters, so lines with CJK, emoji or combining marks are misaligned in a terminal; use `unicode-width` (and grapheme clusters).

## Follow-ups
