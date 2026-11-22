# M2-12 — Name the file in runtime read errors

**Status:** todo
**Depends on:** M1-13
**Crates:** `datascript-datafusion`
**Diagnostic codes:** none new expected (DS3008); reserve a block if needed

## Goal
A malformed row found while a query reads several sources is `DS3008` "cannot read one of the
query's sources", pointing at the query's last node, because DataFusion's error does not name the
file. The error should name the file and point at the scan that reads it.

## Scope
- Find which source failed: for example a per-scan wrapper around the `ListingTable` stream that
  tags errors with the scan's source path and span, or one `ListingTable` per file for globs.
- `DS3008` then reads "cannot read `<path>` as CSV" (or JSON, Parquet) with the primary span on
  the `from` or `join`/`union` table that reads it, as for a single-source query.

## Acceptance criteria
- [ ] A golden `run` case joining a good file with a file that has a malformed row names the bad
      file and points at its table.
- [ ] A glob with one malformed file names that file, not the glob.

## Carried over
- [ ] (from M1-09) A malformed row in a query that reads several sources is `DS3008` "cannot read
      one of the query's sources" at the query's last node, because DataFusion's error does not
      name the file. Find the file (e.g. a per-scan wrapper around the `ListingTable` stream) so
      the error names it and points at its scan.

## Follow-ups
