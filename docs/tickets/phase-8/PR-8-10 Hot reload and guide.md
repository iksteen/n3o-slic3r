# PR-8-10 — plugin authoring guide + exit-criteria smoke

Status: ❌ open.

> **Hot reload descoped (2026-05-31).** The automatic file-watcher hot
> reload that this ticket originally owned (FR-PL-8) is **deferred to
> post-MVP** — see `phase-8.md` scope decision 3 and
> `Execution_Plan.md` §16. This ticket now ships only the authoring
> guide + exit smoke. The **manual** `plugin_reload` command (PR-8-3,
> the errored-plugin recovery path, surfaced in the Plugins panel)
> stays; only the `notify`-based folder watcher is cut. The filename
> retains "Hot reload" to keep existing links stable.

**Scope.** Close the phase: a plugin authoring guide and an
exit-criteria smoke chaining the phase's (software) deliverables into
one repeatable gate.

**Acceptance criteria.**

- **Plugin authoring guide** (`docs/plugin-authoring.md`):
  - The plugin layout (`plugin.toml` + entry `.lua`), the manifest
    fields (incl. `scopes`, `printer_compatibility`,
    `enabled_by_default`, `[settings.*]`), and the three hooks
    (pre-slice / post-slice / pre-send) with their payload shapes.
  - The typed G-code API (line kinds, read fields, insert/replace/
    remove/append), the read-only filament API, and the read-only
    `settings` global.
  - Walks through writing each bundled example (beep-at-layer,
    pause-at-layer, rewrite-bed-temp-by-range, platecycler) from
    scratch.
  - States the sandbox limits plainly (no io/os/network; no
    `rawset`/`getmetatable`/`setmetatable`; instruction + memory
    budgets) so authors aren't surprised.
  - States the **load/activation model**: plugins are discovered on
    launch, are **off by default** (opt-in unless the manifest sets
    `enabled_by_default = true`), and enable via the Plugins panel
    (global) or the per-project / per-plate surfaces. A dropped-in or
    edited plugin is picked up on the next launch or via a manual
    reload — automatic hot reload is post-MVP.
  - Does **not** reference the deferred compose hook or hot reload as
    available.

- **Exit-criteria smoke** (`docs/phase-8-smoke.md` + an integration
  test where automatable): chains the phase exit criteria —
  1. drop/point at an example plugin → it's discovered + loaded,
  2. enable it (global tier) and a slice runs it at post-slice; the
     output reflects it (verify-via-G-code),
  3. a per-plate `off` override suppresses it; re-enabling restores it,
  4. a deliberately-broken plugin is caught + surfaced without crashing
     the host (and `plugin_reload` recovers it once the file is fixed).
  The platecycler hardware proof lives in its own doc (PR-8-7); this
  smoke covers the software exit criteria.

- Tests:
  - The exit-smoke chain above as an integration test (a temp plugins
    root + a real slice), where automatable.
  - `plugin_reload` recovers a plugin after its on-disk file is fixed
    (the manual-reload path; the existing `reload_picks_up_an_edit`
    test already covers the swap).

**Effort.** ~1 day (was ~2; the hot-reload watcher is cut).

**Dependencies.** PR-8-2 (discovery), PR-8-3 (manual reload path +
events), PR-8-9 (panel + activation surfaces). Authoring guide depends
on the example plugins from PR-8-5/8-6/8-7 existing.

**Out of scope.**

- **Automatic hot reload** (the `notify` folder watcher) — deferred to
  post-MVP (FR-PL-8; `Execution_Plan.md` §16).
- Signed / verified plugins, a plugin registry, version pinning — all
  post-MVP.
- Watching for changes to plugin *settings* (those flow through the
  cascade / `config.toml`, not a file watcher).
