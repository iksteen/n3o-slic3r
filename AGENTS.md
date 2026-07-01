# AGENTS.md — project context + operational runbook for n3o-slic3r

This file captures the durable context any agent session in this repo
should pick up immediately, plus the practical runbook for *operating* the
app (launching it, capturing screenshots headlessly). Source-of-truth
development documents live in `docs/dev/` (user-facing docs —
getting-started, troubleshooting, release-notes — stay at the top of
`docs/`); this file points at them and records durable domain facts that
aren't obvious from the code.

## What this project is

**n3o-slic3r** is a multi-printer-first desktop slicer UI built on
Tauri 2 + React + TypeScript, driving OrcaSlicer's `libslic3r` engine
through a vendored Rust + C FFI shim (`crates/slic3r-ffi/`). It runs two
specific printers (Bambu Lab A1 mini and Snapmaker U1) configured
simultaneously, with full slice-and-send workflow, a transparent settings
cascade, and a Lua-based plugin system for G-code manipulation. It also
carries a profile for the full-size **Bambu Lab A1** — same Bambu MQTT
driver as the mini, but a full 4-spool AMS (`ams_type = "AMS"`, chainable
to 16) instead of the mini's AMS lite. The A1 mini + U1 are the
hardware-validated pair; the A1 profile is bundled and selectable
(`resources/profiles/bbl/printer/bambu-lab-a1/`).

The full workflow ships and runs **standalone at runtime** (no host
slicer/libslic3r needed): cascade resolver + adapter, the wgpu edit
viewport and G-code preview, end-to-end slice + G-code parser, settings
UI, multi-printer project model, both printer drivers + filament sync, the
Lua plugin system (platecycler hardware-validated), first-run onboarding,
OrcaSlicer `.3mf` project import, user docs, Linux CI, and distribution
across arch/flatpak/windows-cross/macos-cross with a self-hosted signed
repo. The native project format is `.n3o` (own zip: `project.json` +
per-mesh geometry blobs); `.3mf` is import-only. Not implemented: the
plugin compose hook (FR-PL-5), plugin hot reload, and an Orca preset
importer.

## Source-of-truth documents

Read these before reasoning about scope, design, or behavior:

- **`docs/dev/PRD.md`** — Product requirements. Goals, non-goals, success
  criteria, feature requirements (FR-CAS-*, FR-MP-*, FR-UI-*, …),
  printer capability matrix, architecture decisions (AD-1..AD-7),
  working practices (§11).
- **`docs/dev/profiles.md`** — Rule-cascade design of record. Two-phase
  resolution (authored cascade + `!important`-style override tiers),
  TOML schema, translation adapter to libslic3r's `DynamicPrintConfig`,
  option scope mechanism. The PRD's §6.1 codifies the requirements;
  this doc owns the design.
- **`docs/dev/design.md`** — Mockup review: what in `docs/dev/design/` is
  reusable as-is, what to port, what to replace.
- **`docs/dev/libslic3r-workarounds.md`** — The set of pre-`apply` and
  post-`apply` workarounds the shim applies to compensate for
  libslic3r's headless-mode quirks. Required reading before bumping
  the OrcaSlicer submodule.

If any of these contradicts something the conversation infers from
code or upstream, the document wins until corrected — then update the
document, don't just nod and move on (PRD §11.3).

## Architecture principles

These are load-bearing rules. Violating one usually means work later
costs more than it had to.

- **3D scene state lives in Rust, not in the renderer.** The
  authoritative scene model (objects, transforms, mesh data,
  selection) is a renderer-agnostic data structure in `core/scene/`.
  The renderer is a read-only consumer that reflects state events into
  pixels. All scene mutations go through Tauri commands; the renderer
  never owns state. Per AD-8, the **prepare-tab edit viewport is the Rust
  wgpu renderer** (`src-tauri/src/viewport_render.rs`
  + `src/viewport/WgpuViewport.tsx`) — Strategy A: wgpu renders offscreen
  in Rust and the frame is blitted into an opaque webview `<canvas>` (a
  transparent webview over GPU content smears on WebKitGTK; see
  `docs/dev/wgpu-renderer.md`). Mesh geometry never crosses the IPC bridge
  — it's uploaded straight to the GPU Rust-side. The **G-code preview**
  also renders with wgpu (`src-tauri/src/toolpath_render.rs` +
  `src/preview/GcodePreview.tsx`, instanced tubes, same Strategy-A blit).
  See PRD FR-3D-7 and AD-8. Two boundaries that follow:
  - *Transforms* — the renderer mutates object transforms only through
    commands, with `set_object_transform` (full-matrix) as the
    renderer-facing path. The scene layer itself has several transform
    ops (orient, align, lay-flat), some routed through the seated-bounds
    check. The active transform *mode* (translate/rotate/scale) is
    renderer-local UI state owned by `App`; `core/scene` holds no gizmo
    or pivot state.
  - *Camera* — the renderer owns its camera; the wgpu viewport holds
    `{az, el, dist, center}` frontend-side and frames the plate footprint
    from the bed (a view-aware corner fit). The scene model holds no
    camera or projection state.
  - *Rendering is on-demand.* The wgpu viewport does **not** run a
    continuous rAF loop — it would peg a CPU core + the GPU on a static
    scene. It renders only when something changes (`render()`, coalesced
    one-in-flight), driven by scene events, camera moves, drags, resize.
    Any code that changes the rendered picture must trigger a `render()`.

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

