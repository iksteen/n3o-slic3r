# PR-6-9 — Layer slider + keyboard nav

Status: ❌ open.

**Scope.** Single React component that owns the layer-window
state (`LayerWindow`) and renders the slider control. Drives
PR-6-8's renderer via the `layerWindow` prop. Three modes:
single-layer, up-to-N, range. Keyboard shortcuts for layer
stepping.

**Acceptance criteria.**

- New module `src/preview/LayerSlider.tsx` + state types in
  `src/preview/types.ts`:
  ```tsx
  export type LayerWindow =
    | { mode: "single"; layer: number }
    | { mode: "up-to"; max: number }
    | { mode: "range"; min: number; max: number };

  export interface LayerSliderProps {
    layerCount: number;
    value: LayerWindow;
    onChange: (next: LayerWindow) => void;
  }
  ```

- **Slider UI:**
  - **Single mode:** one slider thumb, label shows "Layer N
    of M".
  - **Up-to-N mode:** one slider thumb, label shows "Layers
    1..N of M".
  - **Range mode:** two slider thumbs, label shows "Layers
    A..B of M".
  - Mode picker: three small radio buttons or a segmented
    control above the slider.
  - Layer thumbnail / Z value tooltip on hover (defer to
    Phase 9 if effortful; basic numeric label is MVP).

- **Keyboard shortcuts** (only when the slider has focus or
  preview mode is active and no input is focused):
  - `↑` / `↓` — step layer up/down (single or up-to mode),
    or extend/contract the max in range mode.
  - `Shift + ↑/↓` — step by 10.
  - `Home` / `End` — jump to first/last layer.
  - `1` / `2` / `3` — switch modes (single/up-to/range).

- **State persistence:** layer window doesn't persist across
  preview mounts (every load lands at "up-to-N, max = last
  layer"). Persistent state is Phase 9 polish.

- **Default:** `{ mode: "up-to", max: layerCount }` on load
  (full print visible).

- **Edge cases:**
  - `layerCount === 0` (empty gcode) → slider greyed out.
  - Switching mode preserves the current visible layer: e.g.
    single→up-to keeps the current layer as the new `max`;
    up-to→range keeps the current `max` as the new `max`,
    sets `min = 0`.

- Tests (`src/preview/__test__/LayerSlider.test.tsx`):
  - **Mode switch preserves visible layer** across all 6
    transitions.
  - **Keyboard shortcuts** map to the expected state
    transitions (use `userEvent.keyboard`).
  - **Clamps to [0, layerCount-1]:** stepping past either
    end is a no-op, not an error.
  - **`onChange` fires once per user interaction** (no
    double-fires from React strict mode).

**Effort.** ~1.5 days. The mode-preservation logic + keyboard
handler are the main work; the slider control itself is
straightforward.

**Dependencies.** PR-6-8 (renderer that consumes
`layerWindow`).

**Out of scope.**

- The shader uniforms that actually clip layers (PR-6-8
  owns the GPU side).
- Layer thumbnails (Phase 9).
- Per-layer time bars / scrub heatmap (Phase 9).
- Touch-screen gestures (post-MVP).

**Cut candidate.** Range mode → save ~1 day. Keep single +
up-to. Per Exec Plan cut list. **Not recommended** — user
signed off on all 3 modes.
