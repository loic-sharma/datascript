# M7-02 — S3, GCS and Azure sources

**Status:** todo
**Depends on:** M7-01
**Crates:** `datascript-compiler`, `datascript-datafusion`, `datascript-cli`
**Diagnostic codes:** DS3120–DS3139 (D-018)

## Goal
`from 's3://bucket/logs/2024-*/*.parquet'`, `gs://…` and `az://…` (also `abfss://`) read from
cloud object stores, with globs.

## Scope
- `object_store`'s `aws`, `gcp` and `azure` features, each behind its own cargo feature of
  `datascript-datafusion` and `ds` (all on by default in release builds unless the size cost says
  otherwise), and only in native builds (not wasm32). Record the release binary size and clean
  build time before and after each in the task's notes.
- Credentials come only from each cloud's standard chain: environment variables, profile and
  config files, instance metadata. Never from the script text. Region detection for S3, and an
  endpoint override for S3-compatible stores (MinIO, R2) through an environment variable or a
  `ds` config (decide which; no config file exists today).
- Globs list objects under the longest literal prefix, then match; sorted matches and name-based
  schema merging as for local files (LANGUAGE.md §3.1).
- Errors from this task's block: no credentials found (help lists where they are looked for),
  access denied (names bucket and key), bucket not found, region mismatch. Diagnostics never
  contain secrets.
- Same sandbox policy as M7-01 for `ds ui` and the LSP.
- Out of scope: writing to object stores with `output` (M2-25). Not planned yet; do not add it
  here.
- Update LANGUAGE.md §3.1.

## Acceptance criteria
- [ ] Integration tests against a local S3-compatible server (MinIO or `object_store`'s in-memory
      store behind the same code path), gated so `cargo test --workspace` passes without Docker.
- [ ] Golden or unit tests for each error, with a check that no credential value appears in any
      diagnostic.
- [ ] `datascript-datafusion` still passes its wasm32 check with these features off.

## Follow-ups
