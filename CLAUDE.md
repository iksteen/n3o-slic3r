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
manipulation. It also carries a profile for the full-size **Bambu Lab
A1** — same Bambu MQTT driver as the mini, but a full 4-spool AMS
(`ams_type = "AMS"`, chainable to 16) instead of the mini's AMS lite.
The A1 mini + U1 remain the hardware-validated pair; the A1 profile is
bundled and selectable (`resources/profiles/bbl/printer/bambu-lab-a1/`).

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
  Linux flatpak, release prep) is complete — the MVP candidate is
  reached.** Done: flatpak
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
  `err_code` with Developer-Mode guidance). The
  `.3mf`/`n3o_project.json` format is **finalized for MVP** (PR-9-5,
  2026-06-07): the field-by-field review landed — derived/transient
  state dropped, logical-key overrides, writer stamp, UUID group
  identity. With that, **all of Phase 9 is done.** Post-MVP deferrals (plugin
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

Build flow (Linux and macOS native; Windows is cross-from-Linux):

```bash
git submodule update --init --recursive
./scripts/build.sh deps    # one-time — OrcaSlicer's deps tree
cargo build                # 15 min cold, fast incremental
npm install && npm run tauri dev
```

`scripts/build.sh deps` is the only step that isn't driven by `cargo`.
After that, the slic3r-ffi crate's `build.rs` invokes cmake to build
`libslic3r_ffi.{so,dylib}` and that's the heaviest step in a `cargo build`.

**Platform notes for the deps + FFI build:**

- **Linux** drives `external/OrcaSlicer/build_linux.sh` and installs the
  deps prefix flat at `deps/build/OrcaSlicer_dep/usr/local`.
- **macOS** drives `build_release_macos.sh -d -a <arch>`, which
  **arch-namespaces** the prefix at `deps/build/<arch>/OrcaSlicer_dep/usr/local`
  so arm64 and x86_64 trees coexist. `scripts/build.sh` and
  `crates/slic3r-ffi/build.rs` branch on platform/arch and select the right
  prefix; on macOS `build.rs` passes `-DCMAKE_PREFIX_PATH`,
  `-DCMAKE_OSX_ARCHITECTURES=<arch>` (matching the cargo target), and
  `-DCMAKE_OSX_DEPLOYMENT_TARGET=11.3` (matching the deps), and namespaces
  its own cmake build dir as `build/slic3r-ffi-<arch>`. Prereqs come from
  Homebrew: `cmake ninja pkg-config gettext libtool automake autoconf
  texinfo node`, plus Rust via `rustup`.
  - **Native** (Apple Silicon, hardware-validated): `./scripts/build.sh deps`
    then `cargo build` / `npm run tauri build`.
  - **Intel cross from Apple Silicon** (validated — runs under Rosetta):
    `rustup target add x86_64-apple-darwin` + Rosetta
    (`softwareupdate --install-rosetta`), then
    `./scripts/build.sh deps x86_64` (cross-builds the Intel deps) and
    `npm run tauri build -- --target x86_64-apple-darwin`. Output lands under
    `target/x86_64-apple-darwin/release/bundle/`. A `universal` lipo'd build
    is the obvious next step but is not wired up yet.
- **Windows** is the cross-from-Linux path (`cargo xwin` + the
  `packaging/windows-cross/` toolchain); see `crates/slic3r-ffi/build.rs`.
- **macOS can also cross from Linux** via osxcross (`packaging/macos-cross/`).
  Same shape as the Windows cross: `build-deps.sh <arch>` rebuilds the dep tree
  with the osxcross toolchain into the arch-namespaced
  `deps/build/<arch>/OrcaSlicer_dep/usr/local` prefix the native build also
  uses, then `build.sh <arch> cargo build … --target <arch>-apple-darwin`
  builds the engine + shim + app (build.rs injects the osxcross toolchain when
  it sees a macOS target on a non-macOS host). **Validated (2026-06-13):** the
  full chain — deps → libslic3r → `libslic3r_ffi.0.dylib` → the `n3o-slic3r`
  app binary — cross-compiles and links to arm64 Mach-O from Linux. The
  remaining gap is `.app`/`.dmg` bundling: Tauri's macOS bundler + `codesign`
  are macOS-only, so the Linux cross currently yields the binary + dylib, not a
  packaged `.app`. See `packaging/macos-cross/README.md`.

**macOS `.app` bundling** (`npm run tauri build`): the engine ships as a
dylib, so the bundle must carry it and be re-signed to load it.
`src-tauri/tauri.macos.conf.json` (auto-merged by the Tauri CLI, same as
`tauri.windows.conf.json`) copies `libslic3r_ffi.0.dylib` into
`Contents/Frameworks` and ad-hoc signs (`signingIdentity: "-"`). Because the
cmake build dir is arch-namespaced but the config path must be static,
`build.rs` maintains a `build/slic3r-ffi-current` symlink pointing at the
arch it just built; the config embeds the dylib through that symlink, so a
native and a `--target x86_64-apple-darwin` build each bundle the matching
dylib without a per-arch config.
`src-tauri/build.rs` adds an `@executable_path/../Frameworks` rpath so the
relocated app finds it; and `src-tauri/entitlements.macos.plist` sets
`com.apple.security.cs.disable-library-validation` — **required**, because
the hardened runtime otherwise refuses to load an ad-hoc-signed dylib that
doesn't share the main executable's Team ID. The result is a relocatable,
ad-hoc-signed `.app` + `.dmg`; Gatekeeper still rejects it on download
(no Developer-ID notarization — needs a paid Apple account).

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

Per-session memories live in a directory keyed by the working directory
Claude Code launched from. The repo has been developed from more than one
checkout location, so this path is host-specific — e.g.
`~/.claude/projects/-Users-ingmar-Documents-GitHub-n3o-slic3r/memory/` on
the macOS checkout, `~/.claude/projects/-home-ingmar-src-prive-n3o-slic3r/memory/`
on the original Linux one. Memories seen so far:

- `feedback_stop_chasing_bug_chains.md` — when a fix unmasks another
  instance of the same pattern, surface options instead of patching
  deeper.
- `feedback_ask_before_filesystem_search.md` — don't `find ~` for
  tooling of uncertain existence; ask the user.

The memory directory is keyed by the working directory Claude Code
launched from. If the project moves on disk, the memory must move too
(see "Build & path-operation gotchas" in `README.md`).
