# PR-4-12 — Support toggle per object + first ~30 "why this matters" annotations

Status: ❌ open.

**Scope.** Two related but independently-scoped deliverables that
naturally pair (both are user-facing-content work that doesn't
need new infrastructure):

1. **Support toggle per object** — a single on/off switch on the
   selected object's row (FR-3D-6). When ON, libslic3r generates
   supports per the cascade-resolved settings; when OFF, no
   supports.
2. **First ~30 "why this matters" annotations** — authored text
   blocks attached to the highest-impact options, surfaced by
   PR-4-11's tooltip layer.

**Acceptance criteria.**

- Support toggle:
  - New component `src/settings/object/SupportToggle.tsx`,
    rendered prominently above the settings list when the
    Object tab is active.
  - Toggle state maps to the per-object override of
    `enable_support` (a libslic3r `bool`). ON sets
    `enable_support = true`; OFF sets `false`. Both via
    PR-4-9's `scene_object_override_set`.
  - Reset (drops the override → falls back to cascade resolution)
    is the same as any other override's reset.
  - Visual: a switch + a one-line summary "Auto-generated supports
    using current cascade settings" (ON) / "No supports on this
    object" (OFF).
  - Smoke: toggling supports on a selected object → re-slice
    produces a G-code with `;TYPE:support` features when ON,
    none when OFF. (Verifies via PR-3-6's parser on the slice
    output.)

- Annotations:
  - New file `src/settings/annotations/data.ts`:
    ```ts
    export const ANNOTATIONS: Record<string, string> = {
      layer_height: "Lower = finer surface detail and slower print. …",
      sparse_infill_density: "Higher = stronger part, more filament. …",
      // ... 30 entries
    };
    ```
    Plain-text or short-markdown content; no per-locale i18n yet
    (Phase 9 if needed).
  - Pick the 30 highest-impact options. Authoring guidance:
    cover the categories users touch most ("Quality", "Walls",
    "Top/Bottom", "Strength", "Speed", "Support", "Adhesion");
    aim for ~4-5 annotations per top category.
  - Each annotation is 2-4 sentences: what the setting controls
    in physical/printer terms, the trade-off, and a quick rule
    of thumb. Avoid restating libslic3r's tooltip.
  - PR-4-11's `SettingTooltip` consumes this map; no other
    wiring needed.

- vitest:
  - SupportToggle round-trip: ON → cascade resolve for the object
    reports `enable_support = true`; OFF → false; reset → cascade
    fallback (whatever the cascade resolves without override).
  - Annotation map has at least 30 entries and every entry's key
    is a known libslic3r option (cross-check against
    `slicer_options`).

**Effort.** ~2 days. Support toggle is ~half day. Authoring 30
annotations is ~1.5 days (writing is harder than wiring; resist
shipping thin two-sentence stubs).

**Dependencies.** PR-4-9 (per-object override set/clear), PR-4-11
(tooltip surface that consumes the annotations).

**Out of scope.** Paint-on supports — post-MVP (PRD §10). Per-
material support overrides ("PETG supports for ABS bodies") —
Phase 7c filament work. Locale-specific annotations.

**Cut candidate.** Annotations beyond the first 10 highest-impact
options (~1 day savings vs full 30). Per Execution Plan §6 the
full 30 is the floor; cutting any further hurts the
"primary differentiator" UX claim of the cascade-aware UI. The
support toggle is uncuttable — it's a PRD MVP requirement
(FR-3D-6).
