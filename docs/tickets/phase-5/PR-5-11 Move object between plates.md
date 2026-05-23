# PR-5-11 — Move-object-between-plates

Status: ❌ open (cut candidate — see notes).

**Scope.** Single operation: move an object from one plate
to another, preserving its world-space position where the
target plate's printer geometry allows.

Owns FR-MP-6.

**Acceptance criteria.**

- New Tauri command:
  ```rust
  scene_move_object(
      from_plate: PlateId,
      to_plate: PlateId,
      object_id: ObjectId,
  ) -> Result<MoveReport, String>
  ```
  Validates that:
  - `object_id` exists on `from_plate`
  - `to_plate` is different from `from_plate`
  - the object's bounding box, when re-anchored on the
    target plate's bed, fits within the target printer's
    `build_volume`

- `MoveReport`:
  ```rust
  pub struct MoveReport {
      pub object_id: ObjectId,
      pub new_position: [f32; 3],
      /// `Some(reason)` if the original world-space
      /// position didn't fit and the object was
      /// repositioned. The UI surfaces this as a toast.
      pub repositioned: Option<RepositionReason>,
  }
  pub enum RepositionReason {
      OutOfBounds,
      OnExclusionZone,
      BelowBedSurface,
  }
  ```

- Implementation details:
  - Remove the object from `from_plate.scene.objects`.
  - If the target's build volume is identical to the
    source (same printer family), keep the world-space
    position verbatim — covers the common "two A1 minis
    in the project" case.
  - Otherwise, re-anchor to the target's plate-center +
    bed-z, with collision check against existing
    objects on the target.
  - Per-object overrides on the moved object (PR-5-7)
    move with the object — they're attached to the
    `ObjectId` which stays unique across plates per
    PR-5-2's scene-wide id allocator.

- Emits two events: `scene:object_removed { from_plate,
  object_id }` and `scene:object_added { to_plate,
  object_id, ... }`. The frontend mirror handles each
  on the respective plate; if the user is currently
  viewing one of the affected plates, the viewport
  reflows.

- UI surface: drag-and-drop the object from the
  viewport onto a plate tab in `PlateTabs`. The tab
  highlights on drag-over; release calls the command.
  Alternative: right-click on object → "Move to
  plate…" menu with the plate list.

- Tests:
  - 3-plate fixture, move an object from plate 1 → plate
    2, verify the object map updates on both plates +
    overrides travel with the object.
  - Out-of-bounds case: source plate is A1 mini (180×180),
    object is at (170, 170), target is a smaller
    printer (150×150) — the report indicates
    `RepositionReason::OutOfBounds` and the object is
    moved to the target's plate center.

**Effort.** ~2 days. Backend command + the drag-drop
plumbing on the PlateTabs side.

**Dependencies.** PR-5-2 (per-plate scene state),
PR-5-3 (PlateTabs for the drop target).

**Out of scope.** Multi-select move (move N objects in
one drag) — Phase 9 polish. Cross-plate-copy (move
preserves vs copy duplicates) — would need a separate
command + UI surface; defer.

**Cut candidate.** **Whole ticket** per Execution Plan §7
cut list (saves ~2 days). Cut first if shipping pressure
hits — users can delete + re-add an object on the target
plate as a workaround. The PRD requirement (FR-MP-6)
becomes Phase 9 polish.
