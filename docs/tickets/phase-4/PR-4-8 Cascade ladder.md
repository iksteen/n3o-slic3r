# PR-4-8 — Hover cascade ladder (portal-rendered)

Status: ✅ shipped — `src/settings/ladder/CascadeLadder.tsx` ships the portal-rendered popover with the six-layer list (object section appended separately), 120ms close delay matching the mockup, left-of-row default position with right-fallback when row is near viewport edge, `useLadderHover()` lifecycle hook with cancel-on-re-enter so the cursor can travel from row to ladder without losing it. The SettingsPanel mounts one CascadeLadder centrally and threads `onRowEnter/onRowLeave` callbacks through Field; per-row data is read at render time from the hovered key. `buildLadderLayers` populates: defaults from `schema.default_value`, project/object from override maps, cascade-tier value attributed to `printer` as a proxy until profile-source tagging lands in Phase 5. `cascade_fallback` rows surface the authored-cascade value when an override is active.

**Scope.** The contextual disclosure surface that shows the full
cascade for a single setting on hover. Renders every layer (not
just the layers that defined a value — undefined layers show an
em-dash), highlights the winner, marks overridden layers, and when
an absolute override is active also shows the cascade fallback (the
value that would resolve without the override).

Owns the rest of FR-CAS-7 — the **rule + authored-tier tint**
(PR-4-7) is the always-visible quiet summary; the ladder is the
on-hover full disclosure. Together they cover the "show the
source" requirement; the breadcrumb chip strip in the mockup is
a designer A/B tweak, not part of either ticket.

**Acceptance criteria.**

- New `src/settings/ladder/CascadeLadder.tsx`:
  - Pops open on row hover with a 200 ms open delay and a 400 ms
    close delay (so the cursor can travel from the row to the
    ladder without losing it).
  - **Rendered via React portal at body level**, not nested in
    the settings panel's scroll container — otherwise the
    scroll-overflow clips the popout.
  - Anchored to the hovered row; auto-positions left/right based
    on viewport edge proximity.

- Content per ladder row (one per cascade layer):
  - Layer name (e.g. "Bundled defaults", "Bambu A1 mini printer
    profile", "Generic PLA filament", "Textured PEI plate",
    "Project overrides", "Object: <name>").
  - Layer-hue chip matching PR-4-7's palette.
  - Resolved value at that layer, or em-dash (`—`) if the layer
    didn't define a value for this option.
  - **Winning layer** highlighted with a bold border + ✓ glyph.
  - **Overridden-by** marker: layers whose value was beaten by a
    later layer show a strikethrough on the value + a `→ overridden
    by <layer>` annotation.
  - **Cascade fallback** (when an absolute override is active):
    a `── cascade fallback ──` separator row, then the
    authored-cascade resolution beneath. So the user knows what
    they'll revert to.

- Backend: re-uses `cascade_trace` from PR-1-5 with the bulk
  bundling from PR-4-7 (per-row trace pre-fetched in the panel's
  resolve loop, not per-hover). Hover is purely a render trigger
  on already-fetched data — no network round-trip on hover.

- Per-object section (FR-CAS-7b, partial):
  - When the **project** tab is active and an object overrides
    this setting, the ladder appends a `── overriding objects ──`
    section listing each overriding object by name + filament
    color + override value.
  - Clicking an object in this section selects it in the viewport
    and switches to the Object tab. (Selection wiring goes
    through the existing `scene_select` command.)
  - When the **object** tab is active, this section is elided
    (the user is already editing the object).

- Smoke check:
  - Hover a row whose value is overridden at the project tier:
    ladder shows defaults / printer / filament / plate /
    **project** (winner) with the cascade-fallback row below
    showing what would resolve without the override.
  - Hover a row with no overrides: ladder shows the full layer
    list with the winning layer highlighted; no cascade-fallback
    separator.
  - On the Project tab with 2 objects overriding this setting:
    `── overriding objects ──` section lists both with their
    color dots; click one → object selects and tab switches.

- vitest:
  - `CascadeLadder` renders layers in the canonical cascade order
    (defaults → printer → filament → plate → user → project →
    object).
  - Cascade-fallback row appears iff an absolute override is
    active in the trace.
  - Overriding-objects section appears only on Project-tab
    context with > 0 overriding objects.

**Effort.** ~3 days. The portal + auto-positioning is a day; the
layered rendering with all the edge cases (winners, overridden,
fallback, overriding objects) is two days.

**Dependencies.** PR-1-5 (`cascade_trace`), PR-4-7 (breadcrumb +
bulk trace fetch), PR-4-9 (per-object overrides — needed for the
overriding-objects section).

**Out of scope.** Editing values inline from the ladder (would
make the ladder a complex form, defeating its read-only
disclosure role). Comparing across plates — that's a Phase 5
diff-across-plates feature, not in Phase 4.

**Cut candidate.** The per-object click-through in the overriding-
objects section (~half day per Execution Plan §6 cut list). The
section + color dots stay; clicking only highlights, doesn't
switch tabs. The 5-user UX test in the exit criteria gates on
breadcrumb + ladder visibility, not click-through.

**Design reference.** The mockup at
`docs/design/SettingsPanel.jsx`'s `CascadeLadder` component is
the canonical reference — port it almost verbatim. Key bits:

- `ReactDOM.createPortal(..., document.body)` is the portal call
  that escapes the `.settings-scroll` overflow clip.
- Fixed positioning math (mockup lines 47–55): default to the
  left of the row, fall back right when there isn't room. Same
  edge-pad value (8 px) and width (250 px) the mockup uses.
- Open/close timing: `setTimeout` of 120 ms on close (mockup
  uses 120; ticket said 200/400 earlier — **align to the
  mockup's 120 ms** since that's the timing the design tested
  against).
- The `.ladder-row` markup: `.l-dot` (filled when defined, empty
  when not) + `.l-name` + `.l-val` (em-dash for undefined). The
  winner row gets the `winner` modifier class, losers that
  define a value get `overridden`.
- The object section: when `objectOverrides.length > 0`, render
  the `.ladder-section-title` with the object hue dot + count
  ("N objects override"), then one `.ladder-row.obj-row` per
  override carrying the filament's swatch color.
- The mockup elides the `object` layer from the main layer
  list (filters it out at line 65) since it's handled by the
  per-object section — match that.

The cascade-fallback row (what the ticket calls out as a
separator + the underlying authored-cascade value) isn't in the
mockup yet; introduce it below the main layer list with a
`.ladder-fallback` class so a future designer can style it.
