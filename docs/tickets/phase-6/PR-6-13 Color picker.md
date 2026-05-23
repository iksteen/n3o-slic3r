# PR-6-13 — Color-mode picker + palette UI

Status: ❌ open.

**Scope.** Dropdown / segmented picker for the active color
mode (Feature / Speed / Flow / LayerTime / Tool) + a small
palette toggle (Default / Classic). Drives PR-6-8's renderer
via the `colorMode` + `palette` props.

**Acceptance criteria.**

- New module `src/preview/ColorModePicker.tsx`:
  ```tsx
  interface ColorModePickerProps {
    mode: ColorMode;
    palette: Palette;
    onChange: (next: { mode: ColorMode; palette: Palette }) => void;
  }
  ```

- **UI:**
  - Segmented control (or dropdown — pick simpler):
    `[Feature] [Speed] [Flow] [Layer time] [Tool]`. Active
    mode highlighted with `--accent`.
  - Below the mode picker, a small text link "Palette:
    Default ▾" toggles to Classic via a popover or right-
    click context menu.
  - Positioned in the preview-mode toolbar, top-left of the
    viewport.

- **Legend / scale strip:**
  - Discrete modes (Feature, Tool) render a small legend
    below the picker: color swatch + label for each
    feature type or tool index actually present in the
    current gcode.
  - Continuous modes (Speed, Flow, LayerTime) render a
    gradient strip with min/max value labels:
    `[colormap] 30 — 240 mm/s`.
  - Legend is interactive in v2 (click to highlight only
    that feature); MVP is static.

- **State persistence:**
  - Active mode persists across preview mounts via
    `localStorage` (`n3o-slic3r:preview:color-mode`).
  - Palette persists similarly
    (`n3o-slic3r:preview:palette`).
  - Defaults: `mode = "Feature"`, `palette = "Default"`.

- **Color values for the legend** come from PR-6-5's
  palette definitions. The frontend needs to know the RGB
  hex per FeatureType + per tool index — either invoke a
  `preview_palette_legend` Tauri command (clean, central
  source of truth) or hardcode the palette in TS (faster,
  risks divergence). Recommend the invoke; cost is
  negligible.

- Tests:
  - Default values render correctly.
  - Clicking each mode button fires `onChange` with the
    correct mode.
  - localStorage round-trip works.
  - Continuous-mode legend renders min/max values from the
    stats data.

**Effort.** ~1 day. Picker UI + legend strip. The Rust
palette-legend command is trivial (returns a `HashMap`).

**Dependencies.** PR-6-5 (color encoders define the
palette), PR-6-7 (Tauri command for the palette legend),
PR-6-8 (renderer accepts the props).

**Out of scope.**

- Custom user palettes (post-MVP).
- Per-mode min/max range overrides ("show only segments
  faster than 100mm/s") — Phase 9.
- Click-legend-to-isolate (Phase 9).

**Cut candidate.** Cut alternate Classic palette → save
~0.5 days. Default-only ships. Compatible with the "all 5
modes" decision; the alternate palette is independent.
