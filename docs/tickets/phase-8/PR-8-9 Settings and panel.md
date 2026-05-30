# PR-8-9 — plugin-declared settings in the cascade + Plugins panel

Status: ❌ open.

**Scope.** The frontend phase: plugin-declared settings (from the
manifest) become real, cascade-participating settings under a Plugins
category, and a Plugins panel lists installed plugins with their
enabled state, scope, and errors. This is what turns the example
plugins' hardcoded defaults (PR-8-5/8-6/8-7) into user-editable
settings.

Owns **FR-PL-6** (plugin-declared settings in the cascade UI) and
**FR-PL-9** (Plugins panel).

**Acceptance criteria.**

- **Backend — plugin settings into the cascade:**
  - The manifest `[settings.*]` declarations (PR-8-2) are surfaced to
    the settings catalog as a synthetic **Plugins** category, namespaced
    per plugin (`plugin.<name>.<key>`) so two plugins can't collide.
  - These settings participate in the cascade like any other: a plugin
    setting resolves through the authored/override tiers, and the
    resolved value is what the host passes to that plugin's hook
    (replacing the manifest-default read used in PR-8-5..8-7).
  - Plate-level plugin metadata (per-plate values a compose-style
    plugin would key on — for MVP, just whatever the post-slice
    plugins expose, e.g. the platecycler macro override) is editable
    where it belongs; document the boundary (global plugin settings via
    the cascade vs. any per-plate value via plate metadata).

- **Frontend — Plugins panel:**
  - A Plugins category/panel listing each discovered plugin: name,
    version, declared hooks, printer scope, **enabled toggle**, and
    **last error** (the `plugin_list` / `plugin_set_enabled` commands
    from PR-8-3, refreshed on the `plugin:changed` event).
  - A plugin in the errored state shows its error inline and reads as
    disabled; toggling enabled re-attempts load (calls `plugin_reload`).
  - The settings panel's Plugins category renders the plugin-declared
    settings using the existing data-driven form components (string /
    number / bool / enum), wired to the same cascade write path as
    other settings (per-printer visibility honored: a plugin scoped to
    one printer model hides its settings on others).
  - Follows the design language already in `src/index.css` /
    `docs/design`; no bespoke styling unless a gap exists.

- Tests:
  - Backend: a manifest setting appears in the catalog under the
    Plugins category, namespaced; resolving it through the cascade and
    overriding it returns the override; the host hands the resolved
    value to the hook.
  - Frontend: panel renders a stubbed plugin list (enabled/errored);
    the enabled toggle and an errored plugin's inline error render;
    a plugin-declared enum setting renders + writes through the
    cascade.

**Effort.** ~3 days (spans backend cascade integration + frontend
panel + settings rendering).

**Dependencies.** PR-8-2 (manifest settings), PR-8-3 (plugin commands
+ events), `core/cascade` + the Phase 4 settings UI it plugs into.

**Out of scope.**

- A plugin install/uninstall/marketplace flow — plugins are dropped in
  the folder by hand for MVP.
- Per-plugin log streaming beyond `last_error` (a one-line error is
  enough for MVP; a full log pane is post-MVP).
- Hot reload refreshing the panel live → PR-8-10 (this ticket consumes
  the `plugin:changed` event; the watcher that fires it on file change
  is PR-8-10).
