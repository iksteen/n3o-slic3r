# Phase 8 — tickets

Phase 8 (Lua plugin system, ~4 person-weeks in the plan, less the
deferred compose hook) is the **extensibility phase** — the PRD's
"users and the project lead can reshape G-code and the slice/send
pipeline without touching Rust." Source: `docs/Execution_Plan.md` §10,
PRD §6.9 (FR-PL-1..9). Stated goal:

> Working Lua plugin host. *(Automatic hot reload deferred to post-MVP
> — see scope decision 3.)*

Phase 8 has **no printer dependency for most of its surface** and the
plan notes it can run in parallel with Phase 7. The flagship platecycler
plugin's hardware smoke needs Phase 7a's A1 mini send path. The
read-only filament API (PR-8-8) was originally scoped to Phase 7c's live
filament-state model, but shipped **reduced** to the slice-time bound
loadout (the `PrinterInstance` material→slot mapping), which has no 7c
dependency; the live `loaded`/`mismatch` surface is a post-7c-4
follow-up. The hook infrastructure, G-code bindings, and example
plugins likewise can start immediately.

Individual tickets live one-per-file in `phase-8/`. This file is the
index plus phase-level status and the two kickoff scope decisions.

## Scope decisions (2026-05-30)

Two decisions taken at phase kickoff that **diverge from
`Execution_Plan.md` §10 / PRD §6.9** and need the source docs updated
(see [Doc updates owed](#doc-updates-owed)):

1. **Compose hook (FR-PL-5) deferred to post-MVP.** The plan's
   project-level compose hook ("receives all sliced plates + project
   metadata, returns a transformed bundle, may change plate count")
   existed *specifically* to support a multi-plate batch platecycler.
   With platecycler redefined (decision 2) as a post-slice macro
   append, the compose hook has **no MVP consumer** — and building a
   cross-plate transform mechanism with nothing to validate it against
   is over-engineering for the MVP. The hooks that ship are
   **pre-slice, post-slice, pre-send**. Compose moves to the
   post-MVP list.

2. **platecycler is a post-slice macro append, not a multi-plate
   batch.** Instead of porting the Python tool's
   concatenate-N-plates-into-one-job behavior, the MVP platecycler
   plugin **appends the Chitu PlateCycler eject/swap macro**
   (`DEFAULT_SWAP_GCODE`, characterized in
   `docs/spikes/spike-5-platecycler.md`) to the **tail of a single
   plate's G-code**. When that print finishes, the finished plate is
   auto-ejected and a fresh one loaded — ready for the next print.
   This is a per-plate **post-slice** hook: no cross-plate
   composition, no Python/Pillow runtime dependency, no `.gcode.3mf`
   re-wrapping.

3. **Hot reload (FR-PL-8) deferred to post-MVP (2026-05-31).** The
   automatic `notify`-based folder watcher that reloads plugins on file
   change is cut from the MVP. Plugins load on launch; the **manual**
   `plugin_reload` command (also the errored-plugin recovery path)
   stays, and the Plugins panel exposes it. PR-8-10 keeps the authoring
   guide + exit smoke; the watcher moves to the §16 post-MVP list. The
   "edit → active in under 60 seconds" loop returns with it.

Net effect on the phase: the compose-hook ticket and the 3MF
compose-API work are dropped (~3–4 days saved); the platecycler
ticket shrinks from "port the Python pipeline" to "append a
configurable macro block"; PR-8-10 loses the hot-reload watcher.

## Hooks in scope

| Hook | Fires | Plugin sees | Can change |
|------|-------|-------------|------------|
| pre-slice | before the cascade adapter hands config to libslic3r | resolved settings (read) | settings values |
| post-slice | after each plate slices, before send/preview | the plate's typed G-code + plate/printer metadata | the G-code (insert/replace/remove/append) |
| pre-send | before a driver sends a payload | the per-printer send payload | the payload bytes |
| ~~compose~~ | ~~after all plates slice~~ | — | **deferred (post-MVP)** |

## Vertical slice

The phase closes its first end-to-end slice at **PR-8-5**: a real
`.lua` file, discovered from the plugins folder via its TOML manifest,
loaded into the sandboxed host, runs at **post-slice** and modifies
real libslic3r-emitted G-code through the **typed** model — verified
by re-slicing and grepping the output (green unit tests alone don't
prove libslic3r accepted the change). Everything after 8-5 deepens: more
hooks, the flagship plugin, the filament API, and the settings UI.

## Sequencing

`8-1 → 8-2 → 8-3 → 8-4 → 8-5` is the spine (foundation → manifest →
host/dispatch → G-code bindings → post-slice wired). After the slice
closes at 8-5, the rest are largely independent:

- **8-6** (pre-slice + pre-send) extends dispatch — needs 8-5's host.
- **8-7** (platecycler) needs 8-5 (post-slice) + **Phase 7a** for the
  real-hardware smoke.
- **8-8** (filament bindings) — **reduced** to the slice-time
  material→slot mapping (the bound `PrinterInstance` loadout), which
  removed the Phase 7c dependency. The live `loaded`/`mismatch` surface
  is a post-7c-4 follow-up. See the ticket's scope decision.
- **8-9** (settings UI + Plugins panel) needs 8-2's manifest settings
  declarations; frontend.
- **8-10** (authoring guide + exit smoke) last. **Hot reload is
  deferred** to post-MVP (scope decision 3); only the manual
  `plugin_reload` ships.

## Status by deliverable

| Deliverable | Status | Ticket |
|-------------|--------|--------|
| mlua (Lua 5.4, vendored) + sandboxed runtime + chunk load/call | ✅ done | [PR-8-1](phase-8/PR-8-1%20mlua%20foundation.md) |
| Plugin manifest (TOML) schema + plugins-folder discovery + validation | ✅ done | [PR-8-2](phase-8/PR-8-2%20Manifest%20and%20discovery.md) |
| `PluginHost`: load manifested plugins, dispatch hooks, error isolation | ✅ done | [PR-8-3](phase-8/PR-8-3%20Plugin%20host%20and%20dispatch.md) |
| Typed G-code Lua bindings (lines/layers/commands, insert/replace/remove/append) | ✅ done | [PR-8-4](phase-8/PR-8-4%20Gcode%20bindings.md) |
| post-slice hook wired into the slice pipeline + beep/pause example plugins | ✅ done | [PR-8-5](phase-8/PR-8-5%20Post-slice%20hook.md) |
| pre-slice + pre-send hooks wired + bed-temp-by-range example | ✅ done | [PR-8-6](phase-8/PR-8-6%20Pre-slice%20and%20pre-send.md) |
| **platecycler plugin** (post-slice macro append) + A1 mini hardware smoke | ✅ done | [PR-8-7](phase-8/PR-8-7%20platecycler%20plugin.md) |
| Read-only filament-loadout Lua bindings (slice-time material→slot mapping) | ✅ done | [PR-8-8](phase-8/PR-8-8%20Filament%20state%20bindings.md) |
| Plugin-declared settings in the cascade UI + Plugins panel (frontend) | ✅ done | [PR-8-9](phase-8/PR-8-9%20Settings%20and%20panel.md) |
| Plugin authoring guide + exit-criteria smoke (hot reload deferred post-MVP) | ✅ done | [PR-8-10](phase-8/PR-8-10%20Hot%20reload%20and%20guide.md) |

> **Phase 8 complete (2026-05-31).** PR-8-9 shipped much larger than its
> original ticket — a three-tier (global → project → plate) activation +
> settings cascade with tri-state per-level enablement, opt-in
> (off-by-default) plugins, `printer_compatibility` enforcement, global
> state in `config.toml`, and the three UI surfaces (brand-menu Global,
> project-menu Project, settings-panel Plate tab). The project lead
> confirmed the **UI** spot-check and the **platecycler hardware smoke**
> (auto-eject on the A1 mini + PlateCycler). PR-8-10 closed the phase
> with the **authoring guide** (`docs/plugin-authoring.md`) and the
> **exit smoke** (`docs/phase-8-smoke.md` + automated chain in
> `plugin_post_slice.rs`). **Hot reload (FR-PL-8) is deferred to
> post-MVP** (scope decision 3) — the manual `plugin_reload` ships.

## Review pass (after PR-8-5)

A code review over PR-8-1…8-5 surfaced ten findings; nine were fixed,
one deferred:

- **Robustness:** all plugin mutex locks are now poison-tolerant
  (recover the guard) and `apply_post_slice` holds the host lock only
  for the Lua dispatch (file I/O + parse/serialize run unlocked) — a
  panicking plugin can no longer wedge the host or block plugin UI
  commands during file work.
- **Correctness:** `g:layers()` recomputes positions live, so inserting
  at multiple layers while iterating stays aligned; inserting after an
  unterminated final line no longer merges them; `validate_entry` now
  `canonicalize()`s the entry and rejects symlinks escaping the plugin
  dir.
- **UX:** re-enabling a plugin clears its stale `last_error`; non-UTF-8
  output gets a clear "plugins skipped" warning.
- **Cleanup:** shared `core::paths::data_dir()` (autosave + plugins) and
  a `resource_root()` helper in `lib.rs`; the G-code bindings use new
  `Comment::new`/`Other::new`/`Line::set_line_ending` instead of reaching
  into model internals.
- **Deferred (#10):** `apply_post_slice` does a full read→parse→
  serialize round-trip and clones the lines per plugin — heavy on a
  50 MB plate. Noted in code; optimize if large multi-material jobs
  feel it.

An unbiased **second pass** (fresh reviewers, post-hardening) caught
more, now fixed:
- A panic inside post-slice plugin Lua used to unwind the worker thread
  silently (no terminal event → UI stuck "Running", temp file leaked).
  `apply_post_slice` is now wrapped in `catch_unwind`: a panic leaves
  the plate's unmodified G-code and the slice completes normally.
- The G-code rewrite is now **atomic** (write sibling temp + rename), so
  a failed/partial write can't leave a truncated `.gcode` that the
  summary/preview consume as a finished slice.
- The live-recompute `g:layers()` is correct for ordinary inserts but
  not for inserting/removing `LayerChange` lines mid-iteration — the
  comment now scopes that honestly (was overclaiming).
- Tests gained: a real-output parse→serialize **byte-identity** check
  (proves the no-op passthrough on actual libslic3r G-code), a
  **multi-plate plugin-error isolation** test, an `M0` negative control,
  a ≥2-layer fixture guard, and unique temp dirs.
- Example plugins dropped their **dead `[settings.layer]`** block (the
  Lua hardcodes the layer; the knob did nothing) — re-add when PR-8-9
  wires plugin settings.

The **PR-8-8 review** surfaced a sandbox gap (now closed): the read-only
metatable guards on host objects (the filament snapshot tables) could be
bypassed because the base library still exposed `rawset` /
`getmetatable` / `setmetatable`. `rawset` writes past `__newindex`;
`getmetatable`/`setmetatable` could read or replace a guard metatable.
The sandbox now strips all three (`sandbox.rs`), so metamethod-based
read-only enforcement is unbypassable by construction — a binding stays
safe even if it forgets a per-object `__metatable` lock. `rawget` /
`rawequal` / `rawlen` stay (no bypass value). The `settings` userdata
was already immune (`rawset` errors on userdata); this hardens the
table-based bindings.

## Exit criteria (Execution_Plan §10, adjusted) — all met

The software chain is automated as the **exit smoke**
(`docs/phase-8-smoke.md`; `phase_8_exit_smoke` +
`plugin_reload_recovers_a_broken_plugin` in `plugin_post_slice.rs`).

- ✅ A non-Rust developer can take an example plugin, edit it, drop it in
  the plugins folder, and enable it from the Plugins panel — active on
  the next launch or via a manual reload (PR-8-10;
  `docs/plugin-authoring.md`). *(The automatic "under 60 seconds" loop
  returns with post-MVP hot reload — scope decision 3.)*
- ✅ A plugin error is caught and surfaced (Plugins panel) without
  crashing the host (PR-8-3 error isolation;
  `plugin_reload_recovers_a_broken_plugin`).
- ✅ The platecycler plugin appends its swap macro to a real A1 mini
  print's G-code such that the PlateCycler auto-ejects the finished
  plate on the project lead's hardware (PR-8-7, confirmed 2026-05-31).
  *This is the reduced proof point that replaces the original "compose
  hook produces a `.platecycler.3mf`" criterion.*

## Doc updates owed

Per PRD §11.3 (living documents), these source docs diverge from the
kickoff decisions above and get a deferral note when PR-8-1 lands:

- **PRD §6.9 FR-PL-5** — mark the compose hook deferred to post-MVP;
  note the MVP platecycler is a post-slice macro append.
- **`Execution_Plan.md` §10** — same; move "Compose hook API" and the
  multi-plate platecycler port to "What follows the MVP" (§16).
- **`docs/Execution_Plan.md` §16** — add compose hook + multi-plate
  platecycler to the post-MVP list.

**Done 2026-05-31 (hot-reload deferral, scope decision 3):** PRD
FR-PL-8 marked deferred (+ MVP goals / §3.3 success criterion
reframed); `Execution_Plan.md` §10 goal/deliverable/exit-criteria
updated and FR-PL-8 added to §16; the phase-8 status table + exit
criteria + PR-8-10/PR-8-9/PR-8-3/PR-8-2 cross-refs updated.
