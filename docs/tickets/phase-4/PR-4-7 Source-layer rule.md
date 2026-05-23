# PR-4-7 — Source-layer rule + authored-tier tint + override count badges

Status: ❌ open.

**Scope.** Every settings row carries a **left-edge rule** that
reveals the winning cascade layer's hue on hover, plus a
persistent **background tint** when the winning layer is the
project or object tier (the two override tiers users author
through the UI). These two affordances together are the canonical
"show the source" surface — quiet at rest, consultable on hover.

The category sidebar (PR-4-3) and the panel header show
**override-count badges** so the user knows where divergence
from the printer default lives without scanning every row.

This ticket ships the **`accountability === "rule"`** variant
from the mockup — the default in `TWEAK_DEFAULTS`. The
breadcrumb chip strip (`accountability === "breadcrumb"`) and
the ladder-only mode (`accountability === "ladder-only"`) are
designer A/B tweaks and are NOT lifted into production. The
companion hover ladder ships in PR-4-8.

Owns FR-CAS-7's row-level disclosure (the quieter half — the
ladder in PR-4-8 is the louder half).

**Acceptance criteria.**

- **Row-level rule** — match `docs/design/styles.css:972-1029`
  exactly:
  ```css
  .set-row {
    /* Left rule hidden at rest */
    box-shadow: inset 3px 0 0 0 transparent;
    transition: box-shadow 120ms, background 120ms;
  }
  .set-row:hover {
    background: var(--surface-2);
    box-shadow: inset 3px 0 0 0 hsl(var(--row-hue) 70% 55%);
  }
  .set-row.overridden:hover {
    box-shadow: inset 4px 0 0 0 hsl(var(--row-hue) 70% 50%);
  }
  ```
  `--row-hue` is bound per-row to the winning layer's `hue`
  from `CASCADE_LAYERS` in `docs/design/data.jsx`. Production
  reuses the mockup's stylesheet verbatim — copy the relevant
  blocks from `docs/design/styles.css` into the production
  styles, do not rewrite.

- **Authored-tier tint** — persistent background that marks
  rows at rest when the winning layer is `project` or `object`:
  - `.set-row.authored-project` → soft purple-violet tint (light:
    `hsl(--proj-hue 90% 97.5%)`; dark: `hsl(--proj-hue 25% 16%)`)
    + `font-weight: 600` on `.set-name`.
  - `.set-row.authored-object` → soft rose tint (light:
    `hsl(--obj-hue 90% 97.5%)`; dark: `hsl(--obj-hue 25% 16%)`)
    + bold name. Wins over project tint.
  - Conflict state (`.set-row.has-conflict`): warning-tinted
    background. Triggered when the resolver flags a tie or an
    override-with-no-cascade-fallback condition (PR-1-5 trace
    surfaces this).

- **Per-row className composition** (per
  `docs/design/SettingsPanel.jsx:252-260`):
  ```jsx
  className={`set-row
    ${isOverridden ? "overridden" : ""}
    ${conflict ? "has-conflict" : ""}
    ${hasObjectAuthored ? "authored-object"
      : hasProjectAuthored ? "authored-project" : ""}`}
  style={{ "--row-hue": layerMeta.hue, ... }}
  ```
  Derive `hasObjectAuthored` / `hasProjectAuthored` from the
  `cascade_trace` result for this row.

- **Per-category override-count badge** (consumed by PR-4-3's
  `CategorySidebar`): count of options in that category whose
  winning layer is an override tier (`user` / `project` /
  `object`). Renders in `.cat-rail-count` as `overrides/total`
  in `--accent-text` when overrides > 0; plain `total`
  otherwise (matches `docs/design/SettingsPanel.jsx:645-650`).

- **Panel-header total badge:** sum of per-category counts,
  next to the search bar. Click opens the diff view (PR-4-10);
  hover shows a tooltip "{N} settings overridden across {M}
  categories". The mockup's `.statusbar` (app.jsx:373-383)
  prototypes the same info; lift the count text idea, not the
  bottom-bar placement.

- **Backend integration:** the panel's `cascade_resolve` call
  in PR-4-4 must include the per-row trace so each row knows
  its winning layer. PR-4-7 verifies `cascade_trace` (PR-1-5)
  bundles ≥ 600 rows in < 30 ms for the canonical A1 mini
  cascade; if it doesn't, add a bulk `cascade_resolve_with_trace`
  endpoint. The trace is what tells us *which* layer won — the
  row-hue derivation needs it.

- vitest:
  - The row's `--row-hue` matches the winning layer's `hue`
    from `data.jsx` for fixture traces (defaults → 220,
    project win → 285, etc.).
  - `authored-project` / `authored-object` modifier classes
    apply iff the winning layer is `project` / `object`.
  - Per-category override count is a pure function over the
    fixture trace map; snapshot test.

**Effort.** ~2.5 days. The styling lift from
`docs/design/styles.css` is mechanical (half a day). Trace
bulk-endpoint validation + className composition + per-category
accumulator is two days.

**Dependencies.** PR-1-5 (`cascade_trace`), PR-4-3
(category sidebar — receives the badge), PR-4-4 (panel
scaffold).

**Out of scope.** The hover cascade ladder (PR-4-8 — the rule
+ tint summarize at rest; the ladder is the on-hover full
disclosure). The breadcrumb chip strip — it's the
`accountability === "breadcrumb"` tweak, not the shipping UX.
A `breadcrumb` build-time / dev-mode toggle for designers who
want to keep evaluating the alternative — Phase 9 if anyone
asks.

**Cut candidate.** The panel-header total badge (~half day);
per-category badges already give the same information at
finer grain. The badge is convenient but cuttable.

**Design reference.** The "rule" mode CSS lives at
`docs/design/styles.css:972-1029` (`.set-row` block + the
`.authored-project` / `.authored-object` / `.overridden` /
`.has-conflict` modifiers). The className composition is in
`docs/design/SettingsPanel.jsx:252-260`. Hue palette is in
`docs/design/data.jsx:10-18` (default 220, printer 18,
build_plate 95, filament 175, user 235, project 285, object
340). The category-count badge convention is at
`docs/design/SettingsPanel.jsx:645-650`. **The breadcrumb
markup at lines 267-279 (gated on `accountabilityMode ===
"breadcrumb"`) is NOT to be lifted** — it's a designer
comparison tweak.