- **Comments earn their place.** A comment states *why* — a non-obvious
  constraint, rationale, or gotcha. Never restate what the next line
  plainly does (`// Nothing selected:` on an `else`, `// cursor's point
  on the plane` on the ray-plane formula), and never editorialize about
  a design's virtue ("renderer-agnostic X"). If it just narrates the
  code, delete it.

## Domain facts

Non-obvious facts about the hardware and architecture worth having up front.

- **Snapmaker U1 is a 4-toolhead CoreXY toolchanger** with magnetically
  docked toolheads on steel-ball kinematic couplings, eddy-current
  auto-alignment, ~5–10 s tool swaps. Klipper-based firmware. It is
  *not* IDEX. Each toolhead is independently replaceable, so each slot
  is a distinct cascade-layer entity with its own nozzle size, hotend
  type, and temperature range.
- **Bambu A1 mini uses AMS lite filament-swap** at a single hotend.
  Not a toolchanger. AMS lite reports per-slot filament identity
  (type, color, brand if reported) over MQTT.
- **Live camera + the Snapmaker control plane both work.**
  Per-instance camera streaming (`core/driver/camera.rs`,
  `camera_start`/`camera_stop`, `src/driver/useCameraStream.ts`) works for
  both vendors, and the U1 pair/mTLS/MQTT "monitor mode" control plane
  lives in `core/driver/snapmaker/{pairing,mtls,camera,snap_token}.rs`.
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
- **`orca-slicer-ffi` is owned in-house.** Authored by the project lead
  and vendored at `crates/slic3r-ffi/`. FFI extensions are first-party
  work, not external dependencies.
- **`platecycler` is owned in-house.** It ships as a Lua plugin using
  the **post-slice** hook — a macro append that auto-ejects the finished
  plate at print end. The compose hook (FR-PL-5) is not implemented.
- **Cascade resolution is two-phase** (design of record:
  `docs/dev/profiles.md`). The resolver lives in `src-tauri/src/core/cascade/`,
  the libslic3r translation in `core/cascade_adapter/` (`adapt` does the
  `bed_temp` → per-plate-type expansion + `curr_bed_type` set). The live
  *slice* path routes through resolver + adapter, **not** the input's embedded
  config (the `.3mf`/STL is loaded for geometry only):
  `core/slice/orchestrator.rs::resolve_cascade` composes a fresh authored
  cascade from the bound `PrinterInstance` (`profile_library::compose_cascade`
  — fragments + the instance's machine overrides baked as `!important`; the
  user/project/object tiers are **not** baked). Phase two is
  `cascade::resolve_with_overrides` (user = `Project.user_overrides`,
  project = `Plate.project_overrides`) → `to_resolved` →
  `cascade_adapter::adapt` → `DynamicPrintConfig`. The panel resolves the same
  way (`project::resolve::resolve_instance_cascade`), so what it shows matches
  what slices; `plate_cascade_trace(plate_id, key)` returns the per-key `Trace`
  (winning tier + fallback) for the "why is X=Y" UX. Tier-vs-baked parity and
  the end-to-end path are guarded by tests in `tests/slice_orchestrator.rs`
  and `profile_library::composer`.

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

Build flow (Linux + macOS native; Windows and macOS also cross-from-Linux):

```bash
git submodule update --init --recursive
./scripts/build.sh deps    # one-time — OrcaSlicer's deps tree; the only non-cargo step
cargo build                # 15 min cold, fast incremental
npm install && npm run tauri dev
```

The slic3r-ffi `build.rs` invokes cmake to build `libslic3r_ffi.{so,dylib}` —
the heaviest step in a `cargo build`. For a fast local engine set
`N3O_SLIC3R_FFI_CMAKE_CONFIG=Release` (the default RelWithDebInfo builds `-O0`).

