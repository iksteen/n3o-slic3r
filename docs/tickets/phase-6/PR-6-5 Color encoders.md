# PR-6-5 — Color-mode encoders

Status: ✅ shipped.

**Scope.** Per-segment color computation for the five MVP color
modes: feature type, speed, flow rate, layer time, tool index.
Pure functions over the PR-6-4 `SegmentSet`; output is a
per-vertex `[r, g, b]` color buffer the renderer (PR-6-8) binds
as a vertex attribute.

**Acceptance criteria.**

- New module `core/preview/colors.rs`. Suggested surface:
  ```rust
  pub enum ColorMode {
      Feature,
      Speed,
      Flow,
      LayerTime,
      Tool,
  }

  pub enum Palette {
      /// Color-blind-safe default. Categorical colors picked
      /// from a CVD-tested palette (e.g. Wong 2011 or the
      /// Okabe-Ito 8-color scheme).
      Default,
      /// Alternate palette for users who explicitly prefer
      /// a slicer-conventional warm-cool feature mapping.
      Classic,
  }

  /// Per-vertex color buffer ([r, g, b] floats, 0..1) sized
  /// `positions.len()` (3 floats per vertex × 2 vertices per
  /// segment). Caller binds as a Three.js attribute.
  pub fn encode_colors(
      segments: &SegmentSet,
      mode: ColorMode,
      palette: Palette,
      layer_times: Option<&[f32]>,  // only required for LayerTime
  ) -> Vec<f32>;
  ```

- **Per-mode color mapping:**

  | Mode | Source | Mapping |
  |------|--------|---------|
  | `Feature` | `segments.feature` | Discrete: each FeatureType variant → palette entry (Wong 8-color: perimeter/external/infill/solid/top/bridge/support/skirt). |
  | `Speed` | `segments.speed` | Linear interpolation along a viridis-style colormap, min..max of the segment set. |
  | `Flow` | `segments.flow` | Same continuous colormap as Speed but driven by flow. |
  | `LayerTime` | `layer_times[segments.layer_index]` | Same continuous colormap, driven by per-layer time from PR-6-6 stats. |
  | `Tool` | `segments.tool` | Discrete: 8 palette entries for 8 tools (libslic3r supports up to 8). Index modulo 8 for any printer with more. |

- **Color-blind safety:**
  - `Palette::Default` uses Wong / Okabe-Ito categorical
    colors for discrete modes (Feature, Tool).
  - Continuous modes (Speed, Flow, LayerTime) use viridis-
    style colormaps that are perceptually uniform across
    common color-vision deficiencies (deuteranopia,
    protanopia, tritanopia).
  - Document the chosen color values in module docs;
    reference Wong (2011) or equivalent attribution.

- **Travel + retraction colors:** travels render as a flat
  desaturated grey (e.g. `#808080` at low alpha). Retractions
  render as a small dot/marker at the retract point, color
  `#ff4444` regardless of mode. The color encoder doesn't
  emit travel/retraction colors; the renderer hard-codes
  them (PR-6-8).

- Tests:
  - **Output buffer length matches `positions.len()`** (3
    floats per vertex).
  - **Discrete-mode determinism:** the same `FeatureType`
    always maps to the same RGB across calls.
  - **Continuous-mode normalization:** the min and max
    values in the input array map to the colormap's first
    and last entries respectively.
  - **Tool wraparound:** tool index 9 maps to the same color
    as tool index 1 (9 mod 8 = 1).
  - **LayerTime requires `layer_times`:** call with `mode =
    LayerTime, layer_times = None` returns an error (or
    panics with a clear message — pick during impl).

- **Perf:** encoding 3M segments takes < 100ms on the dev
  hardware. Criterion bench in PR-6-16.

**Effort.** ~1.5 days. Straightforward mapping work; the
colormap LUTs are short tables.

**Dependencies.** PR-6-4 (`SegmentSet` shape), PR-6-6
(`layer_times` for the `LayerTime` mode).

**Out of scope.**

- Frontend color-mode picker UI (PR-6-13).
- Palette swatches in the picker (PR-6-13).
- Per-mode legend / scale bar (Phase 9 polish).
- Custom user palettes (post-MVP).
- Color-vision-deficiency simulator preview (post-MVP).

**Cut candidate.** Flow + LayerTime modes → save ~1 day total.
Keep Feature + Speed + Tool. Per Exec Plan cut list. **Not
recommended** — user signed off on all 5 modes for MVP.
