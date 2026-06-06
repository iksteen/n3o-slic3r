# Phase 6 — tickets

Phase 6 (G-code preview, ~3 person-weeks + ~3 days of carryover
wiring) ships the **in-app G-code visualizer** that closes the
PRD's "fully usable for a complete print workflow with no
external G-code viewer" claim (§6.6). Source:
`docs/dev/Execution_Plan.md` §8. Stated goal:

> Production-quality in-app G-code visualization. Hard
> requirement, not a polish item. Builds directly on the typed
> G-code parser from Phase 3.

Phase 6 is **a self-contained vertical slice**: a new panel /
mode that consumes G-code (either freshly sliced by Phase 3 or
dropped from disk) and renders it interactively. It does not
extend, modify, or depend on Phase 4/5 surfaces at runtime — it
sits next to the 3D viewport, sharing the topbar and bed-grid
chrome only.

**Carryover from earlier phases:** the Slice button today
re-loads a user-picked mesh file through libslic3r and ignores
the scene's transforms, per-plate printer/material bindings,
project/object overrides, and multi-plate composition. The
orchestrator's source acknowledges this deferral
(`orchestrator.rs:265-267`: "PR-3-9's project writer is the
future path … for MVP we slice whatever file the caller pointed
us at"). Phase 6 can't ship a meaningful preview without the
slice reflecting what the user actually composed; PR-6-1
through PR-6-3 close that gap as a prerequisite block.

Individual tickets live one-per-file in `phase-6/`. This file is
the index plus phase-level status and notes.

## Status by deliverable

### Prerequisite block — wire Slice to the live scene

| Deliverable | Status | Ticket |
|-------------|--------|--------|
| Scene-to-slice input builder: `(Project, PlateId) → SliceJobInput` | ✅ shipped | [PR-6-1](phase-6/PR-6-1%20Scene-to-slice%20input%20builder.md) |
| `slice_active_plate` Tauri command (replaces `slice_start_default_a1mini`) | ✅ shipped | [PR-6-2](phase-6/PR-6-2%20Slice%20active%20plate%20command.md) |
| Frontend Slice button rewire: drop file picker, drive from scene | ✅ shipped | [PR-6-3](phase-6/PR-6-3%20Slice%20button%20rewire.md) |

### Preview proper

| Deliverable | Status | Ticket |
|-------------|--------|--------|
| Preview IR: extrusion + travel segment build | ✅ shipped | [PR-6-4](phase-6/PR-6-4%20Preview%20IR.md) |
| Color-mode encoders (feature / speed / flow / layer-time / tool) | ✅ shipped | [PR-6-5](phase-6/PR-6-5%20Color%20encoders.md) |
| Per-layer + full-job stats computation | ✅ shipped | [PR-6-6](phase-6/PR-6-6%20Stats%20computation.md) |
| Preview Tauri commands (load, buffers, stats) | ✅ shipped | [PR-6-7](phase-6/PR-6-7%20Tauri%20commands.md) |
| `GcodePreview` Three.js renderer + camera + bed | ✅ shipped | [PR-6-8](phase-6/PR-6-8%20Renderer.md) |
| Layer slider (single / up-to-N / range) + keyboard nav | ✅ shipped | [PR-6-9](phase-6/PR-6-9%20Layer%20slider.md) |
| Travel + retraction visibility toggles | ✅ shipped | [PR-6-10](phase-6/PR-6-10%20Travel%20toggles.md) |
| Hover inspection: raycast → segment tooltip | ✅ shipped | [PR-6-11](phase-6/PR-6-11%20Hover%20inspection.md) |
| Per-layer + full-job stats panels | ✅ shipped | [PR-6-12](phase-6/PR-6-12%20Stats%20panels.md) |
| Color-mode picker + color-blind-safe palette | ✅ shipped | [PR-6-13](phase-6/PR-6-13%20Color%20picker.md) |
| Drag-drop external `.gcode` + `.gcode.3mf` loader | ✅ shipped | [PR-6-14](phase-6/PR-6-14%20Drag-drop%20loader.md) |
| App preview/3D mode toggle + auto-switch after slice | ✅ shipped | [PR-6-15](phase-6/PR-6-15%20App%20mode%20toggle.md) |
| Perf gates (50MB <5s; 60fps slider; <1.5GB) | ✅ shipped | [PR-6-16](phase-6/PR-6-16%20Perf%20gates.md) |
| Phase 6 exit-criteria smoke + docs | ✅ shipped | [PR-6-17](phase-6/PR-6-17%20Exit-criteria%20smoke.md) |

## Architecture invariant — the preview owns its own Three.js scene

Phase 2 shipped the 3D viewport with its own `sceneMirror`,
event-bridge, and `ViewportCanvas` Three.js setup. **The preview
does not extend that scene** — it builds a parallel `PreviewScene`
with its own renderer, camera, and meshes, mounted in the same
viewport DOM region (managed by a top-level mode toggle).

Reasons:

- The viewport scene carries gizmos, selection state, raycasting
  against object meshes. None of that is meaningful for G-code
  paths.
- The preview's vertex count is ~10-50× the viewport's (50MB
  G-code → ~1-5M extrusion segments). Re-using the viewport's
  scene graph would force every viewport interaction to walk
  millions of irrelevant children.
- Mode toggle vs. layered overlay simplifies hit-testing: a
  preview-mode hover is unambiguously a preview hover.

Shared chrome only: the topbar (project name, slice button,
preview/3D toggle), the bed grid (read from active plate's
printer profile), and the bed dimensions overlay. Camera state
is **not** shared — the preview lands at a sensible default
("look down the +Z axis from above the bed").

## Architecture invariant — Slice is driven by Project state, not by file paths

Post-PR-6-2, the slice pipeline reads the live `Project`. The
`slice_active_plate` command takes a `plate_id` (defaults to the
active one), pulls the plate's scene + printer binding + material
bindings + overrides, and produces a `SliceJobInput` via the
PR-6-1 builder. Temp `.3mf` files for libslic3r consumption are
internal — never user-visible, written under
`std::env::temp_dir()`, cleaned up after the worker exits.

**No path-based `slice_*` command survives.** The old
`slice_start_default_a1mini` is removed; the orchestrator
worker's `model.load(&job.model_path)` keeps pointing at a real
file, but that file is the temp `.3mf` the input builder wrote,
not anything the user picked.

This finalizes the Phase 5 promise that "per-plate is the
contract" — the slice path is the last surface that still
treated the project as single-plate-implicit.

## What's *not* in Phase 6

- **Multi-plate "Slice all" button** — defer to Phase 7 polish
  (or earlier as a quick win). PR-6-3 ships per-plate slicing;
  looping is a UI affordance.
- **Variable-layer-height authoring** — Phase 9 polish.
  PR-6-12's per-layer stats surface `layer_height` so the user
  can spot variable-layer surprises in third-party G-code, but
  the authoring UX is later.
- **Settings panel integration** — the preview is read-only
  over a sliced G-code file; tweaking a setting and re-slicing
  is a manual user flow (close preview, edit, slice again).
  Auto-re-slice on settings change is Phase 9.
- **Re-slicing from the preview** — no "Slice again" button in
  preview mode; the user toggles back to 3D and re-clicks the
  Slice action.
- **3D paint-on overlays** (paint-on supports, paint-on seam)
  — post-MVP per PRD §10.
- **Tool path simulation / time playback** — Phase 9 / post-MVP.
  Preview is a static visualizer; the layer slider is the only
  temporal control.
- **Driver UX** (send-to-printer affordances per driver) —
  Phase 7. Phase 6's preview reads what the slice pipeline
  produced; sending it to a real printer is later.

## Dependency graph

```
PR-6-1 (scene-to-slice input builder)
  └── PR-6-2 (slice_active_plate Tauri command)
       └── PR-6-3 (frontend slice button rewire)  ← unblocks meaningful preview content

PR-6-4 (preview IR — segment build from gcode::Line)
  ├── PR-6-5 (color encoders — depend on IR shape)
  ├── PR-6-6 (stats computation — depend on IR shape)
  └── PR-6-7 (preview Tauri commands — depend on IR + color + stats)
       ├── PR-6-8 (GcodePreview renderer)  ← critical path
       │    ├── PR-6-9 (layer slider)
       │    ├── PR-6-10 (travel + retraction toggles)
       │    ├── PR-6-11 (hover inspection)
       │    ├── PR-6-12 (stats panels)
       │    └── PR-6-13 (color-mode picker)
       └── PR-6-14 (drag-drop external file loader)

PR-6-3 + PR-6-8 + slice-job-done event
  └── PR-6-15 (App mode toggle + auto-switch after slice)

All of PR-6-1..-15 ─► PR-6-16 (perf gates)
                  ─► PR-6-17 (exit smoke)
```

Two critical paths:
1. **PR-6-1 → PR-6-2 → PR-6-3** — the prerequisite block. Until
   this lands, every preview test has to feed a hand-picked
   `.gcode` file; the smoke test (PR-6-17) can't validate the
   intended "slice the scene → preview the result" loop.
2. **PR-6-7 → PR-6-8** — the rendering critical path. PR-6-4,
   -5, -6 can land in parallel; once PR-6-7 wires them into
   Tauri commands the renderer can mount and the rest fans out.

The two paths are independent; recommend running them in
parallel from day one.

## Exit criteria for the phase (from Execution Plan §8)

- Load a 50MB production G-code (e.g. a multi-hour multi-material
  print), step through layers, switch color modes, and inspect
  segments — all without external tools.
- Performance targets met on the project lead's reference hardware
  (integrated GPU laptop): 50MB end-to-end in <5s; 60fps layer
  slider; <1.5GB memory.
- Preview correctly visualizes G-code from this app's slicer, and
  also G-code from foreign sources (Orca, Cura, Prusa) for
  compatibility.
- **End-to-end slice→preview loop works without external files:**
  set up a multi-plate scene, slice the active plate from the
  in-app button, watch the preview mount automatically with the
  fresh G-code. (Adds the PR-6-1..-3 wiring as a checkable smoke
  step.)

PR-6-17 mechanizes this on a real 50MB fixture (sliced once and
checked into the repo, or generated in-test by slicing a known
model). PR-6-16 mechanizes the perf gate with criterion
benchmarks + a frame-time-during-scrub assertion.

## Cut candidates (from Execution Plan §8)

If pressed for time:

- **Layer time + flow color modes** (sub-deliverable of PR-6-5)
  → saves ~2 days. Keep feature + speed + tool — those cover the
  common debug cases.
- **Per-layer stats panel** (half of PR-6-12) → saves ~2 days.
  Keep full-job stats. Hurts the "spot a variable-layer-height
  surprise" use case but the layer slider's layer number is still
  visible.
- **Drag-drop external `.gcode`** (PR-6-14's standalone-loader
  half) → saves ~1 day. Cut LAST per Exec Plan: hurts the
  standalone-preview-as-Slicer story significantly.
- **Layer-range view** (sub-deliverable of PR-6-9) → saves ~1
  day. Keep single + up-to-N. Users debugging a layer band would
  step single-layer instead.

The cut candidates the user signed off on for MVP scope: **none**
— all 5 color modes, all 3 slider modes, both drag-drop formats,
and GPU layer culling all included. Trim the list above only if
the phase runs hot.

**Not cuttable:** the PR-6-1..-3 prerequisite block. The current
slice flow's scene-blindness is a blocker for the rest of the
phase, not an optional polish item.

## Implementation notes

### Layer culling strategy (GPU)

PR-6-9's slider hits a 60fps gate against a 50MB gcode (~1-5M
extrusion segments). The chosen path:

