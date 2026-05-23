# PR-6-11 — Hover inspection (raycast → segment tooltip)

Status: ❌ open.

**Scope.** Raycast onto the extrusion `LineSegments` in
PR-6-8's renderer. On hover-with-hit, surface a tooltip
showing the source gcode line, position, speed, feature,
layer. Powered by PR-6-7's `preview_segment_detail`
command.

**Acceptance criteria.**

- New module `src/preview/HoverTooltip.tsx`:
  ```tsx
  interface HoverTooltipProps {
    detail: SegmentDetail | null;
    mouseX: number;
    mouseY: number;
  }
  ```

  Renders as a small floating panel near the cursor when
  `detail != null`, hidden otherwise. Content:
  - Source gcode line (monospace, e.g. `G1 X120.34 Y84.21
    E0.0341 F1800`).
  - Position: `(X, Y, Z)` in mm.
  - Speed: `1800 mm/min` (convert from internal mm/s).
  - Feature: human-readable label ("External perimeter").
  - Layer: `42 of 187`.
  - Tool: `T0` (omit when single-tool).
  - Extrusion: `0.034 mm` (omit for travels).

- **Raycast pipeline** (lives in `previewScene.ts` /
  `GcodePreview.tsx`):
  - On `mousemove` over the canvas, project to NDC, raycast
    against `extrusionsMesh` only (not travels, not the bed
    grid).
  - Three.js's `Raycaster.intersectObject` on
    `LineSegments` returns intersections; pick the closest
    in z.
  - Each intersection carries an `index` into the
    `BufferGeometry`'s position attribute. Two vertices per
    segment → `segmentIndex = index / 2`.
  - Throttle to ~60Hz with `requestAnimationFrame` to avoid
    invoke spam.
  - On hit: invoke `preview_segment_detail(handle,
    segmentIndex)` → set tooltip detail.
  - On miss: clear tooltip detail.

- **Performance:** raycasting a 3M-segment `LineSegments`
  in Three.js is `O(N)` per cast. Profile during impl:
  - If acceptable (<16ms per cast on dev hardware), ship as-is.
  - If too slow, fall back to a spatial index: bucket
    segments by layer + bin XY into a coarse grid; only
    raycast against the current `layerWindow`'s segments.

- **Tooltip positioning:** anchored to the mouse, offset by
  `(12, 12)` to keep it off the cursor; flips to the other
  side when near the right/bottom viewport edges.

- Tests:
  - **`HoverTooltip` renders all fields** correctly for a
    sample `SegmentDetail`.
  - **Travel segments omit extrusion + tool** (or render
    them as `—`).
  - **Edge-flip logic:** mouse at `(viewport_width - 5,
    100)` flips tooltip to the left of the cursor.
  - **Raycast smoke** (deferred if jsdom lacks WebGL — note
    in ticket).

**Effort.** ~1.5 days. Raycast plumbing is the biggest
unknown; the tooltip UI is straightforward.

**Dependencies.** PR-6-7 (`preview_segment_detail` command),
PR-6-8 (renderer exposes `onSegmentHover` callback).

**Out of scope.**

- Click-to-pin tooltip (Phase 9).
- "Jump to gcode line" cross-reference panel (Phase 9).
- Multi-segment selection (post-MVP).

**Cut candidate.** None — hover inspection is FR-GP-4, hard
MVP requirement.
