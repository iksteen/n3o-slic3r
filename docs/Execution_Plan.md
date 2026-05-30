# MVP Execution Plan

Multi-Printer Slicer

*Companion document to the PRD*

| **Field** | **Value** |
| --- | --- |
| Plan version | 0.1 (draft) |
| Total MVP effort | ~37.5 person-weeks of focused work, distributed across 10 phases |
| Team size assumed | 1 (project lead) |
| Tooling assumption | Heavy Claude Code use |
| Buffer included | 20% absorbed into phase estimates |
| MVP freeze checkpoint | End of Phase 9 |
| Note on time | Phase durations below are effort estimates in work-weeks, not calendar weeks. Real-world calendar will stretch based on context-switching, hardware testing windows, life. Phase ordering and dependencies are real; the wall-clock dates are not. |

# 1. Planning approach

This plan is structured in 10 phases (a foundation, a validation spike phase, and eight build phases). It is expressed in effort, not calendar time: each phase has a person-week estimate that describes how much focused work it requires, not how many weeks of wall-clock time it consumes. Phase ordering and dependencies are real; the dates are not.

This framing exists because the actual calendar will stretch unpredictably: hardware-testing days interleave with development days, some weeks will be lost to other obligations, some phases will run faster than estimated and others slower. What matters for this plan is that the effort estimates are honest and the dependencies are correct.

A hard constraint shapes the plan: the MVP must be fully independent of OrcaSlicer or any other slicer at runtime. This affects three things specifically: (a) G-code preview is a first-class feature with its own phase, not a polish item; (b) the G-code parser is built early so it can serve both preview and plugins; (c) milestone demos cannot use OrcaSlicer's viewer as a verification oracle — the app verifies itself, or an alternative independent G-code analyzer is used during development only.

A second hard constraint: the MVP must produce output that the targeted printers actually accept. This is a printer-compatibility constraint, not a format constraint. For the A1 mini, that happens to mean wrapping G-code as .gcode.3mf with Bambu metadata extensions (the printer rejects raw G-code). For the U1, it means sending plain .gcode (the printer is Klipper-based and expects raw G-code over HTTP). 3MF is therefore a tool used by some printer drivers, not a universal output format — adding a future printer means writing its driver, not extending shared format code. Separately, .3mf is also the project file format the app uses (and reads from other slicers as a migration path); this is unrelated to the send-format question.

The configuration model is documented separately in the profiles strategy document (docs/profiles.md). It defines a rule cascade with selector-based overrides and CSS-like specificity, plus a translation adapter that emits libslic3r's flat DynamicPrintConfig. This plan's Phase 1 implements that design; the PRD §6.1 captures the requirements that follow from it.

Four rules govern the plan:

- **Vertical slices early.** By end of Phase 3, the app can take an STL, load it, render it, slice it, and produce G-code — even if the UI is ugly and the cascade is stubbed. Subsequent phases deepen each layer rather than building horizontal completeness.

- **Spike before committing.** When a phase depends on an unverified assumption about libslic3r, a printer protocol, or a third-party library, a small focused experiment validates the assumption before the phase's main work begins. Phase 0.5 is dedicated to this pattern; later phases continue it informally. A spike is one day of work that routinely saves a week.

- **Hardware testing every phase from Phase 5.** Once printer comms exist, every phase ends with a real print. No exceptions. Issues caught at the printer are 10x cheaper than issues caught at release.

- **Cut list maintained continuously.** Each phase has explicit cut candidates that can be moved to v1.1 without invalidating the MVP. If a phase runs hot, cuts are made from that phase's cut list, not by extending the timeline.

# 2. Phase 0 — Foundation (effort: ~2 person-weeks)

Goal: a Tauri app that launches, links the FFI, and round-trips a hello-world FFI call to the UI.

### Deliverables

- Tauri 2.x project scaffolded, React + TS + Tailwind frontend wired.

- orca-slicer-ffi Rust crate linked into the Tauri core.

- First Tauri command exposes slic3r_version() to the frontend; UI displays it.

- CI building on Linux. Windows/macOS CI is post-MVP.

- Logging infrastructure (tracing crate) wired into the Rust core.

- Repo structure matches the module boundaries in the PRD.

### Exit criteria

- App launches on the project lead's primary dev machine. Frontend shows libslic3r version. CI green on Linux.

### Cut candidates

- None. This phase is the foundation; cuts here cause cascading failures.

# 2.5. Phase 0.5 — Engine validation spikes (effort: ~1 person-week)

Goal: validate architectural assumptions about libslic3r and printer protocols with small focused experiments before Phase 1 commits to them. One week, five spikes, each producing a written finding.

These spikes exist because the planning phase identified several assumptions worth testing cheaply. A failed spike triggers a planning revision; a passed spike unblocks downstream work with confidence.

### Spikes

- **Spike 1 — Cascade adapter end-to-end.** Write a minimal TOML rule cascade (a default rule + a filament rule + a plate rule), implement a stub resolver and adapter that emits a DynamicPrintConfig, hand it to libslic3r via the FFI, slice a Benchy. Confirm the adapter pattern works end-to-end before Phase 1 commits to the full design. Validates per-extruder vector reading, scope dispatch, and dimensional expansion in one experiment. Document any FFI surface gaps. **Constraint (added after P0-5):** the seed config must be a converted real OrcaSlicer device profile (Bambu A1 mini or Snapmaker U1 from `external/OrcaSlicer/resources/profiles/`), not a hand-rolled minimum config. P0-5 confirmed that libslic3r's `Print::validate()` rejects FullPrintConfig defaults before any slicing happens, so the spike has to exercise the real round-trip — converting an Orca JSON device profile into our cascade, resolving it, dispatching to libslic3r, producing valid gcode.

