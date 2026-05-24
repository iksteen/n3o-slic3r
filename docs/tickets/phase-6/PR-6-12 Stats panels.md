# PR-6-12 — Per-layer + full-job stats panels

Status: ✅ shipped.

**Scope.** Two side-panels in preview mode that surface
PR-6-6's computed stats. Full-job panel always visible;
per-layer panel updates as the layer slider scrubs.

**Acceptance criteria.**

- New modules:
  ```
  src/preview/
    FullJobStatsPanel.tsx
    PerLayerStatsPanel.tsx
  ```

- **`<FullJobStatsPanel/>` props:**
  ```tsx
  interface FullJobStatsPanelProps {
    stats: FullJobStats;
    header: HeaderMetadata;  // estimated time, filament use from gcode header
  }
  ```

  Renders (top-down):
  - **Header section** (small, dimmed): slicer-of-origin
    (from `header.generator`), estimated time (from
    `header.estimated_time_seconds` if present, else
    "computed: X" from `stats.total_duration_seconds`).
  - **Time breakdown:** total time, then per-feature
    horizontal bar chart (perimeter / infill / support /
    travel / other) with %.
  - **Filament use:** per-tool rows (`T0: 12.4m of Generic
    PLA`). Filament identity comes from the gcode header's
    filament metadata if present, else "tool 0..N".
  - **Layer count + height range:** "187 layers,
    0.16-0.24mm (variable)" — the `(variable)` badge
    appears when `HeightStats.variable == true`.
  - **Bounding box:** `120 × 80 × 35 mm`.

- **`<PerLayerStatsPanel/>` props:**
  ```tsx
  interface PerLayerStatsPanelProps {
    stats: PerLayerStats | null;  // null when layerWindow is range
    layerIndex: number;
    layerCount: number;
  }
  ```

  Renders:
  - "Layer N of M (Z = X.XXmm, height = 0.20mm)"
  - Time: `12.3s`
  - Max speed: `120 mm/s`
  - Per-feature time breakdown (compact list).
  - Per-tool filament: `T0: 0.84m, T1: 0.20m`.

  When `stats == null` (range mode), panel shows "Range
  view: select single or up-to mode to see per-layer
  stats."

- **Layout:**
  - Both panels stack vertically in a fixed-width column on
    the right side of the preview viewport (~280px wide).
  - The settings panel doesn't render in preview mode
    (PR-6-15 hides it); preview reclaims that column for
    stats.
  - Per-layer panel sits above full-job panel.

- **Stats fetching:**
  - Full-job stats come from `preview_load`'s response
    (PR-6-7), no extra invoke.
  - Per-layer stats: invoke `preview_layer_stats(handle)`
    once per preview load (small payload, ~200 layers ×
    ~200 bytes = ~40KB). Memoize in `useState`. Look up
    `stats[layerIndex]` per render.

- Tests:
  - Both panels render all fields correctly for a sample
    fixture.
  - `(variable)` badge appears iff `HeightStats.variable`.
  - Single-tool fixture omits the per-tool rows in favor of
    a single "Filament: 12.4m" row.
  - Range mode renders the placeholder text in the
    per-layer panel.

**Effort.** ~1.5 days. Mostly straightforward stat
rendering; the per-feature bar chart is the only fancy bit
(could be CSS flex with width %, no chart library needed).

**Dependencies.** PR-6-6 (stats types), PR-6-7
(`preview_layer_stats`), PR-6-9 (`layerWindow` for the
per-layer index).

**Out of scope.**

- Editable stats (e.g. "what if I sped up X"?) — Phase 9.
- Export stats to CSV / JSON — Phase 9.
- Stats charts (time-vs-layer plot) — Phase 9.
- Per-extruder filament prices / cost estimate — post-MVP.

**Cut candidate.** Per-layer panel → save ~1 day. Keep
full-job only. Per Exec Plan cut list. **Not recommended** —
user signed off on both panels.
