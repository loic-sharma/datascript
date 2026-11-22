# M6-04 — Bring the core docs in line with the code

**Status:** todo
**Depends on:** M2-13, M2-14
**Crates:** none (docs only: `docs/LANGUAGE.md`, `docs/ARCHITECTURE.md`, `docs/DIAGNOSTICS.md`, `docs/TESTING.md`, `docs/DECISIONS.md`, `docs/ROADMAP.md`, `docs/spikes/datafusion-wasm.md`, `docs/tasks/README.md`, `AGENTS.md`, `README.md`)
**Diagnostic codes:** none

## Goal
A review of `docs/` (November 2026) found the core docs had drifted from the code. Bring them back
in line. This waits for M2-13 and M2-14, which change some of the same files. Check each item
against the code first; some will have been fixed by then. If this is too much for one session,
split it by document into new tasks.

## Scope

### LANGUAGE.md
- [ ] §1 string operators: list `!startswith`, `!endswith` and the negated `_cs` forms (the lexer has
  them); don't imply `matches regex_cs`, `in_cs` or `between_cs` exist.
- [ ] §2: bare `from` at a query head (DS0151) vs. in an expression (D-036), `FROM` (DS0105, D-037),
  `range … from`, and that any `let`, even a scalar one, shadows a host table (DS2040).
- [ ] §2: the `@'…'` verbatim string and escapes inside `['…']` quoted identifiers.
- [ ] §3.1: `print` is with the operators in §3.2 while `range` and `datatable` are sources; pick one place.
  Mention DS3008, DS2006 and automatic decompression.
- [ ] §3.2: `join`/`union` right sides accept reader calls (`read_csv(…)`); add `reader_call` to
  §8's `right_table`. State null-key behaviour (`join` doesn't match nulls; `summarize` and
  `distinct` group them) and mark three-valued logic `≠ KQL` if it is. Optional `[name =]` on
  `summarize … by`; which side semi/anti joins return.
- [ ] §4: non-chaining applies to every level-4 operator, not just ordering comparisons; `in ()`
  needs at least one item; D-031's tie-breaking and the type of `null == null`.
- [ ] §7 function catalog: `y` in `format_datetime`; the `datetime_part`/`datetime_diff` part names and
  the direction of `dt1 - dt2`; `bin(timespan, timespan)`; `abs(timespan)`; the optional offset on
  `now`/`startof*`/`endof*`; `≠ KQL` on `extract` returning `null` instead of `""`.
  `builtins.rs`'s doc for part names omits `"week"` and `"nanosecond"`.
- [ ] §8 grammar: define `agg_item`; add `project-away`, `project-rename`, `getschema` and the other
  missing operator rules; `$right.k == $left.k` in `join_cond`; no trivia inside `typed_null`;
  `render … with ()` may be empty.
- [ ] References to "ARCHITECTURE.md § Source resolution" (no such heading) and other plain-text
  doc references: make them links.

### ARCHITECTURE.md
- [ ] `datascript-lsp` uses `tower-lsp-server`, depends on `datascript-engine` (not DataFusion) and is
  wired by `datascript-cli` (D-052). Fix the crate diagram.
- [ ] `datascript-engine` has no `Catalog`; `RenderSpec` is in `datascript-compiler::ir`. Same line in README.md.
- [ ] Type names: `ast::BinExpr`, not `BinaryExpr`; `ir::ExprKind` has no `Case` (`case` is a `Call`),
  and `Call` takes `function: String`.
- [ ] `datascript-engine` also depends on `arrow-cast` and `arrow-ipc`.
- [ ] Capability checks are not a separate pass (D-041). Check `SchemaLookup`, `SourceError`,
  `Analysis`, `QueryResult` (`render_span`, `statement`) and `Session` (`run_statement`, streaming
  API from D-055) against the code. Offsets are `u32`, not `TextSize`, if that is still true.
- [ ] Add the `datascript-tests` and `datascript-fixtures` crates. `lsp_types` is `ls_types`. The
  REPL row is missing `.max-rows` and `.exit`.

### DIAGNOSTICS.md
- [ ] The `info` examples don't match the real DS5001 and DS1007 messages, and rename a join key that
  D-019 merges.
- [ ] The Allocation table doesn't name M1-19, M1-20 and M1-22, which took codes from those blocks.

### TESTING.md
- [ ] The fixtures section misses the M1-08 groups (`widen/`, `conflict/`, `compressed/`, `json/`,
  `options/`, `bad/`, `uint64.parquet`) and the `datascript-fixtures` crate.
- [ ] Drop the stale "(M3)" on `datascript-lsp/tests`; fix the missing `crates/` prefix; the
  `show: schema` header format; list the `lints/`, `plan/`, `harness/`, `skeleton/` and
  `errors/runtime/` case folders.

### DECISIONS.md
- [ ] D-005 is out of order (between D-018 and D-019).
- [ ] D-046 doesn't say D-055 replaced how `--max-rows` works; D-055 says `RunStream::select`
  "replaces `run_statement`", which is still public; D-043's `QueryResult` sketch is out of date.
- [ ] D-045 implies the engine builds DataFusion without its function crates; the spike doc is right
  that `datafusion-functions` is always compiled in.
- [ ] D-018 cites `DS0120–DS0139` instead of `DS0102–DS0149`; D-027's size note is stale; mark
  amendments on D-007, D-012, D-031, D-045; make `**Accepted.**` and date tags consistent; Q-006
  still says "not needed for M1"; explain how Q-001 and Q-009 were closed.

### ROADMAP.md, task index, spike, AGENTS.md, README.md
- [ ] ROADMAP.md: milestones are parallel tracks after M1, not ordered by dependency; mark M0 and M1
  done and reflect landed M2/M3 tasks; M3's Run description predates D-054 (M3-05 terminal output,
  M3-06 grid); list M2-09 onward; M1 omits lints (M1-14); M6 has no "Done when".
- [ ] `docs/tasks/README.md`: M1 `Parallel with` hints contradict the dependency chain and some pairs
  aren't symmetric; M1 planning prose is in the present tense. Make its "How to use this" match
  AGENTS.md (update the index on finish; follow-ups can become new task files).
- [ ] AGENTS.md: the one-line summary goes in the task file, not the index; mention the `blocked`
  status; add `-p datascript-diagnostics` to the wasm check, as CI does.
- [ ] README.md: the VS Code extension is no longer "planned".
- [ ] `spikes/datafusion-wasm.md`: mark M0-05 notes that M1 superseded (MSRV 1.94; `/` and `%` became
  UDFs in D-045); the parquet compression row needs `datafusion/compression`.
- [ ] Closed tasks: M1-25 has no done summary; M1-09 has no `**Diagnostic codes:**` line; M1-13's
  checked boxes lack their deferral notes and still calls M1-14 `todo`.

## Acceptance criteria
- [ ] Every item is fixed or marked as already correct.
- [ ] No broken relative links in `docs/`.

## Follow-ups
