# M3-03 — Language server: quick fixes, navigation, semantic tokens

**Status:** todo
**Depends on:** M3-01, M2-13
**Crates:** `datascript-lsp`, `datascript-compiler` (`ide`)

## Goal
Code actions from machine-applicable suggestions, go to definition and find references for `let`s,
rename, document symbols, and semantic tokens. For UDFs (after M2-05) and formatting (after M2-07)
the later task carries the work over; this task does not wait for them.

## Scope
- Resolve `let` names from the syntax tree, in `datascript_compiler::ide` next to `complete`, so
  the web UI and WASM can share it. The bound program does not link a use to its definition: a
  scalar `let` use is `ExprKind::Let { name, value }` with the folded value, and there is no HIR
  at all in syntax-only mode.
- In syntax-only mode (D-053) every feature here must work from the parse alone or be skipped:
  navigation and rename over `let`s, document symbols and semantic tokens work; code actions come
  only from syntax diagnostics.

## Acceptance criteria
- [ ] Each feature has an LSP test.

## Carried over
- [ ] (from M3-04) Semantic tokens must work in syntax-only mode (D-053): they come from the parse alone, or skip what needs the binder. Map the token types onto the TextMate scopes in `editors/vscode/syntaxes/datascript.tmLanguage.json` (`keyword.control.operator`, `variable.other`, `entity.name.function`, …) so themes color both the same. They should fix what the grammar cannot know: contextual keywords (`by`, `on`, `has`, …) used as column names.
- [ ] (from M3-02) Completion gaps left by M3-02: an `extend` item does not yet see the columns made by the items before it (D-040), and `$left.` / `$right.` in `join` conditions offer nothing. Both are in `datascript_compiler::ide::complete`.

## Follow-ups