- **Spike 2 — Mixed-nozzle-size slice.** Slice a small test model with a Prusa XL profile configured for 0.4mm on tool 0 and 0.6mm on tool 1. Verify libslic3r emits sensible per-tool extrusion widths and tool-change G-code. This validates the per-toolhead independence assumption for U1.

- **Spike 3 — Bambu A1 mini AMS slice.** Slice a 4-color test model with a Bambu A1 mini profile + AMS multi-color. Compare the G-code and the wrapping 3MF to what Bambu Studio produces for the same input. Document the metadata-format gaps and the purge-volume-driven structure that libslic3r generates.

- **Spike 4 — coEnums known limitation impact.** Identify the 9 options affected by the known FFI limitation. Determine whether any are on the critical path for A1 mini or U1 operation. If yes, schedule the FFI fix into Phase 1; if no, defer.

- **Spike 5 — platecycler portability.** Run the existing platecycler Python tool against G-code produced by Spike 3. Confirm the transform pipeline still works with our libslic3r output (vs Bambu Studio output). Document any divergence — this informs the Phase 8 compose-hook implementation.

### Exit criteria

- Five findings documents committed to the repo (one per spike), each with: assumption tested, method, result, implications for downstream phases.

- Any failed spike has a corresponding plan-revision PR open, not deferred.

### Cut candidates

- Spike 4 (coEnums) — saves 1 day, but you may discover the limitation mid-Phase 4 instead of mid-Phase 0.5. Cheap to do now.

- Spike 5 (platecycler) — saves 1 day, but Phase 8 will be slower if the surprise lands late.

# 3. Phase 1 — Rule cascade resolver + translation adapter (effort: ~4 person-weeks)

Goal: working rule cascade resolver and translation adapter, fully tested, with no UI yet. Implements the design captured in the profiles strategy document. The resolver handles rule loading, predicate evaluation, specificity ranking, source-order tie-breaking, and trace generation. The adapter translates resolved logical settings into libslic3r's DynamicPrintConfig, handling dimensional expansion and dispatch quirks.

### Deliverables

- Schema generator: introspect libslic3r options → typed Rust schema (option name, type, scope bitmask, per-extruder vs scalar, dimensional metadata). Drives validation, UI rendering, and the translation manifest.

