# M3-03 — Language server: quick fixes, navigation, semantic tokens

**Status:** todo
**Depends on:** M3-01
**Crates:** `datascript-lsp`

## Goal
Code actions from machine-applicable suggestions, go to definition and find references for `let`s
and UDFs, rename, document symbols, semantic tokens, and formatting (after M2-07).

## Acceptance criteria
- [ ] Each feature has an LSP test.

## Carried over
- [ ] (from M3-04) Semantic tokens must work in syntax-only mode (D-053): they come from the parse alone, or skip what needs the binder. Map the token types onto the TextMate scopes in `editors/vscode/syntaxes/datascript.tmLanguage.json` (`keyword.control.operator`, `variable.other`, `entity.name.function`, …) so themes color both the same. They should fix what the grammar cannot know: contextual keywords (`by`, `on`, `has`, …) used as column names.

## Follow-ups
