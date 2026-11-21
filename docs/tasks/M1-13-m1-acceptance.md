# M1-13 — M1 acceptance and polish

**Status:** todo
**Depends on:** M1-03, M1-10, M1-11, M1-12
**Crates:** all

## Goal
Close out M1 against the "done when" list in [ROADMAP.md](../ROADMAP.md).

## Scope
- The README example runs end to end as a golden test and as `examples/top_customers.ds`.
- Audit: every ● operator and M1 function has happy-path and error cases. Fill the gaps.
- Error-quality review: read every golden diagnostic against DIAGNOSTICS.md style rules. Fix
  messages, re-bless, and note recurring patterns in DIAGNOSTICS.md.
- Resolve or explicitly defer open questions touching M1 (Q-001, Q-005).
- Update README status and a short "Getting started" section.

## Acceptance criteria
- [ ] All ROADMAP M1 criteria checked off.
- [ ] No `skip:` golden cases without a linked task.

## Follow-ups
