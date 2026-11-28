# M4-17 — Web UI: multi-file tabs, Outline & Scope

**Status:** todo
**Depends on:** M4-09, M4-04
**Crates:** `web/`
**Diagnostic codes:** none

## Goal
Multi-file tabs with pinning, plus the Outline and Scope inspector tabs
([WEB_UI.md §4.2–§4.3](../WEB_UI.md#43-right-pane--contextual-inspector-collapsed-by-default)).
Split out of M4-05.

## Scope
- **Multi-file tabs & pinning**: slim tab bar preserving open tabs' outputs, scroll position, and
  undo history, with pinning and close/pin buttons (M4-05, which depends on this task, adds
  the `Alt+W` chord and palette commands).
- **Outline & Scope inspector tabs** ([WEB_UI.md §4.3](../WEB_UI.md#43-right-pane--contextual-inspector-collapsed-by-default)).

## Acceptance criteria
- [ ] Playwright tests for multi-tab workflows (outputs and undo history survive switching) and the
      Outline and Scope tabs (jumping to a heading, a cell, a `let`'s defining cell).

## Follow-ups
