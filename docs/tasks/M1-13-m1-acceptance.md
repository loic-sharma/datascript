# M1-13 — M1 acceptance and polish

**Status:** todo
**Depends on:** M1-03, M1-10, M1-11, M1-12, M1-14
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

## Follow-ups
