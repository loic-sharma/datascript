# M6-02 — Packaging and releases

**Status:** todo
**Depends on:** M1-13
**Crates:** release tooling

## Goal
Release automation (`cargo-dist` or similar): prebuilt binaries for macOS/Linux/Windows, a Homebrew
tap, `cargo install`, npm publishing for `@datascript/wasm`, and VS Code Marketplace publishing.
Changelog and versioning policy.

## Acceptance criteria
- [ ] A dry-run release from CI produces all artifacts. Nothing is published without the maintainer's approval.

## Carried over
- [ ] (from M3-04) Ship `ds` inside platform-specific VS Code packages (`vsce package --target darwin-arm64` …) and have `editors/vscode/src/extension.ts` prefer the bundled binary when `datascript.server.path` is unset. Today the default is `ds` on `PATH`. CI's `vscode` job already builds the .vsix; set the real `publisher` in `editors/vscode/package.json` before publishing.

## Follow-ups
