# PR-5-7 — Per-object override Tauri backend (deferred from PR-4-9)

Status: ❌ open.

**Scope.** PR-4-9 shipped the frontend reset button + objects-
overriding badge against a stubbed callback prop. The
backend storage + Tauri commands deferred to Phase 5 since
the natural home is per-plate state. Now that PR-5-2 has
landed the per-plate refactor, plug the real backend in.

Owns the **backend half** of FR-3D-3.

**Acceptance criteria.**

- Per-plate object override storage:
  - `PlateSceneState.object_overrides:
     HashMap<ObjectId, HashMap<String, String>>`
    (declared in PR-5-2 already).
  - Each entry is the object's authored override map:
    setting key → serialized libslic3r value.

- New Tauri commands (replacing PR-4-9's stub callbacks):
  - `scene_object_override_set(plate_id, object_id, key, value)`
    — upserts.
  - `scene_object_override_clear(plate_id, object_id, key)`
    — drops a single override.
  - `scene_object_override_clear_all(plate_id, object_id)`
    — wipes the object's entire override map (for the
    "reset all object overrides" UX).
  - Each command emits
    `scene:object_overrides_changed { plate_id, object_id }`
    so the panel re-resolves.

- Frontend wiring:
  - PR-4-9's stub callbacks (`onSetObjectOverride`,
    `onClearObjectOverride`) become real `invoke()`
    wrappers.
  - PR-4-4's panel host (`App.tsx`, PR-5-9 ships the
    integration) passes the plate-scoped commands.

- `cascade_resolve` integration: extend the existing
  command (PR-1-9) to accept an optional `object_overrides:
  HashMap<String, String>` map. The panel passes the
  active object's overrides when the Object tab is active;
  empty map otherwise.

- Tests:
  - Set → clear → set round-trip; verify the override
    persists across `cascade_resolve` calls.
  - 3-plate fixture: override the same key on different
    objects across different plates; each plate's
    cascade resolves independently.
  - `cascade_resolve_with_overrides` accepts the
    object-tier map and the resolved value reflects it
    (verifies PR-1-4's resolver path actually consumes
    the tier).

**Effort.** ~1.5 days. Tauri command plumbing + cascade
integration; the storage is a `HashMap` extension on
the per-plate state.

**Dependencies.** PR-5-1 (project types), PR-5-2 (per-plate
state), PR-1-4 (resolver's override-tier path —
already present, just plug the object tier through).

**Out of scope.** UI changes — PR-4-9 already ships the
reset + badge surfaces. Per-volume overrides — would need
a 4th cascade tier; post-MVP.

**Cut candidate.** `scene_object_override_clear_all` (~half
day) — users can clear settings one-by-one via PR-4-9's
per-row reset. Cut if shipping date pressure hits.
