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

## Enablement model (decided 2026-05-30)

The original ticket left "enabled state / scope" undefined. Decided:

**Two separate axes.** Don't conflate them (today's single `enabled`
bool does):

- **Health** — loaded vs errored. Host-managed, session, global:
  auto-disable on a runtime error, `reload` to recover, surfaced in the
  panel as a status. Not a user activation control.
- **Activation** — does the user want it to run, and where. A
  cascade-resolved `plugin.<name>.enabled` (bool).

**Activation rides the override tiers.** `plugin.<name>.enabled` is a
synthetic plugin flag that resolves through the **same override tiers**
as plugin settings (FR-PL-6) — so global / per-project / per-plate all
fall out of one mechanism. Precedence (highest wins): **plate → project
→ global-default → manifest-default (`true`)**. Because overrides are
two-way, a global `true` can be flipped to `false` for one plate (the
"enable globally, disable for this plate" requirement) and vice-versa.
It is resolved **directly from the override tiers, not via
`cascade::resolve`** (step 2): a plugin key has no authored cascade rule,
so the tiers + the manifest default are its only sources, and routing it
through the cascade would only feed unknown `plugin.*` keys to the
libslic3r adapter (which drops them). Implementation note, not a
behavioural difference.

**Plugins declare their applicable scopes.** A new manifest field
`scopes` lists which cascade levels a plugin's `enabled` flag *and* its
`[settings.*]` may be set at — vocabulary `global` / `project` /
`plate`. Default when omitted: all three. A plugin only meaningful
app-wide declares `scopes = ["global"]`; the UI offers no project/plate
control for it and an override at those tiers should be ignored at
resolve. **Not yet enforced (deferred):** step 1 parses + surfaces
`scopes`, but step 2's activation resolver does not consult it — an
override at any tier currently takes effect regardless of declared
scope. Scope-gating lands with the settings resolver (step 3/4), where
the UI also stops offering the disallowed tiers. (Printer applicability
stays a separate axis — `printer_compatibility`, now enforced.)

**Dispatch gating is per-plate.** The orchestrator already resolves the
cascade per plate; it computes the plate's **active plugin set** =
plugins that are (a) healthy, (b) `plugin.<name>.enabled` resolves
`true`, (c) `printer_compatibility` matches the plate's printer model,
and hands that set to `host.dispatch`. This unifies enablement +
printer enforcement in one place. `printer_compatibility` is now
**enforced** (was informational + a Lua self-guard). pre-send has no
per-plate cascade context, so it gates on health + global-default
enabled + printer (driver_kind→model); per-plate enable doesn't apply to
a whole-job send (document the boundary).

**Persistence.**
- **global** default: a new plugin-state file in the data dir
  (`data_dir`), per-plugin `{ enabled, settings }`. Replaces today's
  session-only in-memory default (which is why a dropped-in plugin
  silently re-enabled every restart). Written by the panel toggle / the
  global settings edit.
- **project / plate**: ride the existing `.3mf` (`Project.user_overrides`
  / `Plate.project_overrides`) as `plugin.<name>.enabled` +
  `plugin.<name>.<key>` keys.

**Build order (this ticket now splits — bigger than the original ~3d):**
1. ✅ Manifest `scopes` field (vocab + parse + validate + surface).
2. ✅ Enablement-as-cascade-setting + per-plate active-set gating in the
   orchestrator + `printer_compatibility` enforcement. Activation is
   resolved **directly from the override tiers** (`plugin.<name>.enabled`,
   user < project < object precedence, default true) rather than through
   `cascade::resolve` — a plugin key has no authored cascade rule and
   must not reach the libslic3r adapter. A per-plate `DispatchGate`
   (printer model + activation map) gates `dispatch`/`any_hook`; the
   plain methods stay as permissive wrappers. printer_compatibility is
   enforced for pre/post-slice (model from the slice context) and
   pre-send (model resolved from the plate's instance). pre-send carries
   no per-plate activation (a send is whole-job) — that and the global
   tier land in step 3.
3. **Global enable/disable layer — ✅ done (settings-value half open).**
   The global activation tier now lives in
   `$XDG_CONFIG_HOME/n3o-slic3r/config.toml` (`core/config.rs`,
   `[plugins.enabled]` name→bool). The host holds a `global_enabled`
   map, seeded from config at startup (`lib.rs`) and updated by the
   `plugin_set_global_enabled` command (writes config + emits
   `plugin:changed`). `is_active` resolves **override tier → global tier
   → manifest default (true)**, so a globally-disabled plugin is skipped
   on **both** slice and send (the step-2 "still runs on send" gap is
   closed — pre-send dispatch goes through the host, which applies the
   global tier). `PluginSummary` gained `globally_enabled` (the panel
   toggle's state, separate from the `enabled` health flag).
   - **Still open:** resolved plugin-*setting values* handed to hooks
     (replacing the manifest-default reads in 8-5..8-7), and global
     plugin *settings* in `config.toml`. **Unify, don't duplicate:**
     step 2's `resolve_plugin_activation` hand-rolls the
     user→project→object tier-walk + TOML parse for the `.enabled` flag;
     typed settings ride the *same* tiers — extract one `plugin.*`
     resolver returning a typed map (with `.enabled` as one consumer)
     rather than copying the walk.

### Settings-cascade model (decided 2026-05-31)

Activation is **tri-state per level** — `global` is binary (`on`/`off`);
`project` and `plate` are `inherit` / `on` / `off`. Levels map to:
`global` = `config.toml`, `project` = the cascade *user* tier
(`Project.user_overrides`), `plate` = the cascade *project* tier
(`Plate.project_overrides`). The object tier is **not** a plugin level
(dropped). In the flat override representation, `inherit` = the
`plugin.<name>.enabled` key is **absent** at that tier.

- **Effective activation** (does it run): the first explicit
  (non-`inherit`) value walking **plate → project → global**, default
  `on`. (The dispatch gate already computes this.)
- **Settings promotion (enforced backend-side, not trusting the UI):** a
  level's `plugin.<name>.<key>` values overlay the resolved settings
  **only where that level's activation is explicitly `on`** — `inherit`
  and `off` levels never promote settings, *even when the plugin is
  effectively running via inheritance*. Base = the manifest defaults;
  overlay order `global → project → plate` (finest on-level wins). So to
  attach a per-plate setting value, the plate's activation must be set
  `on` (not left `inherit`). The UI gates which controls it offers by
  scope/activation; the backend independently enforces the same rule.

4. Frontend: Plugins panel + settings category + per-scope controls.
   `plugin_set_global_enabled` is the panel toggle's backend. The UI
   gates settings entry by per-level activation; the backend enforces it
   regardless (see the settings-cascade model above).

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
