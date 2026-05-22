# PR-2-6 — Bed mesh + exclusion zones

Status: ❌ open.

**Scope.** The build-plate visualization the renderer paints
beneath the scene: bed grid, origin marker, A1 mini's exclusion
zone (the AMS-feed area near origin where prints can't reach), and
U1's toolhead parking bay markers. Drives both the visual
representation and the "out of bounds" collision check that PR-2-5
emits warnings against.

Data lives in `core/scene/`; rendering is the renderer's
responsibility (PR-2-9) — this ticket ships the Rust side.

**Acceptance criteria.**

- `ExclusionZone` already declared in PR-2-1's `PrinterProfile`
  surface; this ticket exposes it on the scene state and emits
  `scene:exclusion_zones_changed` when the active printer
  changes.

- `BedMesh` struct in `core/scene/bed.rs`:
  - `extents: BoundingBox` (printer-specific build volume)
  - `grid_spacing: f64` (default 10 mm; PR-2-9 makes this
    user-configurable in a later iteration)
  - `origin_marker: Vec3` (typically `[0, 0, 0]`; some printers
    centre at the bed centre)

- `scene_bed_for_printer(printer: &PrinterProfile) -> BedMesh`
  constructs the bed from a printer profile (using the build
  volume + exclusion zones from PR-1-7).

- Out-of-bounds check helper:
  `pub fn object_out_of_bounds(obj: &SceneObject, mesh: &Mesh, bed: &BedMesh) -> Vec<OutOfBoundsReason>`
  returns reasons including:
  - object's bounding box extends beyond `bed.extents`
  - object intersects an `ExclusionZone`
  - object below build plate (Z < 0)
  PR-2-5 calls this after each transform op.

- The renderer (PR-2-9) subscribes to `scene:bed_changed` events
  to redraw the grid + zones; this ticket emits them.

- Tests:
  - A1 mini bed + AMS exclusion zone: an object placed inside the
    AMS feed zone produces an `IntersectsExclusion` reason.
  - Object placed outside the 180×180×180 build volume: produces
    `OutOfBuildVolume` reason.
  - Object below Z=0: produces `BelowBuildPlate`.

**Effort.** ~2 days.

**Dependencies.** PR-2-1 (scene state), PR-1-7 (printer profile —
already done; the BoundingBox + ExclusionZone shapes shipped in
Phase 1).

**Out of scope.** Grid texture / shading — renderer (PR-2-9).
Z-height limit warnings on tall objects — Phase 5 / Phase 7
filament-aware logic. Multi-plate beds — Phase 5.
