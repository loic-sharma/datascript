# M1-13 — M1 acceptance and polish

**Status:** todo
**Depends on:** M1-03, M1-10, M1-11, M1-12, M1-14, M1-18, M1-19, M1-20, M1-21, M1-22, M1-23, M1-24, M1-25
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
- [ ] (from M1-09) Report bad constant arguments at check time instead of `DS4004` at run time: a literal regex that does not compile (`matches regex`, `trim`, `extract`), an unknown literal part in `datetime_part`/`datetime_diff`, an unknown format specifier in `format_datetime`, a literal `percentile` outside 0–100. The checks exist in `datascript-datafusion` (`udf.rs`, `calendar.rs`, `udaf.rs`); move or share them with the binder (which may not depend on the `regex` crate's engine choice: D-014 already fixes Rust `regex` syntax).
- [ ] (from M1-09) Q-008 is still open: no Kusto cluster was available to check whether KQL errors on `long` overflow and integer division by zero. Check `print 9223372036854775807 + 1` and `print 1 / 0` in KQL, then mark LANGUAGE.md §4.1 ≠ KQL or close Q-008 in DECISIONS.md.
- [ ] (from M1-09) A malformed row in a query that reads several sources is `DS3008` "cannot read one of the query's sources" at the query's last node, because DataFusion's error does not name the file. Find the file (e.g. a per-scan wrapper around the `ListingTable` stream) so the error names it and points at its scan.
- [ ] (from M1-11) `ds run` collects each result fully before printing (`Session::run`). Large results should stream: print table rows as batches arrive and stop reading (which cancels the query) once `--max-rows` is reached, counting the rest only if cheap; write CSV/NDJSON/Arrow batch by batch. This needs a streaming variant of `Session::run` behind the same panic boundary.

## Follow-ups
