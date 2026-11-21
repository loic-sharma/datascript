# M0-01 — Workspace scaffold and CI

**Status:** todo
**Depends on:** none
**Crates:** all (empty stubs)

## Goal
Create the Cargo workspace and CI so every later task has a place to put code and a gate to pass.

## Scope
- Root `Cargo.toml` workspace (edition 2024, `resolver = "3"`) with `[workspace.dependencies]` for
  shared versions.
- Stub library crates: `datascript-diagnostics`, `datascript-syntax`, `datascript-compiler`,
  `datascript-engine`, `datascript-datafusion`. Stub binary crate `datascript-cli` (binary name `ds`,
  prints `--version`). Empty test crate `datascript-tests` (filled in by M0-03).
  Do **not** create the lsp/server/wasm crates yet.
- `rust-toolchain.toml` pinned to `stable` with `rustfmt`, `clippy`, and the `wasm32-unknown-unknown` target.
- `rustfmt.toml`, `clippy.toml` / workspace lints (`unsafe_code = "forbid"` in core crates).
- GitHub Actions workflow: fmt check, clippy `-D warnings`, `cargo test --workspace`, and
  `cargo check -p datascript-diagnostics -p datascript-syntax -p datascript-compiler --target wasm32-unknown-unknown`.
  Cache with `Swatinem/rust-cache`.
- `LICENSE-MIT`, `LICENSE-APACHE`, `.gitignore`, `.editorconfig`.
- An `examples/` folder with `top_customers.ds` (the README example, using fixture paths).

## Out of scope
Any real functionality.

## Acceptance criteria
- [ ] `cargo build --workspace`, `cargo test --workspace`, and the wasm32 check pass locally.
- [ ] CI workflow file exists and is valid (run `act` or push a branch if available).
- [ ] `cargo run -p datascript-cli -- --version` prints the version.
- [ ] README "Building" section updated with real commands.

## Notes
- Use `clap` (derive) in the CLI from the start.
- Keep DataFusion out of the workspace until M1-08 to keep early builds fast.

## Follow-ups
