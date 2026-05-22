# PR-2-8 — Auto-arrange (single plate, no rotation)

Status: ❌ open.

**Scope.** "Auto-arrange" button that lays out all objects on the
active plate without overlap and (for MVP) without rotating them.
Greedy bin-packing using the objects' XY bounding boxes; preserves
each object's authored rotation.

Marked as a **cut candidate** in the Execution Plan: skip → users
place manually, save ~4 days.

**Acceptance criteria.**

- `scene_auto_arrange()` Tauri command that:
  - Reads the current scene's objects + active bed extents.
  - Computes XY footprints from each object's mesh bounding box
    transformed by its current rotation.
  - Greedy-packs the largest-first into the bed extents, leaving a
    small spacing (default 5 mm) between adjacent objects and
    respecting the active printer's exclusion zones.
  - Emits one `scene_object_set_transform` (PR-2-5) command per
    object with the new XY position — the renderer applies the
    diffs naturally.

- Failure mode: if not all objects fit, the command emits a
  `scene:auto_arrange_overflow` event listing the un-placed
  object IDs, and the placed ones still move. The user can
  resize / remove / split into multi-plate (Phase 5).

- Tests:
  - 10 cubes of varying sizes on an A1 mini bed: all fit, no
    overlap (verified by AABB intersection check on the post-arrange
    transforms).
  - 100 cubes that won't fit: the overflow event lists the
    correct count of un-placed objects; the placed ones don't
    overlap each other or the exclusion zone.
  - Idempotent: running auto-arrange twice on a no-overflow scene
    produces the same final positions (modulo greedy-order
    determinism).

**Effort.** ~3 days. Greedy bin-packing + AABB collision is ~1 day;
hooking into the scene + tests is the rest.

**Dependencies.** PR-2-5 (set_transform command), PR-2-6 (bed +
exclusion zones).

**Out of scope.** Rotation during arrangement — Phase 4+ if a user
asks. Multi-plate distribution — Phase 5. Skyline / no-fit polygon
packing — Phase 4+ if greedy packing wastes too much bed area.

**Cut candidate** per Execution Plan §4. If cut: drop this ticket
entirely; users place objects manually via PR-2-5's transform
operations.
