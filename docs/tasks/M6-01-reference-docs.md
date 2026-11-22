# M6-01 — Language reference and docs site

**Status:** todo
**Depends on:** M2-08, M5-03 (embedded playground)
**Crates:** docs site

## Goal
A docs site (e.g. mdBook) with a tutorial, the language reference (operators from LANGUAGE.md,
functions generated from the catalog), the error index from M2-08, and an embedded playground (M5).

## Acceptance criteria
- [ ] Every code example in the docs is executed in CI.

## Carried over
- [ ] (from M1-13) Q-008: check in a Kusto cluster whether KQL errors on `print
      9223372036854775807 + 1` and `print 1 / 0`. Then either mark LANGUAGE.md §4.1 ≠ KQL or
      close Q-008 in DECISIONS.md, and make the reference docs say which.
- [ ] (from M1-13) Q-005: with the first user feedback, check that `['order id']` (a column) and
      `'orders.csv'` (a path) are not confused, then close Q-005 in DECISIONS.md.

## Follow-ups