- Rule cascade resolver (core/cascade): loads TOML rule files, validates predicates and set keys against the schema, parses [[rule]] full form and section shorthand. Accepts a context object, returns resolved settings with trace metadata (winning rule's file:line, specificity, and the list of also-matching rules that lost).

- Two-phase resolution: (1) authored cascade — load and resolve default → printer → build_plate → filament[slot] using specificity-wins with source-order tiebreak; (2) absolute overrides — apply user profile then project file as CSS-!important-style override tiers that win regardless of the cascade's specificity. User is one !important tier; project is a higher !important tier; project loaded after user wins ties between them. Document the boundary clearly in code; the trace tool reports both phases.

- User-profile and project-file shape: flat unconditional set.* entries with no [[rule]] blocks. Generated by the UI when a user saves preferences or edits a project. The resolver applies them in phase 2 as absolute overrides.

- Within-cascade tie-break warnings: when two same-specificity rules from different authored files both apply to the same setting, log a warning at resolution time. (User/project overrides don't trigger warnings — they're expected to win.)

- Trace tooling: 'why is X = 55?' returns a structured trace. When an absolute override (user or project) is active, the trace reports the override source and also reports what the authored cascade would have resolved to (the 'cascade fallback'). When no override is active, the trace reports the winning authored rule's file:line and specificity, plus the list of matching-but-losing rules. Forms the data API behind FR-CAS-7.

- Load-time validation: predicate dimensions and setting names checked against the schema; typos rejected with file:line errors. Scope compatibility checked (object-scoped settings only allowed in object-applicable contexts; print-scoped only in print contexts; etc.) per FR-CAS-12 / FR-CAS-13.

- Translation adapter (core/cascade-adapter): converts resolved logical settings → libslic3r DynamicPrintConfig. Handles identity mappings (most settings) and dimensional expansion (bed temperature across the 6 plate types, etc.) via the translation manifest.

- Translation manifest: initial Rust data structure with ~50 dimensional entries plus identity-map fallback for the rest. Seeded from the libslic3r-source-of-truth via FFI introspection. Author and maintain as a code-review-ed file, since libslic3r upgrades may require manifest updates.

- Adapter dispatch quirks handled: curr_bed_type set, wipe_tower normalized for multi-material toolchange, filament_map / nozzle_volume_type / wall_filament normalized (the shim already does some of this — extend as needed).

- Unscoped options (~71 keys) handled as opaque project metadata: round-trippable from 3MF imports, never set by our rules.

- Context-state structures (not rules): printer profile (slot_count, supported build plates, per-slot toolhead config including nozzle diameter / hotend type / max temp, exclusion zones, build volume); build_plate file (identity, declared surface properties); filament profile (identity, declared base type).

- Two reference printer profiles authored: A1 mini (slot_count=4, AMS lite filament-swap mechanism, single hotend, supports Cool/Textured PEI/Smooth PEI/Engineering/SuperTack plates) and U1 (slot_count=4, toolchange mechanism, per-slot toolhead config, supports U1's ship-standard plate set).

- Reference build plate files for both printers' plates with bed-temperature rules covering common filament types (PLA, PETG, ABS).

- Two reference filament profiles (e.g. generic PLA and generic PETG) with per-plate bed-temperature rules.

- Comprehensive unit tests: golden-file resolution tests covering single-predicate, multi-predicate, specificity ties, source-order tiebreakers, scope violations, unscoped key handling. Property tests for resolver invariants.

- Tauri command surface: load rule files into the cascade, resolve cascade for a given context, fetch trace for a single setting, fetch resolved DynamicPrintConfig (via adapter), list available context dimensions and their valid values.

### Exit criteria

- Given the A1 mini and U1 reference profiles, plate files, and filament profiles, the resolver returns correct effective values with full trace metadata for the canonical contexts (e.g. A1 mini + PEI + PLA in slot 0, U1 + textured PEI + PLA in slot 0 + PETG in slot 1).

- The adapter produces a DynamicPrintConfig that libslic3r accepts via the FFI's existing config-load path, and slicing produces G-code (validation of correctness defers to Phase 3's end-to-end test).

- Trace tool returns correct rule attribution: for a setting with three matching rules at specificities 0, 1, 2, the trace reports the specificity-2 rule as winner and both losers with their file:line.

- Absolute override behavior: a project setting set.bed_temp = 50 wins over a filament rule when.filament.type = 'PLA' when.plate.type = 'PEI' → set.bed_temp = 55 even though the filament rule has higher specificity. The trace reports the project override as winner and the specificity-2 rule as the cascade fallback the user would revert to.

- Load-time validation catches: a misspelled predicate dimension, a set key not in the schema, a scope violation (object-scoped key in a print-scope rule).

- Resolver benchmarks under 10ms for full 4-slot resolution; under 100ms includes adapter expansion to DynamicPrintConfig (FR-CAS-11).

- Comprehensive test coverage on resolver and adapter modules.

### Cut candidates

- Trace tool's 'matching-but-losing rules' list (winner only) — saves 1 day. Hurts FR-CAS-7 UX but the source badge still works.

- Property tests (golden tests only) — saves 2 days. Reduces confidence in cascade-edge-case correctness.

- Reference build plate files / reference filament profiles authored beyond a minimum set (single PLA, single PEI) — saves 2 days. Pushes profile authoring into Phase 7 hardware testing.

# 4. Phase 2 — 3D viewport and model loading (effort: ~4 person-weeks)

Goal: functional 3D scene with model load, transform operations, and bed visualization. Start early because perf risk lives here.

### Deliverables

- **Renderer-agnostic scene state in Rust (FR-3D-7 / AD-8).** Build this before the Three.js layer — it's the foundation the renderer sits on. Scope: typed scene model (mesh registry, per-object transforms and metadata, hierarchy, selection, exclusion-zone data); Tauri command surface for mutations (`scene_select`, `object_translate`, `object_set_transform`, etc.); Tauri event surface for state diffs the renderer applies. Lives in `core/scene/` per PRD §8.2. Unit tests cover the command/event contract without any renderer present. (Gizmo-pivot and camera scene state + their `gizmo_set`/`camera_*` commands were built then removed as dormant view-state — see PRD §9.2; transform *mode* is renderer-local and the renderer owns its own camera. Re-add to the scene model when a pivot-setting UI or persisted-view feature lands.)

- Three.js scene with orbit controls, perspective + ortho toggle, gizmo for move/rotate/scale. The renderer is a *view*: it subscribes to scene events, applies them to its local mirror, and emits user-intent through the command surface. It does not hold authoritative state.

- Load STL, OBJ, and .3mf (project format): geometry, object positions, and as much project metadata as the file carries. Loader runs in Rust, populates the scene state directly; the renderer learns of new meshes via the standard event flow. Bambu Studio, OrcaSlicer, and Snapmaker Orca all save projects as .3mf — this is the migration path for users.

- Object operations: move, rotate, scale, mirror, lay flat, duplicate, delete. Each is a Tauri command operating on the Rust scene state. The renderer reflects the resulting state diff; it does not compute transforms itself.

- Object library / scaffolding panel (FR-UI-10): left side of viewport, sections for Primitives (cube/cylinder/sphere/cone/torus), Calibration (calibration cube, temperature tower, generic flow test), and Imported (user-loaded files this session). Clicking an item adds it to the active plate.

- Auto-arrange (single plate, no rotation for MVP).

- Bed mesh with grid, origin marker, A1 mini exclusion zone, U1 toolhead parking bay visualization.

- Performance stress test: 20M-triangle scene runs at >=30fps on integrated GPU laptop. Decision point: continue with Three.js or pivot to wgpu native window. The state-vs-renderer separation (AD-8) keeps the pivot cost bounded to the renderer layer — see PRD §10 risk row.

- Renderer-side performance: applying scene diffs at 60 Hz worst-case interaction rates (orbit, drag) without dropping below the 30fps render target. Tested with a 1000-object scene to validate the state-side budget (≤5ms p99 for selection / transform / diff computation, per AD-8).

### Exit criteria

- Load a 50MB STL and a Bambu-Studio-authored .3mf, manipulate them, save and reload position.

- Performance target met or pivot decision made and scheduled.

- Scene-state Rust module's command/event surface is fully covered by tests that run without any renderer attached. Swapping in a stub viewer that just logs events produces sensible output for typical interaction sequences (load → select → transform → deselect).

### Cut candidates

- Auto-arrange (manual placement only) — saves 4 days.

- Mirror operation — saves 1 day.

- Ortho camera toggle — saves 1 day.

# 5. Phase 3 — End-to-end slice + G-code parser + 3MF I/O (effort: ~3.5 person-weeks)

Goal: load model → slice → produce G-code → parse it into the typed model. Also: build the 3MF reader/writer module that this project will use everywhere a printer or another slicer touches a project file. Settings UI minimal, but the loop is closed and the foundation for preview, plugins, and printer send exists.

### Deliverables

- Slice orchestration: FFI call running on a worker thread.

- Progress events streaming to UI (requires FFI extension for progress callback).

- Error surfacing: libslic3r errors translated to user-readable messages with offending setting where identifiable.

- G-code output to project directory.

- Post-slice summary: time estimate, filament usage in mass + length.

- Slice button + progress bar in UI.

- Typed G-code parser: streaming, builds typed sequence (Move / Comment / LayerChange / ToolChange / Other) with feature-type annotation. Designed to be used by both preview and plugin systems.

- G-code serializer: round-trip through typed model preserves byte-equivalent output (golden tests).

- Header metadata parser: extracts estimated time, filament use, layer count, settings from G-code comment blocks.

- **3MF reader/writer utility.** Read and write .3mf (project) and .gcode.3mf (sliced project with embedded G-code, thumbnails, metadata). This is a shared utility — not all consumers need it. Used by: Phase 2 (project import from other slicers), Phase 5 (our own project save format), Phase 6 (preview drag-drop of sliced files), Phase 7a (A1 mini driver wraps slice output here), Phase 8 (compose hook produces sliced 3MF for platecycler). Not used by: Phase 7b (U1 driver sends raw G-code, no 3MF involvement).

### Exit criteria

- A user can load a Benchy, click slice, get G-code, and the G-code parses cleanly into the typed model and round-trips identically.

- Parser handles 50MB G-code in under 3 seconds.

- 3MF round-trip: read a Bambu Studio .3mf, write it back, the result is structurally equivalent (model geometry, plate metadata, settings preserved within Bambu format expectations).

- Verification approach: parse, re-serialize, byte-diff equals zero on G-code; structural-diff equals zero on 3MF. This is the project's independent oracle — no external slicer needed.

### Cut candidates

- Header metadata parser deferred to Phase 6 — saves 1 day.

- Per-plate filament cost calculation — saves 1 day.

- 3MF write of complex Bambu metadata extensions (write minimum-viable 3MF, validate by sending a job to A1 mini in Phase 7a) — saves 2 days but raises Phase 7a risk.

# 6. Phase 4 — Settings UI (effort: ~5 person-weeks)

Goal: the cascade-aware settings UI that is this project's primary differentiator. Includes printer-aware visibility filtering, slot-adaptive layout for multi-extruder/toolchanger printers, hover-revealed cascade ladder, and first-class per-object override editing.

### Deliverables

- Data-driven form components: number, percent, dropdown, enum, color, multi-select array.

- Category navigation (Quality, Walls, Top/Bottom, Speed, Travel, Multiple Extruders, Support, Adhesion, etc.) generated from introspection.

- Mode filter (Simple / Advanced / Expert) honoring per-option mode metadata.

- Printer-aware visibility filter: options hidden when not applicable to the active printer's capabilities (FR-UI-7). Search still finds hidden options with 'not applicable' badge.

- Slot-adaptive layout (FR-UI-8): single-pane for slot_count=1, per-slot tab strip for slot_count≥2. Synchronized-edit toggle defaults ON for multi-slot.

- Project / Object editing context tabs (FR-UI-9): writes from the panel land in the active tier (project or per-object). Object tab is disabled when no object is selected; auto-selects project tab when the selection clears.

- Per-setting source-layer breadcrumb (chips colored by layer hue) inline with each setting row (FR-CAS-7).

- Hover-to-reveal cascade ladder: opens on row hover with a short close-delay so the cursor can move to the ladder; rendered via portal at body level so it's not clipped by the scroll container; shows every layer with its defined value, em-dash for undefined, winner highlighted, overridden layers marked, plus the cascade fallback when an absolute override is in effect (FR-CAS-7).

- Objects-overriding-this badge (FR-CAS-7b): on the project tab, when N objects override a given setting, show a small badge with up to 3 filament-color dots (the objects' filament colors) plus +X overflow. The ladder's per-object section lists each overriding object; clicking it selects the object and switches to the Object tab.

- Per-object overrides for any setting (FR-3D-3 expanded): the Object tab edits the object-tier overrides; storage is per-object {setting_id: value} map; UI enforces libslic3r's object/region scope (settings outside that scope are read-only in the Object tab with a 'project-scope setting' badge).

- Reset action per row, appearing when the active editing tier has a value: drops the override and falls back to the cascade resolution underneath.

- Override count indicator at category and panel level.

- Diff view: changes vs printer default, changes vs last save.

- Tooltips combining libslic3r tooltip + 'why this matters' annotations (initially seeded with ~30 hand-written annotations for highest-impact options).

- Inline validation against libslic3r config_validate.

- Support toggle: simple on/off per object (FR-3D-6 from PRD). Libslic3r generates supports from cascade settings; this UI only flips the toggle. Paint-on supports is post-MVP.

- Build plate selector: dropdown listing the active printer's supported plates (per FR-CAS-9). Selection updates the BuildPlate cascade layer; printer-reported plate (where available) is the default with a visible badge when the user overrides.

### Exit criteria

- 5-user UX test passes: given a project where a value differs from default, 5/5 users identify the source layer within 10 seconds — by reading the inline breadcrumb or by hovering for the ladder.

- A1 mini and U1 both render their full settings panel correctly: A1 mini hides toolchange options, U1 hides purge volumes matrix; both show priming tower geometry settings; U1 shows 4-slot tab strip while A1 mini shows single pane.

- Per-object overrides: editing a setting in the Object tab affects only that object; the project tab's row for the same setting shows the objects-badge with the object's color dot.

- Settings panel re-renders under 50ms on cascade change (single-slot) and under 100ms (4-slot).

### Cut candidates

- Diff vs another plate — saves 2 days.

- 'Why this matters' annotations beyond the first 30 — saves 3 days.

- Synchronized-edit affordance on multi-slot tab strip (users edit each tab independently) — saves 2 days. Hurts UX for the common 'configure all toolheads identically' case.

- Objects-overriding-this badge with per-object click-through in ladder (badge stays, click-through cut) — saves 1 day.

# 7. Phase 5 — Multi-printer project model (effort: ~3 person-weeks)

Runs partly in parallel with Phase 4. Goal: multi-plate, multi-printer projects as the default workflow.

### Deliverables

- Project model with N plates, each bound to a printer.

- Plate tab UI: add, remove, switch plates.

- Printer assignment per plate, with cascade re-resolution and validation warnings on mismatched settings.

- Plate-level metadata (FR-MP-7): cycle_count per plate (default 1, integer 1–999), composition order. Stored in project file.

- Model material → slot binding model (FR-MP-8 foundations): bindings are first-class project state, validated at cascade resolution. Printer-state-driven availability check stubbed (real polling lands in Phase 7c).

- Project save/load uses .3mf via the 3MF I/O module from Phase 3. Project metadata includes plate-printer bindings, plate-level metadata (cycle counts, composition order), model→slot bindings, and per-plate cascade overrides. The format extends the standard 3MF metadata namespace; files round-trip with Bambu Studio for shared geometry but our extensions are ignored by other slicers.

- Bed visualization updates per plate based on assigned printer.

- Move-object-between-plates operation.

- Autosave every 30s with recovery on launch.

### Exit criteria

- Create a 3-plate project, assign Plate 1 to A1 mini and Plates 2–3 to U1, slice all three, save and reload with all settings preserved including per-plate cycle counts and material bindings.

### Cut candidates

- Object-move-between-plates — saves 2 days.

- Autosave recovery (still autosave, but no recovery wizard) — saves 1 day.

- Per-plate cycle count UI (defaults to 1, no user control) — saves 1 day. Breaks PlateCycler value prop; cut last.

# 8. Phase 6 — G-code preview (effort: ~3 person-weeks)

Goal: production-quality in-app G-code visualization. Hard requirement, not a polish item. Builds directly on the typed G-code parser from Phase 3.

### Deliverables

- Renderer for extrusion paths: line segments rendered with feature-type, speed, flow, or layer-time coloring.

- Buffer geometry strategy: vertices generated once from the typed model, color attribute updated per color mode without re-tesselation.

- Layer slider: single-layer view, up-to-N view, and layer-range view. Keyboard shortcuts for navigation.

- Travel and retraction toggles.

- Hover inspection: raycast onto segments, display command, position, extrusion, speed, feature, layer.

- Per-layer stats panel: time, filament used, max speed, layer height, feature breakdown.

- Full-job stats panel: total time per feature type, filament per extruder, layer count, bounding box.

- Color-blind-safe default palette; alternate palette selectable.

- Drag-drop external files for standalone preview (no slice required): supports both .gcode and .gcode.3mf. For .gcode.3mf, the 3MF I/O module from Phase 3 unpacks the embedded G-code, extracts plate metadata, surfaces thumbnails.

- Header metadata extraction surfaced in stats panel (estimated time, filament use, slicer-of-origin). For .gcode.3mf, the panel also shows the embedded plate thumbnail and any per-plate metadata.

- Performance: 50MB file end-to-end in under 5 seconds; 60fps layer slider; under 1.5GB memory.

### Exit criteria

- Load a 50MB production G-code (e.g. a multi-hour multi-material print), step through layers, switch color modes, and inspect segments — all without external tools.

- Performance targets met on the project lead's reference hardware (integrated GPU laptop).

- Preview correctly visualizes G-code from this app's slicer, and also G-code from foreign sources (Orca, Cura, Prusa) for compatibility.

### Cut candidates

- Layer time and flow color modes (keep feature type + speed) — saves 2 days.

- Per-layer stats panel (keep full-job stats) — saves 2 days.

- Drag-drop external .gcode (only preview after slice) — saves 1 day. Hurts the standalone story; cut last.

- Layer-range view (keep up-to-N only) — saves 1 day.

# 9. Phase 7 — Printer connectivity and filament sync (effort: ~6 person-weeks total across 3 sub-phases)

Goal: send-and-monitor for both MVP printers, plus filament sync and material-binding UX. Highest-risk phase for surprises. Filament sync is a major UX investment that materially differentiates the product.

## 7a. Bambu A1 mini (effort: ~2 person-weeks)

- LAN MQTT connection using access code + serial.

- Wrap sliced G-code into .gcode.3mf for send (Bambu's required format): use the 3MF I/O module from Phase 3, populate Bambu's metadata extensions (plate thumbnails, filament aggregates, print time, AMS bindings). Validated end-to-end by sending real prints. Spike 3 from Phase 0.5 has already characterized the metadata format.

- Send print to printer.

- Status polling: state, current layer, temperatures.

- AMS lite state read.

- Read currently-mounted build plate (A1 mini reports it in current firmware); feeds the BuildPlate cascade layer with the printer-reported value as default.

- Commands: pause, resume, stop.

- Real print: send Benchy and a multi-color test from AMS lite.

## 7b. Snapmaker U1 (effort: ~2 person-weeks)

- HTTP API connection.

- U1 printer profile (CoreXY toolchanger, 4 independently-configurable toolhead slots). Defaults assume 4× identical 0.4mm steel as shipped; data model permits any combination. Start/end G-code and tool-change macros validated against Snapmaker Orca's published profile as reference.

- Toolchanger G-code emission: libslic3r already supports this pattern (used by Prusa XL); validate output matches what U1 firmware expects.

- Send plain .gcode (not .gcode.3mf): the U1 is Klipper-based and expects raw G-code over HTTP, not a 3MF wrapper. The send path explicitly skips the 3MF wrap that Phase 7a uses.

- Status polling: state, currently-mounted toolhead, per-toolhead loaded filament, temperatures.

- Read currently-mounted build plate where the U1 firmware reports it; otherwise the user selects manually from the U1's supported plate list.

- Commands: pause, resume, stop.

- Per-toolhead independent nozzle/hotend configuration in the cascade: nozzle size, hotend type, max temperature each settable per slot.

- Real prints: single-material (one toolhead), 2-material, 4-material color test exercising all toolheads, tool-change-stress test. Mixed-nozzle-size validation is post-MVP.

## 7c. Filament sync and assignment (effort: ~2 person-weeks)

Filament sync ties the printer comms (7a, 7b) to the cascade (Phase 1) and the multi-printer project model (Phase 5). The work is mostly UX and data-model integration — the underlying primitives exist by this point.

- Printer-side state polling: per-slot loaded-filament identity (type, color, brand/SKU where reported).

- Filament state panel per printer in the UI: live loadout view, last-updated timestamp, manual refresh.

- Filament profile library: ship with profiles for common Bambu and generic filaments, plus a custom-profile editor that lives in the cascade.

- Manual override of printer-reported filament identity (for third-party spools, missing RFID, etc.) with a visible badge distinguishing manual from auto.

- Project material binding UI: per-plate per-printer mapping from model material index → physical slot, with the loaded filament shown inline at each slot.

- Per-(plate, printer) binding persistence: reassigning a plate from A1 mini to U1 surfaces the U1's stored binding or prompts for one.

- Auto-binding heuristic: on first plate-to-printer assignment, attempt to bind model materials to physical slots by filament family match. User confirms or adjusts.

- Mismatch detection at cascade resolution and at slice-time: material family mismatch (PLA/PETG/ABS), temperature range outside ±10°C of profile, color mismatch (informational). Configurable warn-vs-block on slice.

- Sync-on-send: emit the binding into 3MF/G-code metadata in the format each printer expects.

- Multi-color paint UI assigns paint regions to model material indices, never directly to physical slots; the binding layer always mediates.

- Real test: load 4 different filaments in the U1, slice a 4-color print, verify the printer mounts the correct toolhead for each color.

- Real test: same project sliced for A1 mini and U1 produces correct bindings for each, with different physical loadouts on each machine.

### Exit criteria

- Both printers receive jobs from the app and complete prints successfully without manual G-code editing.

- Status of both printers can be monitored simultaneously.

- Filament sync works: changing what's loaded in a printer is reflected in the app within one poll cycle; mismatches are caught before slice.

- A multi-color project sliced for either printer assigns model materials to the correct physical slots and prints with the expected colors.

### Cut candidates

- AMS lite filament identity read (keep slot count but not filament identity) — saves 3 days but degrades multi-color UX. Strong candidate to keep.

- Pause/resume/stop commands — saves 2 days. Send-only is acceptable for MVP if needed.

- Auto-binding heuristic (manual binding only on first assignment) — saves 2 days. Hurts UX but binding still works.

- Mismatch detection beyond material family (skip ±10°C check) — saves 1 day.

- Manual filament-identity override (use printer-reported only) — saves 2 days. Hurts third-party-spool users.

# 10. Phase 8 — Plugin system (effort: ~4 person-weeks)

Runs in parallel with Phase 7 since it has no printer dependency, and reuses the G-code parser from Phase 3. Goal: working Lua plugin host with hot reload.

### Deliverables

- mlua integration with sandboxed Lua 5.4.

- Plugin manifest schema (TOML): name, version, hooks, printer compatibility, settings.

- Lua bindings to the typed G-code model from Phase 3 (no re-implementation): gcode.lines(), gcode.layers(), gcode.commands(), insert/replace/remove operations, printer + plate metadata access.

- Lua bindings to live filament state from Phase 7c (read-only): per-slot identity, loaded flag, mismatch state. Enables material-aware plugins.

- Hook dispatch at pre-slice, post-slice, pre-send, and compose. The compose hook (FR-PL-5) is the project-level hook that receives all sliced plates plus project metadata and returns a transformed bundle.

- Compose hook API: 3MF-level read/write (thumbnails, filament aggregates, print time totals), plate composition order, plate count transformation. Designed to support PlateCycler-style workflows.

- Plugin-declared settings integrated into the cascade UI under a Plugins category. Plate-level metadata (cycle counts, composition order) editable in the plate tab UI.

- Hot reload via file watcher on the plugins folder.

- Plugins panel in UI: enabled state, errors, per-printer scoping.

- **Flagship example plugin — platecycler.** Port the existing platecycler Python tool to a Lua plugin using the compose hook. Reads per-plate cycle_count metadata, concatenates plate G-codes with the Chitu PlateCycler swap macro between them, rewrites 3MF metadata aggregates. Validated end-to-end on the project lead's A1 mini + PlateCycler hardware.

- Three smaller example plugins exercising the per-plate hooks: 'beep at layer N' (post-slice), 'insert pause at layer N' (post-slice), 'rewrite bed temperature by range' (pre-slice).

- Plugin authoring guide: documents the four hooks, the typed G-code model, the filament-state API, and walks through writing each example from scratch.

### Exit criteria

- A non-Rust developer can write a plugin from the example as a starting point and have it active in under 60 seconds.

- Plugin errors are caught and surfaced without crashing the host.

- platecycler plugin produces a .platecycler.3mf from a multi-plate A1 mini project that prints successfully on the project lead's A1 mini + PlateCycler hardware. This is the proof that the compose hook + plugin architecture works end-to-end.

### Cut candidates

- Pre-slice hook (only post-slice, pre-send, and compose) — saves 3 days.

- Plugin-declared settings UI integration — saves 4 days. Plugins still work; users edit Lua to configure.

- Plugin authoring guide (release with example plugins only) — saves 2 days. Hurts plugin ecosystem growth.

- platecycler-as-shipped-example — do NOT cut. This is a primary MVP differentiator (per PRD §3.3 success criteria). If Phase 8 runs hot, cut other Phase 8 work first.

# 11. Phase 9 — Polish, Linux flatpak, and release prep (effort: ~2 person-weeks)

Goal: Linux flatpak build, basic onboarding, release-readiness. Windows and macOS native builds are post-MVP and removed from this phase.

### Deliverables

- Linux flatpak build: manifest, runtime selection (Freedesktop SDK), bundled libslic3r + webview deps, GPU acceleration via flatpak hardware permissions.

- Distribution channel decision: Flathub submission, or self-hosted .flatpakref + repo? Recommendation: self-hosted for MVP (faster iteration, no Flathub review delay), Flathub submission post-MVP.

- WSL2 best-effort: validate that the flatpak runs under WSL2 with WSLg; document known limitations (printer LAN comms via WSL2 NAT may need user-side network setup). Not a blocker if WSL2 fails.

- First-run onboarding: pick your printers from a list including A1 mini and U1, prompt for printer access info.

- Project file format: .3mf extension finalized (per FR-MP-4).

- OrcaSlicer profile importer (one-time migration tool, not a runtime dependency): read .json profile bundles, map to cascade layers. Optional for users; the app ships with first-class profiles for both MVP printers.

- Documentation: getting started (Linux flatpak install path), plugin authoring guide, troubleshooting. Does not reference any other slicer as a required tool.

- Release notes and known issues.

### Exit criteria

- Flatpak installs and runs cleanly on Ubuntu, Fedora, and Arch with current flatpak runtimes.

- Onboarding completes in under 5 minutes for a user who has the A1 mini access code at hand.

- Independence audit passes: an external tester on a clean Linux machine completes the full workflow with no other slicer installed.

- All MVP success criteria from PRD section 3.3 are met.

### Cut candidates

- WSL2 validation — saves 1–2 days. Already best-effort, easy to cut.

- OrcaSlicer profile import — saves 4 days. Painful for adoption from existing users; cut only if necessary.

- Flathub submission (self-hosted distribution only) — saves 2 days plus review wait. Reasonable for early release; do Flathub post-MVP.

# 12. Effort at a glance

| **Phase** | **Effort** | **Depends on** | **Key output** | **Cut at risk?** |
| --- | --- | --- | --- | --- |
| 0 — Foundation | 2 pw | — | Tauri app loads, FFI linked | No |
| 0.5 — Validation spikes | 1 pw | Phase 0 | Five findings documents; assumptions de-risked | No |
| 1 — Rule cascade + adapter | 4 pw | Phase 0.5 | Rule resolver, translation adapter, manifest, scope validation, trace tooling | Low |
| 2 — 3D viewport | 4 pw | Phase 0 | Models load and transform; perf decision made | Medium |
| 3 — Slice + parser + 3MF I/O | 3.5 pw | Phase 1, libslic3r-FFI | STL to G-code closed loop; typed parser; 3MF read/write | Low |
| 4 — Settings UI | 5 pw | Phase 1, Phase 3 | Cascade-aware settings, slot-adaptive, printer-aware visibility, hover ladder, per-object overrides, editing tabs | Low (this is the product) |
| 5 — Multi-printer project | 3 pw | Phase 1, Phase 2 | Plates bound to printers, plate metadata, binding model | Low (this is the product) |
| 6 — G-code preview | 3 pw | Phase 3 | Full in-app preview, .gcode and .gcode.3mf, independence achieved | Low (hard requirement) |
| 7 — Printer + filament sync | 6 pw | Phase 3 (3MF), Phase 5 | A1 mini and U1 send-monitor-sync | Medium |
| 8 — Plugin system | 4 pw | Phase 3, Phase 7 (filament-state) | Lua plugins with hot reload, platecycler ships | Medium |
| 9 — Polish + Linux flatpak | 2 pw | Phase 8 | Linux flatpak; WSL2 best-effort | Medium |

*Total: ~37.5 person-weeks of focused work. Some phases can be interleaved with their predecessors (Phase 4 ↔ Phase 5, Phase 6 ↔ Phase 7) by context-switching within the same developer**'**s brain on the same day; this does not reduce total effort but may reduce calendar time. Other phases hard-block: Phase 7c filament sync requires Phase 7a and 7b done; Phase 8**'**s platecycler validation requires Phase 7a done. Do not plan calendar from effort; plan effort from effort.*

*pw = person-week of focused work, assuming a productive day is ~6 hours of concentrated effort.*

# 13. Milestones and demos

Six hard milestones, each with a demo and a go/no-go check. None depend on a third-party slicer for verification.

| **Milestone** | **After phase** | **Demo** | **Go/no-go decision** |
| --- | --- | --- | --- |
| M0.5 — Engine assumptions validated | 0.5 | Five spike findings documented; any failures have plan-revision PRs | Confirm Phase 1+ assumptions are sound, or revise the plan before committing |
| M1 — Slice loop closed | 3 | Load STL or .3mf, slice, produce G-code, parse + round-trip identically, write .gcode.3mf | Engine integration, parser, and 3MF I/O confirmed; if not, fix or pivot before investing in UI |
| M2 — UX validation | 4 (with Phase 5 in progress) | 5-user test of cascade visibility passes; both printer settings UIs render correctly with adapted layout and visibility | Cascade UX and printer-adaptive UI confirmed; redesign now if either fails |
| M3 — Preview production-ready | 6 | Step through a 50MB G-code and a .gcode.3mf, switch color modes, hover-inspect, all in-app | Independence achieved; the app stands alone as a slicer + viewer |
| M4 — First print | 7a (first half of Phase 7) | Real Benchy from A1 mini via .gcode.3mf send, and a 4-color toolchange test from U1 via .gcode send, sliced and verified via in-app preview only. Followed by a multi-plate PlateCycler print on the A1 mini after Phase 8 lands. | Printer pipeline + filament sync confirmed; preview validated against real prints; both send formats validated |
| M5 — MVP candidate | 9 | Both printers with live filament sync, multi-plate, plugins including platecycler validated on A1 mini + PlateCycler hardware, cross-platform installers, full independence audit | Ship to closed beta or take an extra polish iteration for fixes |

# 14. Cut order if schedule slips

If the schedule slips, cuts come from this list in order. The intent is to preserve the core value proposition (multi-printer + cascade + plugins + sliceable + previewable + printable + independent + platecycler-on-A1-mini) above all else.

*Two features are NOT on the cut list — they are hard MVP requirements. G-code preview ships (internal cuts within Phase 6 exist but the feature itself stays). The platecycler example plugin ships (it is a primary differentiator per PRD §3.3 success criteria and the proof of the plugin architecture).*

| **Order** | **Feature to cut** | **Time saved** | **Cost** |
| --- | --- | --- | --- |
| 1 | OrcaSlicer profile importer (Phase 9) | 4 days | Existing Orca users build profiles from scratch. App still ships with first-class profiles for both MVP printers. |
| 2 | Plugin-declared settings UI (Phase 8) | 4 days | Plugins still work; users edit Lua to configure. |
| 3 | AMS lite read (Phase 7) | 3 days | Multi-color UX worse on A1 mini. |
| 4 | Pre-slice plugin hook (Phase 8) | 3 days | Plugins limited to post-slice and pre-send. |
| 5 | Flathub submission (self-hosted only) (Phase 9) | 2 days + wait | Slower public discovery; reasonable for early release. |
| 6 | Preview: layer time + flow color modes (Phase 6) | 2 days | Preview keeps feature-type + speed coloring. |
| 7 | Preview: per-layer stats panel (Phase 6) | 2 days | Full-job stats panel retained. |
| 8 | Diff vs another plate (Phase 4) | 2 days | Diff still works against printer default. |
| 9 | WSL2 validation (Phase 9) | 1–2 days | Already best-effort; cut without ceremony. |
| 10 | Preview: layer-range view (Phase 6) | 1 day | Up-to-N view retained. |
| 11 | Auto-arrange (Phase 2) | 4 days | Manual placement only. |
| 12 | Object-move-between-plates (Phase 5) | 2 days | Users duplicate objects on target plate. |

*Total available cut budget if every item is taken: ~30 days. Note that all preview-related cuts are internal trims — the preview itself never falls below **'**functional production-quality viewer**'**.*

# 15. External dependencies and unknowns

- **FFI extensions.** Logging redirect, progress callback. Owned in-house; sequenced into phases that need each. Windows/macOS-specific FFI work (symbol exports, macOS CMake) is deferred to post-MVP.

- **Flatpak runtime.** Choose Freedesktop SDK version that matches Tauri's webview and libslic3r dependencies. Lock in by Phase 9.

- **Bambu API stability.** Community libraries track changes; pin to a known-working version and document the fallback.

- **Snapmaker U1 firmware updates.** Could change the HTTP API surface. Test on the firmware version you're targeting; document it.

- **OrcaSlicer upstream cadence.** Decide on a submodule bump policy (e.g. monthly, or only at upstream tagged releases). Recommend tagged releases only.

# 16. What follows the MVP

Out of scope for this plan, but worth listing so MVP decisions don't paint into corners:

- Native Windows build: FFI symbol exports, MSI installer, code signing. The data model and architecture do not preclude this.

- Native macOS build: CMake adjustments, notarized .dmg, code signing.

- Auto-updater (Tauri's updater wired with signed updates).

- Flathub submission for broader discoverability.

- Additional printer integrations: Klipper/Moonraker, OctoPrint, additional Bambu models, Prusa.

- Calibration wizards: flow ratio, pressure advance, temperature towers, max volumetric speed.

- Paint-on supports and modifier meshes.

- Plugin marketplace and signed plugin distribution.

- WASM plugin runtime alongside Lua.

- Wgpu native viewport if webview perf is insufficient at production scale.

- Print farm / fleet management UI.

- Community profile sharing format.

- Advanced G-code preview features: time-based scrubbing, head trajectory animation, simulated print time visualization.