# PR-2-5 — Object transform operations

Status: ❌ open.

**Scope.** The seven operations the user can apply to selected
objects: **move**, **rotate**, **scale**, **mirror**, **lay flat**,
**duplicate**, **delete**. Each is a Tauri command on the
PR-2-2 surface that mutates the scene state and emits the
appropriate `scene:object_updated` (or `added`/`removed`) events.

The renderer reflects the resulting state diff; it does *not*
compute transforms itself. This is the core of the AD-8 invariant
— transform math is one place, and it's Rust.

**Acceptance criteria.**

- Each command in `core/scene/operations.rs`:
  - `scene_object_translate(id, delta: Vec3)` — appends to current
    transform.
  - `scene_object_rotate(id, axis: Vec3, radians: f64)` — rotates
    around object's *current center* (or a user-supplied pivot
    via the gizmo's `pivot` field — see PR-2-10).
  - `scene_object_scale(id, factor: Vec3)` — uniform when all
    components equal; non-uniform otherwise. Surfaces a warning
    event when non-uniform scale would distort dimensional
    settings (PR-1-6's adapter cares about this for line widths).
  - `scene_object_mirror(id, axis: MirrorAxis)` — mirrors across
    X, Y, or Z. **Cut candidate** per the Execution Plan.
  - `scene_object_lay_flat(id)` — auto-orient so the largest flat
    face is on the build plate. Uses a simple "rotate until
    smallest Z extent" heuristic for MVP.
  - `scene_object_duplicate(id) -> ObjectId` — new ObjectId,
    transform offset by ~10mm so the copy doesn't z-fight.
  - `scene_object_delete(ids: Vec<ObjectId>)`.

- All transforms compose via the PR-2-1 `Transform` newtype's
  `compose` operation. The scene state stores the *accumulated*
  transform; the renderer applies it verbatim.

- Each operation emits a single `scene:object_updated` (or
  `added`/`removed`) event with the new transform. Multi-object
  ops (delete with selection) emit one event per object.

- Collision check (out-of-bed-volume) runs after each operation;
  results in a `scene:object_out_of_bounds` warning event (not
  blocking — the user can fix or accept).

- Tests:
  - Translate twice along X — accumulated `Transform` reflects the
    sum.
  - Rotate around pivot — object's world-space position relative
    to pivot is preserved.
  - Mirror across X then mirror across X — original transform.
  - Lay flat on a cube with arbitrary rotation — settles with Z up
    and minimum Z extent ≈ 0.
  - Duplicate then delete original — clone survives at the offset
    transform.

**Effort.** ~3 days. Translate / rotate / scale + composition is
the matrix-math core (1 day); mirror + lay flat + duplicate + bounds
are mechanical (1 day); tests + event integration (1 day).

**Dependencies.** PR-2-1, PR-2-2.

**Out of scope.** Multi-object batch transforms (a "group" concept
that transforms several objects as one unit) — defer to Phase 5
when multi-plate projects need it. Snap-to-grid + snap-to-other-
object — UI sugar, Phase 4. Undo/redo — Phase 4.

**Cut candidate.** Mirror (~1 day savings — users can do it in CAD).