- One `BufferGeometry` for extrusions, one for travels. Built
  once from the preview IR.
- Per-vertex `aLayer: float` attribute carrying the layer index
  the segment belongs to.
- `ShaderMaterial` with two uniforms: `uLayerMin: float`,
  `uLayerMax: float`. Vertex shader passes through; fragment
  shader `discard`s when `aLayer < uLayerMin || aLayer > uLayerMax`.
- Slider scrub updates uniforms only — no buffer rebuild, no
  upload. Hits 60fps trivially.

Range mode: `uLayerMin = A, uLayerMax = B`.
Up-to-N mode: `uLayerMin = 0, uLayerMax = N`.
Single mode: `uLayerMin = N, uLayerMax = N`.

Travel + retraction toggles map to `material.visible` on the
travel `LineSegments` object, not to shader discards — no
per-vertex bookkeeping needed.

Color mode swap: also a uniform update (palette LUT) +
re-bind of the active color attribute. Buffer geometry is
preserved across mode changes.

### Where the preview mounts

The topbar gets a `Preview [P]` button that toggles a top-level
`mode: "scene" | "preview"` state. The viewport DOM region
swaps between `<ViewportCanvas/>` and `<GcodePreview/>`. The
plate tabs strip stays visible (the active plate's last-sliced
G-code is what loads); the settings panel hides in preview
mode (no settings to edit; reclaim the screen real estate for
stats panels).

