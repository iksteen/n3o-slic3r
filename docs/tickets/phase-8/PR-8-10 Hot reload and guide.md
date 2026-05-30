# PR-8-10 — hot reload + authoring guide + exit-criteria smoke

Status: ❌ open.

**Scope.** Close the phase: a file watcher that reloads plugins on
change (the "active in under 60 seconds" author loop), a plugin
authoring guide, and an exit-criteria smoke chaining the phase's
deliverables into one repeatable gate.

Owns **FR-PL-8** (hot reload) plus the phase's documentation + exit
smoke.

**Acceptance criteria.**

- **Hot reload** (`core/plugin/watcher.rs`):
  - A watcher (the `notify` crate) on the plugins root. On a relevant
    change (manifest or entry `.lua` created/modified/removed),
    re-run discovery for the affected plugin and reload it through the
    same path as the manual `plugin_reload` (PR-8-3), then emit
    `plugin:changed` so the panel (PR-8-9) refreshes.
  - Debounced (coalesce a burst of editor saves into one reload) and
    resilient: a reload that fails leaves the plugin in the errored
    state, not the host crashed; the previous good version keeps
    running until a successful reload replaces it.
  - A newly-added plugin directory is picked up without restart; a
    removed one is dropped from dispatch.
  - Reload swaps the `PluginRuntime` atomically with respect to
    dispatch (a slice in flight either sees the old or new plugin, not
    a half-loaded one) — document the locking.

- **Plugin authoring guide** (`docs/plugin-authoring.md`):
  - The plugin layout (`plugin.toml` + entry `.lua`), the manifest
    fields, and the three hooks (pre-slice / post-slice / pre-send)
    with their payload shapes.
  - The typed G-code API (line kinds, read fields, insert/replace/
    remove/append) and the read-only filament API.
  - Walks through writing each bundled example
    (beep-at-layer, pause-at-layer, rewrite-bed-temp-by-range,
    platecycler) from scratch.
  - States the sandbox limits plainly (no io/os/network; instruction +
    memory budgets) so authors aren't surprised.
  - Does **not** reference the deferred compose hook as available.

- **Exit-criteria smoke** (`docs/phase-8-smoke.md` + an integration
  test where automatable): chains the phase exit criteria —
  1. drop/point at an example plugin → it's discovered + loaded,
  2. a slice runs it at post-slice and the output reflects it
     (verify-via-G-code),
  3. edit the plugin file → hot reload picks it up (the <60s loop),
  4. a deliberately-broken plugin is caught + surfaced without
     crashing the host.
  The platecycler hardware proof lives in its own doc (PR-8-7); this
  smoke covers the software exit criteria.

- Tests:
  - Watcher fires a reload on a modified entry file (drive with a
    temp plugins dir + a real file write; gate timing loosely).
  - A failing reload preserves the prior good runtime.
  - Add/remove of a plugin dir updates the dispatch set.

**Effort.** ~2 days.

**Dependencies.** PR-8-2 (discovery), PR-8-3 (reload path + events),
PR-8-9 (panel consumes the refresh). Authoring guide depends on the
example plugins from PR-8-5/8-6/8-7 existing.

**Out of scope.**

- Signed / verified plugins, a plugin registry, version pinning — all
  post-MVP.
- Watching for changes to plugin *settings* (those flow through the
  cascade, not the file watcher).
