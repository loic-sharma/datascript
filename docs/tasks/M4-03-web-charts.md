# M4-03 — Web UI: charts from `RenderSpec` (Apache ECharts)

**Status:** todo
**Depends on:** M4-02, M1-10
**Crates:** `web/`

## Goal
Draw every render kind from [LANGUAGE.md §6](../LANGUAGE.md#6-render) from a `RenderSpec` using
tree-shaken **Apache ECharts** (D-060) plus a custom KPI component for `card`, with a Chart/Table
toggle and reusable component exports for VS Code's M3-06 results panel.

## Scope
- Implement `RenderSpec` → ECharts mapping in `web/src/components/results/` for all 9 kinds:
  `table`, `timechart`, `linechart`, `areachart`, `barchart` (horizontal, category on `x`),
  `columnchart` (vertical), `piechart`, `scatterchart`, and `card` (first row labelled KPI cards).
- Honor all `RenderSpec` properties: `x`, `y`, `series` (combining multi-column series keys per
  LANGUAGE.md §6.2), `title`, `xtitle`, `ytitle`, `legend`, `ymin`, `ymax`, and `stacking`
  (`default`, `stacked`, `stacked100`).
- **Cell UX**:
  - Queries with `| render <kind>` default to Chart view with a crisp **Chart / Table** toggle,
    fullscreen expand (`[ ]`), interactive zoom/hover tooltips, and PNG/SVG export.
  - Plain table queries stay in Table view; offer a quiet **Visualize (`| render`)** helper button
    that appends a starter `| render ...` line to the query so DataScript code stays the source of
    truth (D-063).
  - Match the dark and light themes seamlessly (palette, grid lines, axis labels, tooltips).
- **Graceful degradation**: detect unrenderable or pathological inputs (e.g. >500 distinct series,
  empty dataset, all-null numeric series) and show a clear message with one-click switch to Table
  view instead of hanging the browser.

## Acceptance criteria
- [ ] Unit tests for `RenderSpec` → ECharts option generation and series-limit / empty-data guards.
- [ ] Playwright visual regression tests (screenshots in dark and light themes) for every render
      kind using fixture queries.
- [ ] A spec that is valid but unrenderable (e.g. 10k series) degrades immediately with a message,
      not a hang.

## Follow-ups
