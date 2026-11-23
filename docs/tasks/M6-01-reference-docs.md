# M6-01 — Language reference and docs site

**Status:** todo
**Depends on:** M2-08, M5-03 (embedded playground), M2-13. Not on all of M2, on purpose: the site
can start with the language as it is, and each later M2 task updates the reference for what it adds
(functions come from the catalog).
**Crates:** docs site

## Goal
A docs site (e.g. mdBook) with a tutorial, the language reference (operators from LANGUAGE.md,
functions generated from the catalog), the error index from M2-08, and an embedded playground (M5).

## Acceptance criteria
- [ ] Every code example in the docs is executed in CI.

## Carried over
- [ ] (from M1-13, maintainer only: needs a Kusto cluster) Q-008: check in a Kusto cluster whether KQL errors on `print
      9223372036854775807 + 1` and `print 1 / 0`. Then either mark LANGUAGE.md §4.1 ≠ KQL or
      close Q-008 in DECISIONS.md, and make the reference docs say which.
- [ ] (from M1-13, maintainer only: needs user feedback) Q-005: with the first user feedback, check that `['order id']` (a column) and
      `from 'orders.csv'` (a path) are not confused, then close Q-005 in DECISIONS.md.
- [ ] (from KQL review, 2026-11) Add a "Coming from KQL" page. Lead with the differences that
      change results without an error: `where x != v` drops null rows, `join` defaults to `inner`
      (not `innerunique`), and `tostring(null)` and `extract()` with no match give null instead of
      `""`. Then the ones that give an error with a fix (`from` at a query head, `sort` direction,
      `let` shadowing, dropped aliases). Build it from the `≠ KQL` marks in LANGUAGE.md (M6-04).

## Follow-ups