**Packaging** lives under `packaging/<target>/` (arch, flatpak,
windows-cross, macos-cross), each exposing the same trio: `build.sh`
(unsigned artifact, ensures its dep tree on demand), `publish.sh` (build +
GPG-sign + upload), `clean.sh`. npm mirrors them (`build:<t>` / `publish:<t>` /
`clean:<t>`, plus `build:all` / `publish:all`; top-level `npm run clean`
also sweeps the shared cargo/FFI/deps/dist remainder). Publish env:
`N3O_BASE_URL`, `N3O_PUBLISH_DEST`, `N3O_GPG_KEY`. The cross toolchains build
in-tree (osxcross via `ensure-osxcross.sh`, `cargo xwin` for Windows) and need
no Mac / nothing in `$HOME`. Deep per-target specifics — arch-namespaced macOS
deps prefixes, dylib bundling + ad-hoc signing + entitlements, DMG assembly —
live in each target's scripts and `README.md`; read those before touching a
packaging path. Open items: x86_64/universal macOS, styled DMG, and
Developer-ID notarization (all need a paid Apple account / a Mac).

## Running the app

n3o-slic3r is a **Tauri 2** app: a Rust backend (`src-tauri/`) driving a
**WebKitGTK** webview that loads the React frontend from a **Vite** dev
server on **port 1420**.

```bash
npm run tauri dev
```

This is the blessed entrypoint. It:
- runs `beforeDevCommand` (`npm run dev` → Vite on 1420),
- builds + launches the Rust backend (incremental; fast once warm),
- loads **`.env`** via `dotenv` (the `npm run tauri` script does this) —
  critically setting `WEBKIT_DISABLE_DMABUF_RENDERER=1` and
  `N3O_SLIC3R_RESOURCES_ROOT=./resources`.

### The Wayland gotcha (don't skip the env)

Launching the **bare debug binary** (`./target/debug/n3o-slic3r`) directly
on a Wayland session **crashes** with:

```
Gdk-Message: Error 71 (Protocol error) dispatching to Wayland display.
```

The fix is `WEBKIT_DISABLE_DMABUF_RENDERER=1` (the dmabuf renderer doesn't
work on every compositor, e.g. Hyprland). `npm run tauri dev` sets it from
`.env`; if you run the binary yourself, set it yourself. The debug binary
is a *dev* build — it loads the frontend from the Vite dev URL (1420), so
**Vite must be running** (`npm run dev`) when you launch it standalone.

### Process-kill gotcha

`pkill -f "target/debug/n3o-slic3r"` matches **its own command line** (which
contains that string) and kills the invoking shell — you'll see a spurious
exit code **144**. Use exact-name matching instead:

```bash
pkill -x n3o-slic3r
```

## Screenshotting headlessly

Drive the app in a **nested X server (Xvfb) + xdotool** — display-scoped, so
input can't leak into a locked host session, and WebKitGTK runs fine on X11
via `GDK_BACKEND=x11`. (The Wayland alternatives are all worse on a locked
host: ydotool is global/kernel-level, sway-headless advertises no input
device, Xephyr needs a visible window.) One-time on Arch:
`sudo pacman -S --needed xorg-server-xvfb xdotool` (`import`/`scrot` capture).

```bash
# Vite must serve the frontend, and a debug binary must exist:
npm run dev &                        # → :1420   (build once: cargo build -p n3o-slic3r)
Xvfb :99 -screen 0 1600x1000x24 &
DISPLAY=:99 GDK_BACKEND=x11 WEBKIT_DISABLE_DMABUF_RENDERER=1 \
  N3O_SLIC3R_RESOURCES_ROOT=./resources ./target/debug/n3o-slic3r &
sleep 8                              # libslic3r init + first render
DISPLAY=:99 xdotool search --name n3o-slic3r windowactivate
DISPLAY=:99 xdotool mousemove 1007 530 click 1    # coords = screenshot pixels
DISPLAY=:99 import -window root /tmp/shot.png      # or: scrot
# cleanup: pkill -x n3o-slic3r; pkill -x Xvfb; fuser -k 1420/tcp
```

Read each PNG back and confirm before the next click — a blank/solid frame
means the app didn't render (Vite down or the dmabuf var unset). The app uses
the real user library at `~/.config/n3o-slic3r/`; with printers configured it
opens into the workspace (else onboarding), and prior autosaves raise a
"Recover unsaved projects" dialog to dismiss first.

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
- **Spike before committing.** One day of focused experiment routinely
  saves a week of debugging (PRD §11.2).
- **Living documents.** PRD, profiles.md, design.md, AGENTS.md are not
  commitments frozen at kickoff. Update them when reality diverges,
  commit the doc change alongside the code change (PRD §11.3).
- **Correction posture.** The project lead corrects detail-level
  reasoning without expecting pushback. Incorporate corrections, then
  propagate them to persistent context — don't just nod (PRD §11.4).
