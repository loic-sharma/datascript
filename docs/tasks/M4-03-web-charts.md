# M4-03 — Web UI: charts from `RenderSpec` (Apache ECharts)

**Status:** todo
**Depends on:** M4-09, M1-10
**Crates:** `web/`
**Diagnostic codes:** none

## Goal
Draw every render kind from [LANGUAGE.md §6](../LANGUAGE.md#6-render) from a `RenderSpec` using
tree-shaken **Apache ECharts** (D-061) plus a custom KPI component for `card`, with a Chart/Table
toggle and reusable exports in `web/src/components/results/` for VS Code's M3-06 results panel
([WEB_UI.md §4.2](../WEB_UI.md#42-center-pane--notebook-canvas)).

## Scope
- Implement `RenderSpec` → ECharts mapping for all 9 kinds (`table`, `timechart`, `linechart`,
  `areachart`, `barchart` horizontal, `columnchart` vertical, `piechart`, `scatterchart`, `card`),
  honoring `x`, `y`, multi-column `series`, `title`, `xtitle`, `ytitle`, `legend`, `ymin`, `ymax`,
  and `stacking`.
- Default `| render` queries to Chart view with a Chart/Table toggle, fullscreen expand, zoom/hover
  tooltips, PNG export (Canvas) and SVG export (off-screen `SVGRenderer` re-render), and matching dark/light themes; keep plain table queries in Table view
  with a quiet **Visualize (`| render`)** helper that appends a starter `| render ...` line (D-062).
- Gracefully degrade unrenderable inputs (e.g. >500 distinct series, empty or all-null series) with
  a clear message and one-click switch to Table view.

## Acceptance criteria
- [ ] Unit tests for `RenderSpec` → ECharts option generation and series-limit / empty-data guards.
- [ ] Playwright visual regression tests (dark & light themes) for every render kind using fixture
      queries.
- [ ] A spec that is valid but unrenderable (e.g. 10k series) degrades immediately with a message,
      not a hang.

## Follow-ups
