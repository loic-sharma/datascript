# AGENTS.md — working agreement

DataScript is a KQL-inspired, pipe-based query language in Rust, running on Apache DataFusion,
targeting the CLI, a local web UI, the browser (WASM), and VS Code (LSP). Read
[README.md](README.md) for the vision. This file explains how to work in the repo.

## Before you start a session

1. Read [docs/tasks/README.md](docs/tasks/README.md). Pick a task whose status is `todo` and whose
   dependencies are all `done`, unless the user assigned one.
2. Read the task file, then the docs it links. At minimum skim
   [ARCHITECTURE.md](docs/ARCHITECTURE.md), [DIAGNOSTICS.md](docs/DIAGNOSTICS.md) and
   [TESTING.md](docs/TESTING.md).
3. Set the task's status to `in-progress` in both the task file and the index table.
4. Work on a branch named `task/<id>-<slug>`, e.g. `task/M1-02-parser`.

## While working

- **Tests first.** Behaviour is specified with `.dstest` golden files under `tests/cases/`
  (see [TESTING.md](docs/TESTING.md)). Add cases for the happy path *and* for the errors a user is
  likely to hit. Error output is tested as first-class behaviour.
- **Errors are a top priority.** Never return a bare string error to a user. Produce a
  `Diagnostic` with a code, a primary span, a message in plain words, and help when a fix is
  likely. Follow the style rules in [DIAGNOSTICS.md](docs/DIAGNOSTICS.md).
- **Respect the layering** in [ARCHITECTURE.md](docs/ARCHITECTURE.md):
  `datascript-syntax` and `datascript-compiler` must not depend on DataFusion, tokio, or the
  filesystem. They must build for `wasm32-unknown-unknown`.
- **The spec is the contract.** If an implementation needs to differ from
  [LANGUAGE.md](docs/LANGUAGE.md), update the spec in the same change and record the reason in
  [DECISIONS.md](docs/DECISIONS.md).
- **Diagnostic codes** come only from the block your task file names (D-018). Never pick the
  "next free" code outside it.
- Keep changes scoped to the task. If you find out-of-scope work, add a new task file (next free
  id in the right milestone) and an index row rather than doing it inline.

## Finishing a task

- `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, and
  `cargo test --workspace` pass. Core crates still pass
  `cargo check -p datascript-syntax -p datascript-compiler --target wasm32-unknown-unknown`.
- Every acceptance criterion in the task file is met, or the gap is written down in the task file
  under "Follow-ups".
- **Hand every follow-up to an owner.** A follow-up must never live only in the finished task's
  file, because agents only read the task they pick. For each one, either add a checkbox to the
  owning task's "Carried over" section (create it above "Follow-ups"), formatted as
  `- [ ] (from M1-04) …`, or create a new task file with an index row. Purely informational notes
  (decisions taken, things verified) need no owner.
- Set status to `done` in the task file and the index, with a one-line summary of what landed.

## Commands (once M0-01 lands)

```bash
cargo build --workspace
cargo test --workspace                  # includes golden tests
DS_BLESS=1 cargo test -p datascript-tests   # rewrite expected sections of .dstest files
cargo run -p datascript-cli -- run examples/top_customers.ds
```

## Conventions

- Rust edition 2024, current stable toolchain. No `unsafe` outside the WASM crate without a comment
  explaining why.
- Error codes: `DS` + 4 digits, ranges in [DIAGNOSTICS.md](docs/DIAGNOSTICS.md). Never reuse a
  retired code.
- Crate names are `datascript-*`. The binary is `ds`.
- Public APIs in core crates get doc comments. Internal code gets comments only where the "why" is
  not obvious.