### Coordinate convention

G-code coordinates are in printer space: X/Y in mm relative to
the bed origin (front-left corner), Z in mm above the bed.
This matches the viewport's bed-mesh space — same camera
orientation works without remapping. The preview's "look down
+Z" default starts at `(bed_center_x, bed_center_y - 0.5 *
bed_depth, max_z * 1.5)`.

### Scene-to-slice temp file lifecycle

PR-6-1's input builder writes a temp `.3mf` containing the
plate's meshes + transforms + per-volume extruder assignments.
Location: `std::env::temp_dir().join(format!("n3o-slice-{plate_id}-{pid}-{nanos}.3mf"))`.
Lifecycle: written before `start_slice_job`, deleted in the
job's terminal handler (`Finished` / `Failed` / `Cancelled`).
Failure to delete is a tracing warning, not a hard error —
temp-dir cleanup is the OS's job long-term.

The `.3mf` is the same format PR-5-8's project save writes, but
with the n3o-slic3r extension namespace omitted (libslic3r
ignores unknown namespaces anyway, but skipping them keeps the
temp file lean). Geometry-only, basically.

### What slice-from-scene buys at preview time

The preview lands on real per-plate G-code that respects the
project state. Knock-on benefits:

- **Per-plate previewing** works: switch the plate tab, the
  preview re-mounts with that plate's last-sliced G-code.
- **Material bindings show in the gcode** (PR-3-5's `ToolChange`
  + tool-mode color in PR-6-5 reflect the user's slot
  assignments).
- **Object-tier overrides take effect** (per-object enable_support
  toggles, per-object layer_height, etc.) — the user can spot
  the override's effect in the preview.

## Open questions seeded for the implementer

- **Worker-thread parsing (PR-6-4).** Parsing 50MB of G-code
  on the main thread will jank UI for 1-3 seconds. PR-6-7's
  `preview_load` Tauri command already runs off-main on the
  Rust side; verify the IPC response time (header + layer
  count comes back quickly, geometry buffers are pulled
  lazily on `preview_buffers`). Frontend may want a
  `<LoadingSpinner/>` during the first `preview_buffers` call.
- **Foreign-slicer compat coverage (PR-6-17).** "Render foreign
  G-code correctly" needs concrete fixtures. Pull one short
  print from Orca, Cura, and PrusaSlicer (the spike-3 fixture
  already covers Bambu Studio); render each and spot-check the
  feature classification. Header-metadata extraction is
  separately validated by `header.rs` tests from PR-3-8.
- **Bed grid sharing (PR-6-8).** The Phase 2 bed-grid rendering
  lives in `ViewportCanvas` not in a sharable component. Either
  extract to a `<BedGrid/>` that both surfaces mount, or
  reimplement (it's ~50 lines of Three.js). Decide during
  PR-6-8 based on the diff that extraction would create.
- **`gcode.3mf` plate selection (PR-6-14).** A `.gcode.3mf` may
  contain multiple plates (Bambu Studio's project export
  format). Open question: when the user drag-drops one, which
  plate's G-code do we preview? Options: (a) the first plate;
  (b) a plate picker modal; (c) load all plates as separate
  preview entries with a strip selector. Defer to PR-6-14 with
  (a) as the default; promote to (b) or (c) if real usage
  surfaces a need.
- **Fixture for the 50MB perf gate (PR-6-16).** Production
  G-codes that large aren't in the repo and adding one inflates
  the clone significantly. Options: (i) check in a smaller real
  fixture (~5-10MB) and scale perf assertions proportionally;
  (ii) generate in-test by slicing a known model with extreme
  layer height + perimeter count; (iii) run perf locally /
  nightly with an external fixture path. Pick during PR-6-16.
- **Per-object extruder export in PR-6-1's temp 3MF.** Each
  scene object carries `extruder_id`; libslic3r reads it as
  `extruder` metadata on the volume. PR-5-8's project writer
  already serializes this for save/load but the geometry-only
  temp format may need its own walk. Verify the 3MF writer
  drops `extruder` into the `metadata` block per volume.
- **Cascade context binding in PR-6-1.** PR-1-7 defined
  `SlicingContext`; PR-3-2's `ContextJson` already has the
  shape. Verify the per-plate `Plate.printer` + filament
  bindings translate into `ContextJson.printer` +
  `ContextJson.filaments` cleanly. The active filament index
  comes from the plate's material bindings (model material →
  slot → filament identity).
