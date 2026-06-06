# Claude Code context for n3o-slic3r

This file captures the durable context any Claude Code session in this
repo should pick up immediately. Source-of-truth development documents
live in `docs/dev/` (user-facing docs — getting-started, troubleshooting,
release-notes — stay at the top of `docs/`); this file points at them and
records facts that have already been corrected once during prior sessions
and should not need re-correcting.

## What this project is

**n3o-slic3r** is a multi-printer-first desktop slicer UI built on
Tauri 2 + React + TypeScript, driving OrcaSlicer's `libslic3r` engine
through a vendored Rust + C FFI shim (`crates/slic3r-ffi/`). The MVP
targets two specific printers (Bambu Lab A1 mini and Snapmaker U1)
configured simultaneously, with full slice-and-send workflow, a
transparent settings cascade, and a Lua-based plugin system for G-code
manipulation.

## Source-of-truth documents

Read these before reasoning about scope, design, or behavior:

- **`docs/dev/PRD.md`** — Product requirements. Goals, non-goals, success
  criteria, feature requirements (FR-CAS-*, FR-MP-*, FR-UI-*, …),
  printer capability matrix, architecture decisions (AD-1..AD-7),
  working practices (§11).
- **`docs/dev/Execution_Plan.md`** — 10-phase plan, ~37.5 person-weeks.
  Phase ordering and dependencies are real; calendar dates are not.
  **Status (2026-06-07):** Phases 0–8 are done — the full vertical
  slice ships (cascade resolver + adapter, viewport, end-to-end slice +
  G-code parser + 3MF I/O, settings UI, multi-printer project model,
  G-code preview, both printer drivers + filament sync, Lua plugin
  system with platecycler hardware-validated). **Phase 9 (polish,
  Linux flatpak, release prep) is nearly complete** — done: flatpak
  build + self-hosted signed-repo distribution (validated on Arch,
  Ubuntu, and Fedora, including a full open→slice→print→monitor cycle on
  both printers under WSL2/WSLg), first-run onboarding, OrcaSlicer
  `.3mf` project import, user docs (getting-started / troubleshooting /
  release notes — PR-9-7), Linux CI. The **independence-audit exit
  gate** (PR-9-8) is **met** (2026-06-07): the clean-WSL2 full-cycle run
  on both printers plus an external (non-lead) tester reaching send in
  ~5 min prove "standalone at runtime" (no host slicer/libslic3r), and
  the §3.3 feature criteria are proven in-phase. The audit surfaced one
  finding — the Bambu **Developer Mode** requirement (recent firmware
  rejects third-party MQTT commands, err_code 84033543) wasn't
  discoverable in-app — now fixed (n3o surfaces the command-rejection
  `err_code` with Developer-Mode guidance). Remaining (see
  `docs/dev/tickets/phase-9.md`): the `.3mf`/`n3o_project.json`
  format-finalization review (PR-9-5). Post-MVP deferrals (plugin
  compose hook, hot reload, Orca preset importer) live in plan §16.
- **`docs/dev/profiles.md`** — Rule-cascade design of record. Two-phase
  resolution (authored cascade + `!important`-style override tiers),
  TOML schema, translation adapter to libslic3r's `DynamicPrintConfig`,
  option scope mechanism. The PRD's §6.1 codifies the requirements;
  this doc owns the design.
- **`docs/dev/design.md`** — Mockup review. What in `docs/dev/design/` is
  reusable as-is, what to port, what to replace. Known design gaps
  (plate-printer assignment UI, filament sync, G-code preview).
- **`docs/dev/libslic3r-workarounds.md`** — The set of pre-`apply` and
  post-`apply` workarounds the shim applies to compensate for
  libslic3r's headless-mode quirks. Required reading before bumping
  the OrcaSlicer submodule.

If any of these contradicts something the conversation infers from
code or upstream, the document wins until corrected — then update the
document, don't just nod and move on (PRD §11.3).

## Architecture principles

These are load-bearing rules. Violating one usually means a phase
later costs more than it had to.

