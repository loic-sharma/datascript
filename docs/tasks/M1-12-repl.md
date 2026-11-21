# M1-12 — REPL

**Status:** todo
**Depends on:** M1-11
**Crates:** `datascript-cli`

## Goal
`ds` with no arguments starts an interactive session.

## Scope
- `reedline` line editor: persistent history (`~/.local/share/datascript/history`, or the platform
  equivalent via `dirs`), multi-line input (continue while the input ends with `|` or brackets are
  unbalanced, submit on `;` or a blank line).
- `let` statements persist across inputs within the session. Errors render against the
  snippet the user just typed.
- Meta commands: `.help`, `.schema <source>`, `.tables` (host tables + lets), `.format table|csv|json`,
  `.quit`. Unknown meta commands get did-you-mean.
- Basic syntax highlighting using the lexer, and Tab completion of operators, functions, and lets
  (columns can come later via the binder's stage schemas).

## Acceptance criteria
- [ ] Session logic (statement accumulation, let persistence, meta commands) is unit-tested
      separately from the terminal I/O.
- [ ] Ctrl-C cancels a running query without exiting. Ctrl-D exits.

## Follow-ups
