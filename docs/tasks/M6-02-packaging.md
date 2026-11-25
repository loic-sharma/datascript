# M6-02 — Packaging and releases

**Status:** todo
**Depends on:** M1-13, M2-13, M3-04 (VS Code package), M4-02 (`web/` embedded in `ds`), M5-02 (`@datascript/wasm`)
**Crates:** release tooling

## Goal
Release automation (`cargo-dist` or similar): prebuilt binaries for macOS/Linux/Windows, a Homebrew
tap, `cargo install`, npm publishing for `@datascript/wasm`, and VS Code Marketplace publishing.
Changelog and versioning policy.

## Acceptance criteria
- [ ] A dry-run release from CI produces all artifacts. Nothing is published without the maintainer's approval.

## Carried over
- [ ] (from M3-04) Ship `ds` inside platform-specific VS Code packages (`vsce package --target darwin-arm64` …) and have `editors/vscode/src/extension.ts` prefer the bundled binary when `datascript.server.path` is unset. Today the default is `ds` on `PATH`. CI's `vscode` job already builds the .vsix; set the real `publisher` in `editors/vscode/package.json` before publishing.
- [ ] (from planning, 2026-11-24) If M7-03 is done, publish its wheels to PyPI (built in CI by M7-03), under the same maintainer-approval rule. Otherwise leave this item for whoever finishes M7-03.
- [ ] (from planning, 2026-11-24) If M7-04 is done, ship `libdatascript` (shared and static) and `datascript.h` with each release. Otherwise leave this item for whoever finishes M7-04.

## Follow-ups
