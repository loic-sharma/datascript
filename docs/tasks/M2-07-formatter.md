# M2-07 — Formatter (`ds fmt`)

**Status:** todo
**Depends on:** M1-13
**Crates:** `datascript-syntax`, `datascript-cli`

## Goal
An opinionated formatter over the CST: one operator per line with leading `|`, normalized spacing,
and comments preserved.

## Acceptance criteria
- [ ] `ds fmt [--check] <files>`.
- [ ] Idempotent and comment-preserving over the whole golden corpus (an automated test).
- [ ] Refuses to format files with syntax errors, saying why.

## Carried over
- [ ] (from planning, 2026-11-22) If M3-03 is done, add `textDocument/formatting` to the language server (M3-03 left it for after this task), with an LSP test.
- [ ] (from M1-24) The formatter keeps `from <source>` on the first line of a pipeline, with operators on the following lines: `from orders⏎| where …`, never `from⏎orders`.

## Follow-ups
