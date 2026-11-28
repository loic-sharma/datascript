# M4-12 — Path sandbox hardening & `DS3007` (D-016)

**Status:** todo
**Depends on:** M1-13, M2-13
**Crates:** `datascript-datafusion`, `datascript-engine`, `datascript-diagnostics`
**Diagnostic codes:** DS3007 (planned, D-016), DS3065–DS3069 (sources) (D-018)

## Goal
Make D-016's path sandbox trustworthy before `ds ui` (M4-01) relies on it: fix the two verified
escapes, cover the sandbox with an integration suite, finalize `DS3007`, and apply the sandbox to
`output` paths. This is engine work with no HTTP server; split out of M4-01 so it can run in
parallel with M4-08 and M4-13.

## Scope
- Fix the sandbox escapes and add the `TempDir` suite listed under "Carried over".
- Review `DS3007`'s wording against [DIAGNOSTICS.md](../DIAGNOSTICS.md) style and mark it active
  in `codes.rs` and DIAGNOSTICS.md.
- If M2-25 is done, apply the sandbox to `output` paths (carried over below).

## Acceptance criteria
- [ ] With `ExecContext::sandbox` set, `from '/etc/passwd' | take 1`, `from '../x.csv'`, a symlink
      pointing out of the root, and an escaping glob that matches nothing all fail with `DS3007`
      (D-016, D-036), in `Session` + `DataFusionEngine` tests.
- [ ] Golden or rendered-output tests for `DS3007`'s final wording.

## Carried over
- [ ] (from M1-08, via M4-01) The sandbox hook exists: with `ExecContext::sandbox` set, the DataFusion engine returns `SourceError::OutsideSandbox` (paths compared after resolving symlinks and `..`, globs per match) and the binder reports DS3007 with draft wording. Review the wording, add golden tests, and mark DS3007 active in `codes.rs` and DIAGNOSTICS.md. (M4-01 sets the sandbox to the UI root.)
- [ ] (from M2-29, via M4-01) Fix two verified sandbox escapes in `check_sandbox` / `glob_files` (`crates/datascript-datafusion/src/sources.rs`) before relying on the hook: (1) a missing file under a symlinked directory falls back to lexical `normalize()`, so an intermediate symlink can leave the root; canonicalize the deepest existing ancestor, then append and normalize the rest. (2) A glob that escapes the root but matches nothing (`../*.nope`, `/outside/*.csv`) returns DS3004 instead of DS3007, because only matches are checked; check the glob's non-wildcard prefix against the sandbox first.
- [ ] (from M2-29, via M4-01) Cover the sandbox with a `TempDir` integration suite against `Session` + `DataFusionEngine` (`#[cfg(unix)]` for symlinks): file and directory symlinks pointing out (existing and missing targets), escaping globs with zero, all-outside and mixed matches, a sibling-prefix directory (`root2/a.csv` against `root`), absolute paths inside and outside, and host tables registered inside vs outside the root.
- [ ] (from planning, 2026-11-24, via M4-01) If M2-25 is done, apply D-016's sandbox to `output` paths too: writes outside the root are DS3007, with a test. (If M2-25 lands after this task, M2-25 does it.)

## Follow-ups
