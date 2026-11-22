# M1-13 — M1 acceptance and polish

**Status:** done — the README example is a golden `run` case; every ● operator and M1 function
has happy-path and error cases; the diagnostics review fixed misleading conversion helps, broken
signature notes and `DS9001` for bad `take` counts (now `DS2050`); constant arguments are checked
at check time; carets and tables use terminal widths; Q-005 and Q-008 are deferred to M6-01.
**Depends on:** M1-03, M1-10, M1-11, M1-12, M1-14, M1-18, M1-19, M1-20, M1-21, M1-22, M1-23, M1-24, M1-25
**Crates:** all
**Diagnostic codes:** DS2050–DS2059 (reserved by this task, D-018)

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
- [x] All ROADMAP M1 criteria checked off.
- [x] No `skip:` golden cases without a linked task (there are none).

## Carried over
- [x] (from M1-03) Targeted help for uppercase `NULLS FIRST` / `NULLS LAST` and for SQL's `LEFT JOIN` written as an operator.
- [x] (from M0-02) Carets count characters, so lines with CJK, emoji or combining marks are misaligned in a terminal; use `unicode-width` (and grapheme clusters).
- [x] (from M1-09) Report bad constant arguments at check time instead of `DS4004` at run time: a literal regex that does not compile (`matches regex`, `trim`, `extract`), an unknown literal part in `datetime_part`/`datetime_diff`, an unknown format specifier in `format_datetime`, a literal `percentile` outside 0–100. The checks exist in `datascript-datafusion` (`udf.rs`, `calendar.rs`, `udaf.rs`); move or share them with the binder (which may not depend on the `regex` crate's engine choice: D-014 already fixes Rust `regex` syntax).
- [x] (from M1-09) Q-008 is still open: no Kusto cluster was available to check whether KQL errors on `long` overflow and integer division by zero. Check `print 9223372036854775807 + 1` and `print 1 / 0` in KQL, then mark LANGUAGE.md §4.1 ≠ KQL or close Q-008 in DECISIONS.md.
- [x] (from M1-09) A malformed row in a query that reads several sources is `DS3008` "cannot read one of the query's sources" at the query's last node, because DataFusion's error does not name the file. Find the file (e.g. a per-scan wrapper around the `ListingTable` stream) so the error names it and points at its scan.
- [x] (from M1-11) `ds run` collects each result fully before printing (`Session::run`). Large results should stream: print table rows as batches arrive and stop reading (which cancels the query) once `--max-rows` is reached, counting the rest only if cheap; write CSV/NDJSON/Arrow batch by batch. This needs a streaming variant of `Session::run` behind the same panic boundary.
- [x] (from M1-12) In the REPL, a table cut at `--max-rows` ends with "… N more rows (show them with --max-rows N)", but `--max-rows` is a startup flag the user cannot change mid-session. Reword the footer for the REPL, or add a `.max-rows <n>` meta command (`output::write_repl_results`, `crates/datascript-cli/src/repl/`).

## Follow-ups
- Done: `DS0105` for `LEFT JOIN`, `FULL OUTER JOIN`, `LEFT ANTI JOIN`, … (fix to `join kind = …`;
  `CROSS JOIN` says there is none) and for `NULLS FIRST`/`NULLS LAST` in capitals.
- Done: carets and table cells use `unicode-width` (`datascript_diagnostics::text_width`).
  Grapheme segmentation was not needed: combining marks have width 0 (D-049).
- Done: constant regexes, datetime parts, formats and percentages are `DS4004` from `check`; the
  rules are shared in `datascript_compiler::arg_values` (D-049).
- Done: the REPL has `.max-rows [n]`, and its footer names it.
- Deferred, owned by M6-01: Q-008 (needs a Kusto cluster) and Q-005 (needs users).
- Moved to new tasks: streaming `ds run` (M2-11) and naming the file in multi-source `DS3008`
  (M2-12).
- Not done here: M1-14 (warning lints) is still `todo` although this task listed it as a
  dependency. The ROADMAP's M1 criteria do not include lints, so M1 closes without them; M1-14's
  own goldens should follow the patterns noted in DIAGNOSTICS.md § Recurring patterns.
- Verified: no `DS9001` remains in any golden case; the binder no longer has a "not implemented"
  path.
