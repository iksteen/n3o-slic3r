# PR-6-10 — Travel + retraction visibility toggles

Status: ❌ open.

**Scope.** Two checkbox toggles that hide/show the travel
LineSegments and the retraction Points objects in the
preview. State lives in the preview panel; props flow down
to PR-6-8's renderer.

**Acceptance criteria.**

- New module `src/preview/VisibilityToggles.tsx`:
  ```tsx
  interface VisibilityTogglesProps {
    showTravels: boolean;
    showRetractions: boolean;
    onChange: (next: { showTravels: boolean; showRetractions: boolean }) => void;
  }
  ```

- **UI:**
  - Two compact checkboxes (or icon-buttons with on/off
    state) labeled "Travels" + "Retractions".
  - Positioned in the preview-mode toolbar (top-left of the
    viewport region, next to the color-mode picker
    PR-6-13).
  - Travels default: off (visual noise on dense prints).
  - Retractions default: off (only useful for debugging
    stringing).

- **Renderer integration:** PR-6-8's `<GcodePreview/>`
  accepts `showTravels` + `showRetractions` props and sets
  `material.visible` on the relevant Three.js objects
  accordingly. No buffer rebuild needed.

- **State persistence:** toggle state persists across
  preview mounts via `localStorage` (key:
  `n3o-slic3r:preview:show-travels` and
  `n3o-slic3r:preview:show-retractions`). User's "always
  show travels" preference survives a restart.

- Tests:
  - Default values: travels off, retractions off.
  - Toggling fires `onChange` with the new state.
  - localStorage round-trip works (mount → toggle → unmount
    → remount preserves state).

**Effort.** ~0.5 days. Pure UI; renderer integration is a
prop pass-through.

**Dependencies.** PR-6-8 (renderer accepting the props).

**Out of scope.**

- Travel color customization (Phase 9).
- Retraction marker size customization (Phase 9).
- "Show only retractions on layer N" debug mode
  (post-MVP).

**Cut candidate.** None — small enough that cutting buys
nothing.