- **3D scene state lives in Rust, not in the renderer.** The
  authoritative scene model (objects, transforms, mesh data,
  selection) is a renderer-agnostic data structure in `core/scene/`.
  Three.js is a read-only consumer that reflects state events into
  pixels. All scene mutations go through Tauri commands; the renderer
  never owns state. This is so we can swap renderers (Phase 2 risk:
  if webview 3D performance is insufficient, we switch to wgpu in a
  native window) without rewriting state management. See PRD FR-3D-7
  and AD-8 for the full design. **What is NOT in the scene model
  (ripped out as dormant view-state; re-add when actually wired):**
  - *Gizmo* — there is none. The active *transform mode*
    (translate/rotate/scale) is renderer-local UI state owned by
    `App`. The gizmo *pivot* override (`GizmoState`, `GizmoChanged`,
    `set_gizmo`) is gone; re-add a `core/scene` pivot field + setter
    command if a pivot-setting UI is built. (`rotate_object` still
    takes an optional explicit-pivot arg as a transform primitive.)
  - *Camera* — there is no `CameraState`/`ProjectionMode` in the
    scene model. The renderer owns its own Three.js camera and frames
    from the bed (`initialFrameForBed`); it never synced or restored
    a persisted camera. To add "restore per-plate view on reopen,"
    re-add a `core/scene` camera field + a `scene_camera_set` that the
    renderer commits on orbit-end and reads back on load.

- **Configs are pure data.** No embedded code, no expressions, no
  template strings. The rule cascade (PRD §6.1, docs/dev/profiles.md)
  handles conditional values declaratively. Lua exists for G-code
  post-processing plugins, not for configs.

- **Adapter layer owns libslic3r's vocabulary.** Above the adapter,
  the system speaks in our logical settings; below the adapter,
  libslic3r's flat `DynamicPrintConfig` and dispatch quirks
  (`curr_bed_type`, `wipe_tower`, dimensional key explosions) live
  contained. New libslic3r-specific quirks land in
  `docs/dev/libslic3r-workarounds.md` and the shim, not in higher layers.

- **Standalone at runtime.** The app must complete every workflow
  with no other slicer installed. UX principle from PRD §5.

## Facts that have already been corrected

These came up during planning and prior implementation sessions. Don't
reason from contrary priors.

- **Snapmaker U1 is a 4-toolhead CoreXY toolchanger** with magnetically
  docked toolheads on steel-ball kinematic couplings, eddy-current
  auto-alignment, ~5–10 s tool swaps. Klipper-based firmware. It is
  *not* IDEX. Each toolhead is independently replaceable, so each slot
  is a distinct cascade-layer entity with its own nozzle size, hotend
  type, and temperature range.
- **Bambu A1 mini uses AMS lite filament-swap** at a single hotend.
  Not a toolchanger. AMS lite reports per-slot filament identity
  (type, color, brand if reported) over MQTT.
- **Purging and priming are independent capabilities.** A1 mini
  purges (single hotend reuse) and uses a large priming tower that
  doubles as a purge structure. U1 does *not* purge (per-toolhead
  retained material) but still uses a small priming tower for
  toolhead re-entry stabilization. The PRD's printer-aware visibility
  rules (AD-1) gate purge-volumes-matrix on `purging_required`,
  priming-tower geometry on `priming_tower_used`. The two flags are
  independent.
- **Snapmaker Orca is a fork of OrcaSlicer**, not Cura. U1 ships
  supported by Snapmaker Orca; unmodified OrcaSlicer also works.
- **`orca-slicer-ffi` is owned in-house.** The FFI was authored by the
  project lead. It was its own GitHub repo
  (`github.com/iksteen/orca-slicer-ffi`) but is now vendored at
  `crates/slic3r-ffi/`. FFI extensions are first-party work, not
  external dependencies.
- **`platecycler` is owned in-house.** The MVP ships it as a Lua
  plugin using the **post-slice** hook — a macro append that
  auto-ejects the finished plate at print end (Phase 8 scope decision
  2). The originally-scoped compose hook (FR-PL-5) is deferred
  post-MVP.
