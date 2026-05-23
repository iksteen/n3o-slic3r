# PR-4-9 — Per-object overrides + objects-overriding badge + reset action

Status: ✅ frontend shipped — Reset button per row in Field's `resetButton` slot (counter-clockwise arrow SVG matching the mockup; renders when the active tier has a value, calls `onClearProjectOverride` / `onClearObjectOverride`). Objects-overriding badge in Field's `trailingBadge` slot on the Project tab (up to 3 filament-color dots + `+N` overflow when more than 3 objects override a setting). CascadeLadder receives the `objectOverrides` array on the Project tab and renders the per-object section the mockup specifies. `allObjects: PlateObjectStub[]` prop on `SettingsPanel` carries the per-object override maps; Phase 5's project model populates. **Backend Tauri commands deferred** to Phase 5 (`scene_object_override_set/clear/clear_all` + SceneState storage) since SettingsPanel isn't mounted in App.tsx yet — the panel takes the override storage from props, so the wiring is a Phase-5 concern.

**Scope.** The Object editing tab (PR-4-4 ships the tab; PR-4-9
makes it real) lets the user override any object/region-scope
setting on the selected object. Object overrides live in a per-
object map; PR-1-4's override-tier resolver already handles them.
This ticket also ships the **objects-overriding-this** badge on the
Project tab (FR-CAS-7b) and the per-row **Reset** action that drops
an override and falls back to the cascade resolution.

Owns FR-3D-3 (per-object overrides), FR-CAS-7b (objects-overriding
badge), and the per-row reset affordance.

**Acceptance criteria.**

- Backend per-object override storage:
  - Extend `SceneState` to carry an
    `object_overrides: HashMap<ObjectId, HashMap<String, String>>`
    map. Each entry is the object's authored overrides (key →
    serialized libslic3r value).
  - Tauri commands:
    - `scene_object_override_set(object_id, key, value)` — upserts.
    - `scene_object_override_clear(object_id, key)` — drops the
      override.
    - `scene_object_override_clear_all(object_id)` — wipes the
      object's map (for the "reset all object overrides" UX).
  - Each mutation emits a `scene:object_overrides_changed` event
    so the panel re-resolves.

- Cascade integration:
  - `cascade_resolve` already accepts an `object_overrides` tier
    via PR-1-4. The settings panel includes the active object's
    override map on Object-tab `resolve` calls; for Project-tab
    resolves the overrides are passed as a *separate* per-object
    map so the resolver can report the objects-overriding-this
    set (the reverse query: for setting K, which objects override
    it?).

- Per-row Reset action:
  - Reset button appears in each row when the **active editing
    tier** has a value for the option. On Project tab, reset
    means "drop the project override"; on Object tab, means
    "drop this object's override."
  - Clicking reset calls the appropriate `cascade_fallback` /
    `scene_object_override_clear` and re-resolves. The row's
    value snaps to the underlying cascade resolution.
  - Reset is **per-tier**: clicking reset on the Project tab when
    only an Object override exists is a no-op (the project tier
    isn't overriding the option from this tab's perspective).

- Objects-overriding badge (FR-CAS-7b):
  - On the Project tab, every row whose setting is overridden by
    one or more objects shows a small badge: up to 3 filament-
    color dots (the objects' filament colors via `printer.slot`
    → filament color) + a `+N` overflow if more than 3.
  - Hovering the badge opens (or extends — if the cascade ladder
    is open from PR-4-8, this enriches its overriding-objects
    section) the per-object listing.
  - On the Object tab the badge is elided for the currently-
    edited object's overrides (those are already visible via the
    Object tab itself).

- Scope enforcement:
  - Project-scope settings (per the `OptScopeFlags` from PR-4-1)
    are read-only on the Object tab — the input is disabled with
    a small badge "project-scope setting" (PRD FR-3D-3).
  - The Object tab's list filters out settings whose scope is
    project-only when the user toggles "show only overridable"
    (default ON for the Object tab to reduce noise).

- Smoke check:
  - Select an object → Object tab activates. Edit
    `wall_filament` to a non-default → cascade re-resolves with
    the override, value sticks per object, switching to another
    object shows that object's defaults (not the first's
    override).
  - Project tab: the row for `wall_filament` shows the
    objects-overriding badge with the right color dots; hover
    expands to the per-object list. Click reset on that row from
    Project tab → no-op (only Object tab can clear an object's
    override).
  - Click reset on a project-tier-overridden setting from
    Project tab → row falls back to the cascade resolution and
    the row's breadcrumb loses its purple chip.

- vitest:
  - Backend: `scene_object_override_set` then
    `scene_object_override_clear` round-trip; clear_all empties.
  - Frontend: the row's reset button appearance matches the
    active tier's having-an-override condition.
  - Objects-overriding badge: 0 overrides → no badge; 1 →
    1 dot; 4 → 3 dots + "+1".

**Effort.** ~4 days. Per-object override backend storage + Tauri
plumbing is 1.5 days; cascade integration on Project tab
(objects-overriding reverse query) is a day; UI for the badge +
reset + scope enforcement is 1.5 days.

**Dependencies.** PR-1-4 (`resolve_with_overrides`), PR-4-1
(scope flags), PR-4-4 (panel scaffold with tabs), PR-4-7
(breadcrumb chip already in place to receive the per-object
extension), PR-2-1 (SceneState).

**Out of scope.** Per-volume overrides (the cascade tier is per-
**object**; per-volume granularity would need a 4th tier). Bulk-
edit affordances ("apply override to all selected") — Phase 9
polish.

**Cut candidate.** The Object-tab "show only overridable" toggle
(~half day). Without it, the Object tab shows every setting with
project-scope ones read-only.

**Design reference.** The mockup's `SettingRow` already wires
the per-object override flow — clone its prop shape:

- `selectedObject` (present when `contextLayer === "object"`):
  the object whose overrides we're editing.
- `objects` (all objects on the plate): used to compute the
  `objectOverrides` array per row (mockup lines 134–144 —
  filter to objects whose `overrides[setting.id]` is defined).
- `filaments`: for the per-object color swatches in the badge +
  ladder section.
- `userOverrides`: the project-tier override map (mockup
  separates user from project; PR-4-9 uses the project tier per
  PRD §FR-CAS-3).
- Handlers: `onSetProjectOverride`, `onResetProjectOverride`,
  `onSetObjectOverride`, `onResetObjectOverride` — match these
  names on the production component so the mockup's wiring
  examples translate.

The objects-overrides badge: `.objs-badge` with the inline
arrow SVG (mockup lines 286–296) + `.objs-badge-dots`
containing up to three `.objs-badge-dot` swatches and an
`.objs-badge-more` "+N" overflow. Only renders when
`contextLayer !== "object"` (the Object tab's own settings
shouldn't badge themselves).

The reset button (`.reset-btn`, mockup lines 300–311) is the
counter-clockwise arrow SVG + tooltip text
`Reset <Layer> override (falls back to inherited value)`.
Renders only when `hasValueAtContext` is true (the active tier
defines a value for this setting).
