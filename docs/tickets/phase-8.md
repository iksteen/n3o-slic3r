# Phase 8 — tickets

Phase 8 (Lua plugin system, ~4 person-weeks in the plan, less the
deferred compose hook) is the **extensibility phase** — the PRD's
"users and the project lead can reshape G-code and the slice/send
pipeline without touching Rust." Source: `docs/Execution_Plan.md` §10,
PRD §6.9 (FR-PL-1..9). Stated goal:

> Working Lua plugin host with hot reload.

Phase 8 has **no printer dependency for most of its surface** and the
plan notes it can run in parallel with Phase 7. In practice it lands
after 7c: the read-only filament API (PR-8-8) binds Phase 7c's
filament-state model, and the flagship platecycler plugin's hardware
smoke needs Phase 7a's A1 mini send path. The hook infrastructure,
G-code bindings, and example plugins have no such dependency and can
start immediately.

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

Net effect on the phase: the compose-hook ticket and the 3MF
compose-API work are dropped (~3–4 days saved); the platecycler
ticket shrinks from "port the Python pipeline" to "append a
configurable macro block."

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
hooks, the flagship plugin, the filament API, the settings UI, hot
reload.

## Sequencing

`8-1 → 8-2 → 8-3 → 8-4 → 8-5` is the spine (foundation → manifest →
host/dispatch → G-code bindings → post-slice wired). After the slice
closes at 8-5, the rest are largely independent:

- **8-6** (pre-slice + pre-send) extends dispatch — needs 8-5's host.
- **8-7** (platecycler) needs 8-5 (post-slice) + **Phase 7a** for the
  real-hardware smoke.
- **8-8** (filament bindings) needs **Phase 7c**'s filament-state model.
- **8-9** (settings UI + Plugins panel) needs 8-2's manifest settings
  declarations; frontend.
- **8-10** (hot reload + authoring guide + exit smoke) last.

## Status by deliverable

| Deliverable | Status | Ticket |
|-------------|--------|--------|
| mlua (Lua 5.4, vendored) + sandboxed runtime + chunk load/call | ✅ done | [PR-8-1](phase-8/PR-8-1%20mlua%20foundation.md) |
| Plugin manifest (TOML) schema + plugins-folder discovery + validation | ✅ done | [PR-8-2](phase-8/PR-8-2%20Manifest%20and%20discovery.md) |
| `PluginHost`: load manifested plugins, dispatch hooks, error isolation | ❌ open | [PR-8-3](phase-8/PR-8-3%20Plugin%20host%20and%20dispatch.md) |
| Typed G-code Lua bindings (lines/layers/commands, insert/replace/remove/append) | ❌ open | [PR-8-4](phase-8/PR-8-4%20Gcode%20bindings.md) |
| post-slice hook wired into the slice pipeline + beep/pause example plugins | ❌ open | [PR-8-5](phase-8/PR-8-5%20Post-slice%20hook.md) |
| pre-slice + pre-send hooks wired + bed-temp-by-range example | ❌ open | [PR-8-6](phase-8/PR-8-6%20Pre-slice%20and%20pre-send.md) |
| **platecycler plugin** (post-slice macro append) + A1 mini hardware smoke | ❌ open | [PR-8-7](phase-8/PR-8-7%20platecycler%20plugin.md) |
| Read-only filament-state Lua bindings (per-slot identity/loaded/mismatch) | ❌ open | [PR-8-8](phase-8/PR-8-8%20Filament%20state%20bindings.md) |
| Plugin-declared settings in the cascade UI + Plugins panel (frontend) | ❌ open | [PR-8-9](phase-8/PR-8-9%20Settings%20and%20panel.md) |
| Hot reload (folder watcher) + plugin authoring guide + exit-criteria smoke | ❌ open | [PR-8-10](phase-8/PR-8-10%20Hot%20reload%20and%20guide.md) |

## Exit criteria (Execution_Plan §10, adjusted)

- A non-Rust developer can take an example plugin, edit it, and have it
  active in under 60 seconds (hot reload, PR-8-10).
- A plugin error is caught and surfaced (Plugins panel) without
  crashing the host (PR-8-3 error isolation).
- The platecycler plugin appends its swap macro to a real A1 mini
  print's G-code such that the PlateCycler auto-ejects the finished
  plate on the project lead's hardware (PR-8-7). *This is the reduced
  proof point that replaces the original "compose hook produces a
  `.platecycler.3mf`" criterion.*

## Doc updates owed

Per PRD §11.3 (living documents), these source docs diverge from the
kickoff decisions above and get a deferral note when PR-8-1 lands:

- **PRD §6.9 FR-PL-5** — mark the compose hook deferred to post-MVP;
  note the MVP platecycler is a post-slice macro append.
- **`Execution_Plan.md` §10** — same; move "Compose hook API" and the
  multi-plate platecycler port to "What follows the MVP" (§16).
- **`docs/Execution_Plan.md` §16** — add compose hook + multi-plate
  platecycler to the post-MVP list.