- **The cascade resolver *is* built** (corrected 2026-05-30; this
  bullet previously said it wasn't). The rule-cascade resolver lives
  in `src-tauri/src/core/cascade/` (`resolver`, `loader`, `overrides`,
  `trace`, `validate`) and the libslic3r translation in
  `src-tauri/src/core/cascade_adapter/` (`adapter` does the
  `bed_temp` → per-plate-type dimensional expansion + `curr_bed_type`
  set). `tests/reference_profiles.rs` exercises it end-to-end. Design
  of record is still `docs/dev/profiles.md`.
  - **CONFIRMED (2026-05-31, PR-9-1):** the live *slice* path **does**
    route through the resolver + adapter, not the input's embedded
    config. `core/slice/orchestrator.rs::resolve_cascade` composes a
    fresh cascade from the bound `PrinterInstance`, `cascade::resolve`
    + `cascade_adapter::adapt` build the `DynamicPrintConfig`, and the
    `.3mf`/STL is loaded for **geometry only**. Verified via G-code
    (`tests/slice_orchestrator.rs::resolved_bed_temp_reaches_the_engine_for_both_printers`):
    slicing a raw STL (no embedded config to leak), the engine's body
    `M140`/`M190` carry the cascade-resolved `textured_plate_temp`,
    `curr_bed_type` is the context's plate type, and the two MVP
    printers resolve to different temps from their own fragments.
    The old "`hot_plate_temp=60` not `55`" observation was the *wrong
    key* (the active plate is Textured PEI → `textured_plate_temp`) and
    pre-dated the compose-context fix (310f7b6); the U1 snapmaker-pla
    `55` rule now fires at compose time, guarded by
    `composer::tests::u1_filament_fragment_printer_rule_fires_at_compose_time`.

## Project shape and build

Workspace at the repo root with two Rust crates plus a Tauri
frontend:

```
n3o-slic3r/
├── Cargo.toml                  # workspace
├── crates/slic3r-ffi/          # the FFI shim (build.rs invokes cmake)
│   ├── ffi/                    # C++ shim source
│   └── src/lib.rs              # safe Rust wrapper
├── src-tauri/                  # Tauri backend (depends on slic3r-ffi via path)
├── src/                        # React renderer
├── external/OrcaSlicer/        # submodule, pinned
└── docs/                       # user-facing docs; dev docs under docs/dev/
```

Build flow (Linux; macOS / Windows post-MVP):

```bash
git submodule update --init --recursive
./scripts/build.sh deps    # one-time, ~17 min — OrcaSlicer's deps tree
cargo build                # 15 min cold, fast incremental
npm install && npm run tauri dev
```

`scripts/build.sh deps` is the only step that isn't driven by `cargo`.
After that, the slic3r-ffi crate's `build.rs` invokes cmake to build
`libslic3r_ffi.so` and that's the heaviest step in a `cargo build`.

## Licensing and product principles

- **License: AGPL-3.0-or-later.** Forced by the libslic3r linkage. Any
  derivative must remain AGPL-compatible. Plugins running in the Lua
  sandbox are a separate licensing question (PRD §10).
- **No telemetry, no analytics, no network calls** except to
  user-configured printers. This is a product principle, not a
  default to be overridden (PRD §11.5).
- **Standalone at runtime.** The app must be fully functional without
  any other slicer (OrcaSlicer, PrusaSlicer, etc.) installed. UX
  principle (PRD §5).

## Working practices for build sessions

PRD §11 spells these out. The short version:

- **Verify before coding.** Code that depends on a hardware/protocol/
  library claim must cite a verifiable source — published docs, a
  referenced community library, or a test that confirms behavior.
  "Claude said so" is not a source (PRD §11.1).
- **Spike before committing.** Phase 0.5 names five spikes. The
  pattern continues — one day of focused experiment routinely saves
  a week of debugging (PRD §11.2).
- **Living documents.** PRD, plan, profiles.md, design.md, this
  CLAUDE.md are not commitments frozen at kickoff. Update them when
  reality diverges, commit the doc change alongside the code change
  (PRD §11.3).
- **Correction posture.** The project lead corrects detail-level
  reasoning without expecting pushback. Incorporate corrections, then
  propagate them to persistent context — don't just nod (PRD §11.4).

## Memory

Per-session memories live in
`~/.claude/projects/-home-ingmar-src-prive-n3o-slic3r/memory/`. Currently:

- `feedback_stop_chasing_bug_chains.md` — when a fix unmasks another
  instance of the same pattern, surface options instead of patching
  deeper.
- `feedback_ask_before_filesystem_search.md` — don't `find ~` for
  tooling of uncertain existence; ask the user.

The memory directory is keyed by the working directory Claude Code
launched from. If the project moves on disk, the memory must move too
(see "Build & path-operation gotchas" in `README.md`).
