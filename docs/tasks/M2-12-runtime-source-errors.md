# M2-12 — Name the file in runtime read errors

**Status:** todo
**Depends on:** M1-13, M2-13
**Crates:** `datascript-datafusion`
**Diagnostic codes:** none new expected (DS3008); DS3060–DS3064 if needed (D-018)

## Goal
Name the bad file, and point at the scan that reads it, when a malformed row is found in a query
that reads several sources (see "Carried over").

## Scope
- Find which source failed: for example a per-scan wrapper around the `ListingTable` stream that
  tags errors with the scan's source path and span, or one `ListingTable` per file for globs.
- `DS3008` then reads as it does for a single-source query: the message ``cannot read `<path>` ``
  (`crates/datascript-datafusion/src/errors.rs`), the format and problem in the label, and the
  primary span on the `from` or `join`/`union` table that reads it.

## Acceptance criteria
- [ ] A golden `run` case joining a good file with a file that has a malformed row names the bad
      file and points at its table.
- [ ] A glob with one malformed file names that file, not the glob.

## Carried over
- [ ] (from M1-09) A malformed row in a query that reads several sources is `DS3008` "cannot read
      one of the query's sources" at the query's last node, because DataFusion's error does not
      name the file. Find the file (e.g. a per-scan wrapper around the `ListingTable` stream) so
      the error names it and points at its scan.
- [ ] (from M2-29) `DataFusionEngine::execute` checks that every checked file still exists before
      planning, but a file deleted after that check and before DataFusion opens it still reads as
      no rows (DataFusion lists a missing file as no files). The same per-scan wrapper could turn
      a missing file at open time into the "moved or deleted" `DS3008`.

## Follow-ups
