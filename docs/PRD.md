# Product Requirements Document

Multi-Printer Slicer (working title)

*MVP scope — built on orca-slicer-ffi + Tauri*

| **Field** | **Value** |
| --- | --- |
| Document version | 0.1 (draft) |
| Status | Pre-development |
| Owner | Project lead (FFI author) |
| Target MVP effort | ~35.5 person-weeks (see Execution Plan) |
| Slicing engine | OrcaSlicer libslic3r via orca-slicer-ffi |
| Shell | Tauri (Rust core, web frontend) |
| License | AGPL-3.0-or-later (inherited from libslic3r) |

# 1. Executive summary

Existing slicers — particularly OrcaSlicer and its PrusaSlicer ancestor — are built around a single-active-printer mental model. Owners of multiple machines pay a constant context-switching tax: pick the printer, pick the filament that matches that printer, remember which overrides applied where, repeat for every plate. The settings inheritance model exists but is opaque, with little visibility into what was inherited from where or what a given override is actually doing.

This project delivers a new desktop slicer UI that treats multi-printer operation as the default, not the edge case, and that makes settings inheritance a first-class, visible part of the interface. It does not replace the slicing engine — libslic3r is mature, fast, and battle-tested — but replaces the UI and the configuration model that surrounds it.

The MVP targets two specific printers (Bambu Lab A1 mini and Snapmaker U1) configured simultaneously, with full slice-and-send workflow, a transparent settings cascade, and a Lua-based plugin system for G-code manipulation.

# 2. Problem statement

## 2.1 Multi-printer workflow is hostile

Users with two or more printers describe the same friction across forums and Discord servers: switching printers in OrcaSlicer means re-selecting filament, sometimes losing per-object settings, and never being sure whether a tuned setting will carry over. There is no native concept of 'this plate prints on machine A, that plate on machine B' within a single project. Workarounds involve duplicate project files, named profiles per printer, and tribal knowledge.

## 2.2 Settings inheritance is invisible

OrcaSlicer and PrusaSlicer both implement a layered profile system (vendor → printer → filament → user) but expose it as flat dropdowns. When a setting differs from default, users cannot easily see: where the value came from, what would happen if they reset it, or whether their override is fighting a printer-specific tuning. This drives both new-user confusion and expert-user paranoia.

## 2.3 G-code post-processing is a second-class citizen

Existing slicers permit external scripts as a final post-processing step, but offer no structured G-code model, no per-printer scoping, and no UI integration for plugin-defined settings. Users who want to insert pauses, modify temperatures by layer, or add machine-specific commands write fragile string-manipulation scripts.

# 3. Goals and non-goals

## 3.1 MVP goals

- **Multi-printer by default.** Two printers configured simultaneously, per-plate printer assignment within a single project, side-by-side awareness in the UI.

- **Transparent cascade.** Every setting visibly shows its source layer (default / printer / extruder+nozzle / filament / user / project), with one-click reset-to-inherited and clear override indicators.

- **Slice and send.** Load model → assign to plate(s) → slice → send to Bambu A1 mini or Snapmaker U1 over LAN, with basic status feedback.

- **Plugin system.** Lua-based G-code post-processing with structured G-code model, pipeline hooks, and per-printer scoping. (Automatic hot-reload of plugin files is deferred to post-MVP — plugins load on launch and can be reloaded manually.)

- **Cross-platform.** Linux flatpak for the MVP. Windows and macOS support are post-MVP. WSL2 compatibility is a tested nice-to-have if the flatpak runs cleanly; not a requirement.

- **Runtime independence from other slicers.** Every workflow — load, slice, preview, send, monitor — completes inside this app. No installation of OrcaSlicer or any other slicer is required or recommended at any point.

- **In-app G-code preview.** Layer slider, multiple color modes, hover inspection, and travel toggle, performant on real production G-code files.

- **Filament sync and assignment.** Live read of loaded filaments from each connected printer; per-printer mapping of model materials to physical slots; mismatch detection before slice; bindings persist per-printer so a multi-printer project resolves correctly for each target.

## 3.2 Explicit non-goals for MVP

- Network printer support beyond the two MVP targets

- Cloud sync, accounts, or any backend service

- Native Windows and macOS builds (Linux flatpak only for MVP; WSL2 best-effort)

- Calibration wizards (flow, pressure advance, etc.)

- Multi-plate projects beyond 4 plates

- Mobile companion app

- AI features of any kind

- Marketplace, plugin store, or community profile sharing

- Vase mode UI polish, organic supports tuning UI, paint-on supports beyond a basic implementation

- Forking or maintaining a divergent libslic3r

## 3.3 Success criteria

The MVP is considered successful when:

- A user owning both an A1 mini and a U1 can slice a multi-plate project assigning plates to either printer, send G-code to both, and have prints complete successfully without manual G-code editing.

- Settings cascade visibility passes user testing: 5 of 5 testers, given a project where layer height differs from default, can correctly identify which layer of the cascade is responsible within 10 seconds.

- Time-to-first-slice for a new user with a known printer is under 5 minutes from app launch.

- A plugin that inserts a beep at layer change can be written, dropped into the plugins folder, and enabled from the Plugins UI (active on the next launch, or via a manual reload). The "active within 60 seconds" automatic-reload loop is a post-MVP refinement.

- A user can preview a 50MB production G-code in-app: layer slider, color modes, hover inspection, and per-job stats all functional without any external tool.

- A user with an A1 mini + PlateCycler add-on can compose a multi-plate project, slice it, send it to the printer, and have the PlateCycler complete all plates sequentially. The platecycler plugin (compose hook) ships with the MVP and is the proof point for the plugin architecture.

- An external auditor on a clean Linux machine (flatpak installed, no other slicer software present) completes the full workflow — install app, configure both printers, slice, preview G-code, send to printer, monitor.

# 4. Target users

## 4.1 Primary persona — Multi-printer hobbyist

Owns 2–4 printers of different makes. Runs prints overnight and across days. Currently uses OrcaSlicer or a mix of OrcaSlicer + PrusaSlicer per printer family. Comfortable in a terminal, willing to install a beta, has opinions about retraction. Wants the workflow friction gone.

## 4.2 Secondary persona — Power user / small studio

Runs a print-on-demand side business or small product line. Cares about throughput, consistency, and being able to assign jobs to whichever printer is free. Will pay for software if it pays for itself in saved hours. Wants plugin extensibility for batch operations.

## 4.3 Out-of-scope personas for MVP

First-time printer owners (need wizard-driven onboarding we're not building), production environments with print farms (need fleet management we're not building), and resin printer users (different engine entirely).

# 5. UX principles

These are the non-negotiable design rules. Every UX decision is checked against this list.

- **Show the source.** If a value differs from the default, the UI shows where it came from — which rule, in which file, at which specificity. The rule cascade is designed to make this affordable (FR-CAS-7).

- **Defaults are intelligent.** Printer profile + filament profile + nozzle combination should produce a printable result with zero user input.

- **Multi-printer is the default.** No 'active printer' modal state. Printer is a property of the plate.

- **Reset is one click.** Any override can be reverted to its inherited value without a confirmation dialog.

- **Search beats hierarchy.** Setting search is faster than category navigation. Both work.

- **Tooltips explain consequence, not vocabulary.** Not 'maximum volumetric extrusion rate' but 'how fast you can push filament through the hot end before it can't melt fast enough'.

- **Failure is informative.** When slicing or sending fails, the error tells the user what to change.

- **Standalone at runtime.** The app is fully functional without any other slicer installed. Users never need to install OrcaSlicer, PrusaSlicer, or any third-party tool to complete a print workflow.

# 6. Features and requirements

## 6.1 Configuration model: rule cascade

Configuration is expressed as a rule cascade with selector-based overrides and CSS-like specificity. A separate adapter layer translates resolved logical settings into the flat DynamicPrintConfig that libslic3r consumes. The detailed design is captured in the project's profiles strategy document; this section names the requirements that follow from it.

### Concept summary

A config file is a list of rules. Each rule has zero or more when.* predicates (the selector) and one or more set.* actions (settings to apply). At slice time, the resolver evaluates a context — facts about the current printer, mounted build plate, mounted filaments, toolhead/nozzle configuration per slot, project plate, etc. — and for each setting picks the value from the matching rule with the highest specificity (count of matching predicates). Same-specificity ties resolve by source load order: later sources win.

Resolution runs in two phases. The first phase resolves the authored cascade — the rule files that encode domain knowledge: default rules, printer profiles, build_plate files, filament profiles. Within this phase, files load in the order default → printer → build_plate → filament[slot]; specificity wins; same-specificity ties broken by source load order. A printer file's unconditional set.bed_temp = 60 (specificity 0) loses to a filament file's when.filament.type = 'PLA' → set.bed_temp = 55 (specificity 1) regardless of which loaded first. Load order matters only when specificities tie.

The second phase applies absolute overrides on top, in three tiers: user profile, then project file, then per-object overrides for the active object. These are flat unconditional sets generated from the UI when the user changes a value or saves a profile. They win unconditionally over the authored cascade — specificity does not protect against them. Among themselves, object wins over project wins over user. This matches user expectation: 'I clicked this value to override what I saw' should always win against authored rules the user may not have known about.

Mental model for readers familiar with CSS: user, project, and per-object settings behave as if every set.* entry carried !important. The authored cascade (default + printer + build_plate + filament) is the normal-specificity tier; user is one !important tier; project is a higher !important tier; per-object is the highest !important tier. !important does not raise specificity within a tier — it puts the declaration in a separate tier that wins regardless of specificity in the tier below. Resolution among declarations in the same tier follows the tier's own rules (specificity-and-source-order for the authored tier; later-source-wins for the override tiers).

Context state is structured data, not rules. The active printer profile declares slot count, supported build plates, and per-slot extruder/nozzle configuration (nozzle diameter, hotend type, max temperature). The active build plate file declares its identity and surface properties. Filament binding to slots is project state. The resolver consumes this state as predicates — rules can write when.toolhead.diameter = 0.6 or when.plate.type = 'PEI' — but the state itself is not a cascade of rules.

Presentation vs mechanism. The UI presents the cascade as values-by-source-file (an ordered ladder: printer → build_plate → extruder/nozzle → filament → user → project → object, with the winning layer highlighted). This is a presentation summary, not the resolution mechanism. Underneath, the resolver picks rules by specificity and source order across all loaded files; the ladder shows the highest layer that contributed the winning value. The two views are consistent — a rule in the filament file that wins specificity will display as 'filament' in the ladder — but the ladder is a friendly summary, not a literal trace of the resolution.

### Functional requirements

- **FR-CAS-1.** Configs are TOML files with [[rule]] blocks (full form) and section shorthand for the common single-predicate case ([filament.type.PLA] sugar for when.filament.type = 'PLA'). Configs contain pure data — no embedded code, expressions, or template strings.

- **FR-CAS-2.** The resolver, given a context, returns for each setting: the effective value, the source rule (file path + line + rule index), and the rule's specificity. This is the trace that powers the source-disclosure UX in FR-CAS-7.

- **FR-CAS-3.** Resolution runs in two phases. (1) Authored cascade: load rules from default → printer → build_plate → filament[slot] in that order. Within this set, specificity wins; same-specificity ties broken by source load order (later sources win); within a file, later rules win over earlier rules of equal specificity. (2) Absolute overrides: user profile, then project file, then per-object overrides for the active object. These behave as if every set.* entry carried CSS's !important — they win over the authored cascade regardless of the cascade's specificity. Per-object overrides beat project; project beats user. The tiers do not commingle: a higher-specificity authored rule never beats a project or object override.

- **FR-CAS-4.** User profiles are flat unconditional sets: the UI lets a user save current configured settings as a named profile. Saved profiles are emitted as TOML with set.* entries only — no when.* predicates, no [[rule]] blocks. They apply as the user !important tier on top of the authored cascade (FR-CAS-3); the user's intent is not in competition with rule specificity.

- **FR-CAS-5.** Project files store project-specific overrides as flat set.* values, plus per-object overrides keyed by object identity. The project tier wins over the user tier; per-object overrides for the active object win over project. The user's project-specific and per-object intent always wins over both upstream rules and the user's saved profile.

- **FR-CAS-6.** Per-slot configuration: rules can use when.slot = N or when.toolhead.* predicates. For multi-extruder or toolchanger printers (U1), the adapter resolves the cascade independently per slot and emits the per-extruder vectors libslic3r expects. Per-slot context state (nozzle diameter, hotend type, max temperature) lives in the printer profile.

- **FR-CAS-7.** Source disclosure: every setting in the UI displays its effective value and a per-row breadcrumb showing the chain of layers that contributed to it (each layer rendered as a short chip with its color hue; the winning layer is highlighted). Hovering the row opens a cascade ladder (rendered via portal so the scroll container doesn't clip it) showing all cascade layers, each with its defined value or em-dash if undefined, the winning layer marked, overridden layers marked. When an absolute override (user, project, or object) is active, the ladder also shows what the authored cascade would have resolved to (the 'cascade fallback'), so the user knows what they'll revert to. This is the implementation of the 'Show the source' UX principle (§5).

- **FR-CAS-7b.** Objects-overriding-this badge: on the project editing tab, a setting that is overridden by one or more objects on the plate displays a small badge alongside the row showing N color dots (the filament colors of the overriding objects) and a +X overflow marker if N exceeds three. Hovering the row's ladder shows a per-object section listing each overriding object by name, its filament color, and its override value. Clicking an object in that section selects it and switches to the Object editing tab.

- **FR-CAS-8.** Right-click or context action on any setting offers 'Reset to cascade' (drops the user/project override for this setting, falling back to what the authored cascade resolves to) and, when no override is active, 'Override' (creates a new set.* entry in the user profile or project file for this setting). The active override scope (user vs project) is selectable; project is the default when editing within a project context.

- **FR-CAS-9.** Settings panel header shows count of overrides relative to printer + filament + plate defaults (i.e. how many settings the user/project changed beyond what the structured profiles imply).

- **FR-CAS-10.** A diff view lists every setting that differs from a chosen baseline (defaults only, defaults + filament, last save, another project plate).

- **FR-CAS-11.** Cascade resolution time for a full settings panel render is under 100ms on mid-range hardware for a 4-slot printer with full context. (Higher than the original 50ms target because per-slot resolution multiplies the work.)

- **FR-CAS-12.** Load-time validation: predicate dimensions and setting names are validated against the known schema; typos rejected with file:line. Scope compatibility (object-scoped settings only in object-scoped contexts) validated at load. See FR-CAS-13.

- **FR-CAS-13.** Option scope awareness: libslic3r options have scopes (print, object, region, plus SLA variants) exposed by the FFI as a bitmask. The resolver uses scope to validate rules at load time (a rule's set.* keys must be compatible with its applicable scope), to dispatch resolved values to the correct location in DynamicPrintConfig (print scope → Print's config; object scope → ModelObject::config; region scope → ModelVolume::config), and to drive UI form hints (per-object panels show only object/region-scoped options).

- **FR-CAS-14.** Translation adapter: a Rust module converts resolved logical settings into libslic3r's DynamicPrintConfig. The adapter handles two cases: identity mappings (most settings) and dimensional expansion (settings libslic3r split across hardcoded dimensions — bed temperature across 6 plate types, retraction across nozzle states, etc.; these resolve for every dimension value and emit into all corresponding libslic3r keys). The adapter is the only code that knows libslic3r's vocabulary; the rest of the system works in our logical names.

- **FR-CAS-15.** Translation manifest: a maintained Rust data structure listing which libslic3r keys are dimensional and how they expand. Estimated ~50 dimensional entries plus identity mappings for the rest of the ~666 scoped options. Maintained on libslic3r upgrades when option keys move or new dimensions appear.

- **FR-CAS-16.** Unscoped libslic3r options (~71 keys: compatible_printers, host integration markers, deprecated keys) are treated as opaque project-level metadata. Round-trippable from .3mf imports, never set by rules in our cascade.

- **FR-CAS-17.** 3MF import: when the app opens an OrcaSlicer / Bambu Studio / Snapmaker Orca .3mf, the embedded flat preset config is imported as a single flat overlay (one rule, no when.* predicates, all set.* entries). The imported state is preserved as-is; the user can later derive structured rules from it manually if they want.

## 6.2 Multi-printer project model

- **FR-MP-1.** A project may contain 1–4 plates.

- **FR-MP-2.** Each plate has exactly one assigned printer. Assignment is changeable per plate.

- **FR-MP-3.** Changing a plate's printer recomputes the cascade and re-validates settings; incompatible values surface as warnings, not silent corrections.

- **FR-MP-4.** Project file format is .3mf (extended). The app reads .3mf files authored by Bambu Studio, OrcaSlicer, and Snapmaker Orca (geometry, plate layout, available settings) as the migration path. The app writes .3mf with project extensions in our own namespace covering per-plate printer binding, plate-level metadata (cycle counts, composition order), model→slot bindings, and cascade overrides. Files round-trip with foreign slicers for shared content; our extensions are ignored by them, which is acceptable.

- **FR-MP-4b.** The app produces sliced output in whatever format the assigned printer's driver requires. Send-format selection is per-driver responsibility (see core/printer/<driver> in §8.2): the driver may wrap, transform, or pass through G-code as needed. For the MVP, the A1 mini driver wraps as .gcode.3mf with Bambu metadata extensions, and the U1 driver sends raw .gcode over HTTP. Future printer drivers add their own send paths without touching shared code; 3MF wrapping is a tool used where printers require it, not a universal output format.

- **FR-MP-5.** The 3D viewport shows the bed dimensions of each plate's assigned printer.

- **FR-MP-6.** Models can be moved between plates; arrangement preserves model position where geometrically valid.

- **FR-MP-7.** Plate-level metadata (cycle count, plate composition order) is stored on the plate and survives save/load. Composition plugins consume this metadata; default cycle count is 1.

- **FR-MP-8.** Model material → slot bindings are validated at cascade resolution and before slice. Unavailable slot bindings (slot reported missing or disabled by the printer, or unknown after stale poll) block slice with a clear error and a one-click rebind suggestion.

## 6.3 Settings UI

- **FR-UI-1.** Settings are presented in categories matching libslic3r introspection metadata (Quality, Strength, Speed, Travel, etc.).

- **FR-UI-2.** Mode filter: Simple / Advanced / Expert. Defaults to Simple. Honors libslic3r's per-option mode metadata.

- **FR-UI-3.** Global setting search returns matches across categories with category breadcrumbs.

- **FR-UI-4.** Each input is type-appropriate (number with unit, percent, dropdown, enum, multi-select for arrays, color picker for color values).

- **FR-UI-5.** Validation surfaces inline (red border + message) on invalid input; invalid project cannot be sliced.

- **FR-UI-6.** Tooltips include the option's libslic3r tooltip text plus a 'why this matters' line authored separately.

- **FR-UI-7.** Printer-aware option visibility: settings that are not applicable to the active printer's capabilities are hidden by default (e.g. purge volumes matrix on toolchanger printers where no purging happens, toolchange G-code on single-extruder filament-swap printers). Priming tower geometry settings remain visible whenever the printer uses a priming structure, independent of purging. Setting search still finds hidden options with a 'not applicable to this printer' badge.

- **FR-UI-8.** Slot-count-adaptive layout: printers with one material slot show a single nozzle/filament pane; printers with two or more slots show a per-slot tab strip with a synchronized-edit toggle. Default for multi-slot printers is synchronized-edit ON.

- **FR-UI-9.** Editing context tabs: the settings panel has a Project tab (active by default) and an Object tab (active when an object is selected in the viewport; disabled otherwise). Writes from the settings panel land in the active tab's override tier (project or per-object). The active tab is visually distinguished and shows which object is being edited when Object is active.

- **FR-UI-10.** Object library / scaffolding panel on the left side of the viewport. Sections include Primitives (cube, cylinder, sphere, cone, torus — quick test geometry), Calibration (calibration cube, temperature tower, generic flow test), and Imported (user-loaded STL/3MF files for the current session). Clicking an item adds it to the active plate.

## 6.4 3D viewport

- **FR-3D-1.** Load STL, OBJ, and .3mf (project format). Bambu Studio, OrcaSlicer, and Snapmaker Orca all save projects as .3mf; loading them is the migration path for existing users. .gcode.3mf (sliced project) is handled by the G-code preview (see FR-GP-12), not the 3D viewport. STEP is out of MVP scope.

- **FR-3D-2.** Object operations: move, rotate, scale, mirror, lay flat, auto-arrange.

- **FR-3D-3.** Multiple objects per plate; per-object setting overrides are first-class. Any setting can be overridden per object via the Object editing tab in the settings panel; overrides land in the object override tier (highest absolute-override tier; see FR-CAS-3). Per-object overrides are scoped to object/region in libslic3r's scope model — settings outside that scope cannot be overridden per object and the UI surfaces that constraint.

- **FR-3D-4.** Bed visualization includes printer-specific dimensions, exclusion zones (A1 mini AMS-adjacent zone, U1 toolhead parking bay area), and origin marker.

- **FR-3D-5.** Acceptable performance threshold: 30 fps minimum manipulating a 20M-triangle scene on integrated GPU laptop hardware.

- **FR-3D-6.** Basic supports: auto-generate, on/off toggle per object. Paint-on supports are out of MVP scope.

- **FR-3D-7.** Scene state lives in Rust in a renderer-agnostic structure (objects, transforms, mesh data, selections, gizmo state). The renderer (Three.js for MVP, possibly wgpu later) is a read-only consumer that reflects state changes pushed via Tauri events. All scene mutations go through Tauri commands; the renderer never owns authoritative state. This rule exists so switching renderers does not require touching the state model. See AD-8 for the design rationale and consequences.

## 6.5 Slicing pipeline

- **FR-SL-1.** Slice is invoked per plate. Multi-plate slice runs sequentially with overall progress.

- **FR-SL-2.** Slice runs off the UI thread; progress is reported via Tauri events.

- **FR-SL-3.** Slice failures surface with the libslic3r error message and the offending setting where identifiable.

- **FR-SL-4.** Post-slice output includes G-code, time estimate, filament usage, and per-plate summary.

- **FR-SL-5.** G-code is written to a project-relative output location; user can save anywhere.

## 6.6 G-code preview

G-code preview is a hard MVP requirement, not a polish item. The app must be fully usable for a complete print workflow with no external G-code viewer.

- **FR-GP-1.** Layer slider with first-layer / last-layer endpoints and per-layer step. Keyboard shortcuts for next/previous layer.

- **FR-GP-2.** Range mode: visualize a subset of layers (from layer A to layer B) rather than only up-to-N.

- **FR-GP-3.** Color modes: feature type (perimeter / external perimeter / infill / solid infill / top solid / bridge / support / skirt / travel), speed, flow rate, layer time, tool index (multi-extruder).

- **FR-GP-4.** Hover on a segment shows command, position, extrusion, speed, feature type, layer number.

- **FR-GP-5.** Toggle travel visibility; toggle retractions visibility.

- **FR-GP-6.** Per-layer stats panel: layer time, filament used, max speed, layer height (catches variable-layer-height surprises).

- **FR-GP-7.** Full-job stats: total time per feature type, filament per extruder, layer count, bounding box.

- **FR-GP-8.** Color-blind-safe default palette for all color modes; user-selectable alternate palette.

- **FR-GP-9.** Performance: parse and render a 50MB G-code file (typical mid-complexity print) in under 5 seconds end-to-end; layer-slider interaction at 60fps after initial render; memory under 1.5GB for the same.

- **FR-GP-10.** Preview opens for any G-code file, not only just-sliced jobs. Drag-drop a .gcode file from disk and preview it.

- **FR-GP-11.** Preview reads embedded slicer metadata (estimated time, filament use, settings comments) from G-code header and surfaces it in the stats panel.

- **FR-GP-12.** Preview opens .gcode.3mf (sliced 3MF) directly: unpacks the embedded G-code, extracts per-plate metadata (thumbnails, time estimate, filament aggregates, AMS bindings), and surfaces these in the stats panel. This is the format the A1 mini receives, and the format the platecycler compose plugin produces.

## 6.7 Printer connectivity

### Bambu Lab A1 mini

- **FR-BL-1.** LAN connection using access code + serial; no Bambu cloud dependency.

- **FR-BL-2.** Send sliced G-code as 3MF with correct Bambu metadata extensions.

- **FR-BL-3.** Read printer status: idle / printing / paused / error, current layer, nozzle and bed temperature.

- **FR-BL-4.** Read AMS lite state: per-slot loaded filament type, color, and brand/SKU where reported by the printer. Feeds the filament sync subsystem (section 6.8).

- **FR-BL-6.** Read currently-mounted build plate where the printer reports it (A1 mini reports plate type in current firmware). The reported plate is the default; user can override per-project-plate. Feeds the build_plate cascade layer (FR-CAS-9).

- **FR-BL-5.** Send commands: pause, resume, stop. Camera stream is out of MVP scope.

### Snapmaker U1

The U1 is a CoreXY toolchanger with 4 magnetically-docked toolheads on steel-ball kinematic couplings, an eddy current sensor for auto-alignment, and ~5–10 second tool swaps. Klipper-based firmware. Each toolhead is independently replaceable (Snapmaker ships hardened and alternate-size hotend bundles separately), so each slot is a distinct cascade-layer entity with its own nozzle size, hotend type, and temperature range. The slicer emits tool-change G-code at material boundaries; the printer handles docking and offsets itself.

- **FR-SU-1.** Connection over Snapmaker's HTTP API on LAN.

- **FR-SU-2.** Send G-code with toolchange sequences (T0..T3 or equivalent) correctly emitted at material boundaries by the slice profile.

- **FR-SU-3.** Read printer status: state, currently-mounted toolhead, per-toolhead loaded filament, nozzle and bed temperatures.

- **FR-SU-4.** Send commands: pause, resume, stop.

- **FR-SU-5.** Toolhead alignment offsets are managed by the printer (eddy current sensor); the app surfaces reported offsets as informational but does not modify them.

- **FR-SU-6.** Per-toolhead nozzle configuration: each of the 4 toolhead slots has its own nozzle size, hotend type (standard / hardened), and max temperature in the cascade. Defaults assume 4× identical 0.4mm steel as shipped, but the data model permits any combination.

- **FR-SU-7.** Mixed-nozzle-size prints are supported in the data model and emit valid G-code; comprehensive validation and stress testing of mixed-nozzle workflows is a post-MVP item, but the MVP must not architecturally prevent the user from configuring mixed sizes.

- **FR-SU-8.** The settings UI surfaces per-toolhead nozzle config under the printer profile, not the project; changing a slot's nozzle is a printer-level change that affects every project sliced for that printer.

- **FR-SU-9.** Read currently-mounted build plate where the U1 reports it; otherwise the user selects the active plate manually from the U1's supported plate list in the printer profile. Feeds the build_plate cascade layer (FR-CAS-9).

## 6.8 Filament sync and assignment

Filament sync makes the slicer aware of what is physically loaded in each printer, lets the user map model materials to physical slots per-printer, and surfaces mismatches before a print starts. This is the integration that current slicers — including unmodified OrcaSlicer — handle poorly for U1 and other multi-printer setups.

### Conceptual model

Model materials are abstract extruder indices (1..N) assigned to objects, paint regions, or modifier meshes. A project material binding maps each model material to a physical slot on the assigned printer. The physical slot has live state (loaded filament identity from the printer) and binds to a filament profile in the cascade. Bindings are stored per-(plate, printer), so the same project can resolve correctly when sliced for the A1 mini and for the U1 with different physical loadouts.

### Functional requirements

- **FR-FS-1.** Read printer filament state live: A1 mini AMS lite slots (filament type, color, brand if reported) over MQTT; U1 per-toolhead loaded filament (4 slots) over HTTP.

- **FR-FS-2.** Filament state UI panel per printer: shows current loadout, last-updated timestamp, manual refresh, and reconnect-on-stale.

- **FR-FS-3.** Filament profile library: ships with profiles for common Bambu and generic filaments; user can add custom profiles via the cascade.

- **FR-FS-4.** User can bind a printer slot to a filament profile manually (override the printer's reported identity) for cases where the printer lacks RFID/NFC or the user has loaded a third-party spool.

- **FR-FS-5.** Printer-reported filament identity is presented as the default; user override is clearly indicated as such with a visual badge.

- **FR-FS-6.** Project material binding UI: per-plate per-printer mapping from model material index → physical slot, with the loaded filament shown inline at each slot.

- **FR-FS-7.** Bindings are stored per-(plate, printer): a plate assigned to A1 mini has its own model→slot mapping; reassigning the plate to U1 surfaces the U1's mapping (or prompts for one).

- **FR-FS-8.** Mismatch detection runs at every cascade resolution and before slice: model wants PETG in slot 2 but slot 2 reports PLA → warning surfaced inline in the binding UI and as a slice-time blocker (configurable: warn or block).

- **FR-FS-9.** Mismatch types detected: material family mismatch (PLA vs PETG vs ABS), temperature-range mismatch outside ±10°C, color mismatch surfaced as informational (not blocking).

- **FR-FS-10.** Auto-binding heuristic: when assigning a plate to a printer for the first time, attempt to bind model materials to physical slots automatically based on filament family match. User confirms or adjusts.

- **FR-FS-11.** Sync-on-send: at send-time, the project's binding is emitted into the 3MF/G-code metadata in the format each printer expects, ensuring the printer's AMS/feeder uses the correct physical slot for each material index in the G-code.

- **FR-FS-12.** Manual re-sync action: refresh printer state on demand, with visible feedback. Useful when the user swaps a spool between slice and send.

- **FR-FS-13.** Multi-color paint UI respects the model material indices and assigns paint regions to indices, never directly to physical slots — the binding layer always mediates.

- **FR-FS-14.** Filament profile is a cascade layer: extruder/nozzle settings inherit, filament settings layer on top, user can override per-printer per-filament.

### Non-goals for MVP

- Filament inventory management (tracking how much of which spool is left across all spools the user owns).

- Automatic spool ordering or vendor integration.

- RFID/NFC writing from app to spool (read-only from printer for MVP).

- Filament sharing between printers (one logical filament 'reserved' across two machines).

## 6.9 Plugin system

- **FR-PL-1.** Lua runtime embedded via mlua, sandboxed (no io, os, package access by default).

- **FR-PL-2.** Plugin manifest (TOML) declares name, version, hooks, printer compatibility, exposed settings.

- **FR-PL-3.** Hook points: pre-slice (read/modify settings), post-slice (read/modify G-code per plate), pre-send (per-printer transforms), and compose (project-level hook that runs after all plates are sliced, with access to all plate G-codes and project metadata, producing a transformed project bundle). **Compose is deferred to post-MVP** (2026-05-30; see `docs/tickets/phase-8.md`) — the MVP ships pre-slice / post-slice / pre-send only. Its sole intended MVP consumer, platecycler, was simplified to a post-slice macro append that doesn't need it.

- **FR-PL-4.** Structured G-code API: plugins see a typed sequence of Move / Comment / LayerChange / ToolChange / Other, not raw strings.

- **FR-PL-5.** *(Deferred to post-MVP — 2026-05-30; see `docs/tickets/phase-8.md`.)* Compose hook API: plugins implementing compose receive an array of (plate, typed-gcode, metadata) inputs and return a transformed project bundle. The API includes read/write access to 3MF-level metadata (thumbnails, filament aggregates, print time totals) and to the plate composition order. Compose plugins can emit a different plate count than they received. This was the mechanism intended to support multi-plate PlateCycler batch workflows; the MVP platecycler instead appends the PlateCycler swap macro at post-slice (single plate auto-ejects on completion), so no compose hook is needed for the MVP and this API moves to v1.1.

- **FR-PL-6.** Plugin-declared settings appear in the settings UI under a Plugins category, participate in the cascade. Plate-level settings (cycle counts, composition order) are exposed by compose plugins via plate metadata, not via the global settings cascade.

- **FR-PL-7.** Plugins can read live filament state for the active printer (per-slot identity, loaded flag) via a read-only API, enabling printer- and material-aware plugin behavior.

- **FR-PL-8.** *(Deferred to post-MVP.)* Hot reload: changes to plugin files in the plugins folder are detected and applied without restart via a folder watcher. For the MVP, plugins load on launch and can be reloaded manually (`plugin_reload`); the automatic file watcher is post-MVP.

- **FR-PL-9.** Plugin errors are caught, logged, and surfaced in a Plugins panel without crashing the host.

# 7. Non-functional requirements

| **Area** | **Requirement** |
| --- | --- |
| Performance — slice | Slice time within 10% of OrcaSlicer for the same model and settings (same engine, overhead bounded). |
| Performance — UI | Settings panel re-render under 50ms on cascade changes. Viewport 30fps minimum at 20M-tri scenes on integrated GPU. |
| Memory | Idle footprint under 400MB. Loaded 100MB STL under 2GB total. |
| Startup | Cold start to usable UI under 3 seconds on SSD-equipped mid-range hardware. |
| Platforms | Linux as flatpak (tested on recent Ubuntu, Fedora, Arch with flatpak runtime). WSL2 tested as best-effort. Windows-native and macOS-native are post-MVP. |
| Installer size | Under 200MB flatpak (libslic3r + Tauri webview + assets). |
| Crash recovery | Project state autosaved every 30 seconds; recoverable on next launch. |
| Logging | Structured logs to user log directory; verbosity configurable; never include filament costs or printer access codes. |
| Licensing | AGPL-3.0-or-later. No telemetry, no analytics, no network calls except to user-configured printers. |
| Accessibility | Keyboard navigation for all settings. Color-blind-safe palette for G-code preview color modes. Full accessibility audit deferred to v1.0. |

# 8. Technical architecture

## 8.1 Stack

- **Shell.** Tauri 2.x. Rust core, web frontend via system webview.

- **Frontend.** TypeScript + React + Tailwind. Three.js for the 3D viewport (with a wgpu native viewport as a fallback plan if webview perf is insufficient).

- **Slicing engine.** orca-slicer-ffi (this project's existing FFI) wrapping libslic3r. Linked as a Rust crate in the Tauri core.

- **Plugin runtime.** mlua (Lua 5.4) embedded in the Rust core.

- **Storage.** TOML for profile layers, 3MF for project files, JSON for app state.

- **Printer comms.** rumqttc for Bambu MQTT; reqwest + tokio-tungstenite (both on native-tls, sharing the Bambu stack) for the U1's Moonraker HTTP/WS endpoints.

## 8.2 Module boundaries

- **core/cascade.** Rule cascade resolver. Pure Rust, no I/O, no UI. Loads TOML rule files, validates them against the schema and option scopes, accepts a context object, returns resolved settings with trace metadata (rule source, specificity, file:line). The implementation of FR-CAS-2 through FR-CAS-13. Fully testable in isolation. See the profiles strategy document for the detailed design.

- **core/cascade-adapter.** Translation layer between our resolved logical settings and libslic3r's DynamicPrintConfig. Owns the translation manifest (FR-CAS-15) listing which keys are dimensional and how to expand them. Handles libslic3r's dispatch quirks (curr_bed_type, wipe_tower, filament_map normalization, etc.) so the rest of the system never sees them. The boundary above which all code works in our logical option vocabulary; the boundary below which all code uses libslic3r's names.

- **core/project.** Project model, plate/printer binding, plate metadata (cycle counts), material bindings, persistence.

- **core/scene.** Renderer-agnostic 3D scene state per AD-8 / FR-3D-7. Owns mesh registry, per-object transforms and metadata, selection, exclusion-zone data. Exposes Tauri commands for mutations and emits typed events for view sync. The frontend renderer (Three.js for MVP) consumes events; it does not hold authoritative state. (Gizmo and camera state were dormant view-state and have been removed from the scene model — see §9.2; re-add them here when a pivot-setting UI or persisted-view feature is actually built.)

- **core/slice.** FFI wrapper, slice orchestration, progress events.

- **core/gcode.** Typed G-code model, parser, serializer. Shared by preview and plugins.

- **core/threemf.** 3MF reader and writer utility. Used by parts of the system that need it: project save/load (our own .3mf format extends standard 3MF), project import from other slicers (Bambu Studio, OrcaSlicer, Snapmaker Orca all save .3mf), preview drag-drop of sliced files (.gcode.3mf), and the A1 mini driver (which wraps slice output as .gcode.3mf for Bambu's required send format). The U1 driver does not depend on this module — it sends raw G-code. Future drivers depend on this module only if their printer requires 3MF input.

- **core/filament.** Filament profile library, printer-state-to-profile resolution, mismatch detection, sync-on-send metadata emission.

- **core/plugin.** Lua host, manifest loader, hook dispatch (pre-slice / post-slice / pre-send / compose), sandboxing. Read-only views into project, gcode, and filament state for plugins.

- **core/printer/bambu.** Bambu MQTT protocol.

- **core/printer/snapmaker.** Snapmaker U1 printer-profile adapter (cascade layer). Driver-side comms live under `core/driver/snapmaker/` (Moonraker HTTP+WS).

- **ui/.** React app. Communicates with core via Tauri commands and events only.

*A note on send-format responsibility: each printer driver owns its send path end-to-end, including any wrapping, transformation, or metadata injection the target printer requires. The slicer produces canonical G-code via libslic3r; the driver decides what to do with it before transmission. This means adding a future printer is a self-contained exercise — write the driver, declare its capabilities, no shared-code changes required.*

## 8.3 FFI extensions needed

The current orca-slicer-ffi will need additions to support the MVP. These are owned by the project (FFI author = project lead), so risk is execution time, not external dependency.

### Completed since plan inception

- coEnums default surfacing — done in commit `1bb3503`. All 9 affected option keys now expose a reverse-looked-up default via `def.enum_keys_map`. See `docs/libslic3r-workarounds.md` §5.

- Option scope bitmask — done in commit `58e199e`. `slic3r_option_def_t::scope` carries `SLIC3R_SCOPE_OBJECT | REGION | PRINT | SLA_*` bits derived from the static config classes; backs FR-CAS-13 and the adapter's dispatch logic.

### Still open

- Logging sink redirect (replaces stderr-only boost::log default).

- Slice progress callback (in addition to the slice call itself).

- G-code emission to memory buffer rather than only to file path, if not already supported.

- Windows symbol-export annotations: post-MVP (not blocking the Linux flatpak build).

- macOS CMake adjustments: post-MVP.

# 9. Architecture decisions and printer capability matrix

This section documents decisions about how the architecture handles capability differences between supported printers, and names known gaps explicitly. The goal is that an engineer reading this document knows what was decided, what was deferred, and what was accepted as a known limitation.

## 9.1 Printer capability matrix

The capabilities below are modeled per-printer in the printer profile. The cascade, UI, and G-code emission read this profile to adapt behavior. Adding a future printer is a matter of declaring its capabilities — no core changes required.

| **Capability** | **A1 mini** | **U1** |
| --- | --- | --- |
| Material slots | 4 (AMS lite) | 4 (toolheads) |
| Material switch mechanism | Filament swap at single hotend | Toolhead change |
| Purging required (contamination flush) | Yes (single hotend reuse) | No (per-toolhead retained material) |
| Priming tower used | Yes (large, doubles as purge structure) | Yes (small, per-toolhead re-entry stabilization) |
| Purge volumes matrix relevant | Yes (drives prime tower bulk) | No |
| Per-slot nozzle independence | No (1 hotend) | Yes (4 independent toolheads) |
| Per-slot hotend type configurable | Single hotend, swappable as a unit | Per-toolhead |
| Toolhead offsets | N/A | Managed by printer (eddy current); displayed read-only |
| Build volume | 180×180×180mm | 270×270×270mm |
| Exclusion zones | AMS-adjacent area near bed front | Toolhead parking bay (rear) |
| Plate cycling via compose plugin | Yes (PlateCycler add-on; ships with MVP via platecycler plugin) | Not applicable (no hardware plate eject mechanism) |
| Multi-plate per project typical | Yes (especially with PlateCycler) | Less common (1 plate, multi-material) |
| Connectivity | MQTT LAN (access code + serial) | HTTP LAN (Snapmaker API) |
| Send format | .gcode.3mf (sliced 3MF with Bambu metadata extensions) | Plain .gcode (Klipper-style) |
| Build plates supported | Cool Plate, Textured PEI, Smooth PEI, Engineering Plate, SuperTack (Bambu's plate range for A1 mini) | U1 ship-standard plate set (to be enumerated from Snapmaker Orca profile) |
| Build plate live reporting | Yes (current firmware) | Verify per firmware; user-selectable fallback |
| Live filament identity reporting | AMS lite contents over MQTT | Per-toolhead loaded filament over HTTP |
| Camera (MVP) | Out of scope | Out of scope |
| Pause / resume / stop | Yes | Yes |

## 9.2 Architecture decisions

### AD-1: Printer-aware setting visibility

Decision: the settings UI hides options that are not meaningful for the active printer's capabilities. The visibility filter combines libslic3r's option metadata with per-printer capability flags.

- **Resolves:** purge volumes matrix hidden on toolchanger printers (U1) where no purging happens, but priming tower geometry settings stay visible because both A1 mini and U1 use a priming structure (the A1 mini's large, the U1's small for toolhead re-entry stabilization). Toolchange G-code settings hidden on single-extruder-with-feeder printers (A1 mini). Single_extruder_multi_material toggles surfaced or hidden per capability.

- **Implementation:** each printer profile declares a capability struct (purging_required, priming_tower_used, slot_count, per_slot_nozzle, toolhead_offsets_editable, mechanism, etc.). The settings UI applies a visibility filter per option group based on capabilities. Purging-related options (purge_volumes_matrix, purge volume defaults) only show when purging_required is true; priming-tower geometry options show whenever priming_tower_used is true; the two are independent. Setting search still finds hidden options, with a 'not applicable to this printer' badge.

### AD-2: Single- vs multi-slot settings UI mode

Decision: the settings panel layout adapts to slot count. Printers with one material slot show a single extruder/filament pane; printers with two or more slots show a per-slot tab strip with a 'all slots' synchronized-edit option.

- **Resolves:** A1 mini shows single nozzle/filament pane (the AMS slots are filament-binding, not separate nozzles). U1 shows 4 tabs, one per toolhead, each independently configurable. Future printers slot in based on their declared slot_count.

- **Implementation:** the per-slot tab strip is a UI primitive that takes a slot_count and a 'synchronized edit' affordance. The default for U1 is synchronized edit ON (most users will configure all 4 identical), with a clear toggle to break sync per-slot.

### AD-3: Unified printer status data model

Decision: a single PrinterStatus struct with required core fields and optional capability-gated fields. Printer drivers populate the fields their capabilities support; the UI components render fields that are populated and ignore those that are not.

- **Required core:** connection state, printer state (idle/printing/paused/error), bed temperature current+target, timestamp.

- **Optional:** current_layer, total_layers, nozzle_temp per slot, mounted_toolhead, loaded_filament per slot, ams_contents, time_remaining, current_z.

- **Driver-specific extension blob:** an opaque per-driver JSON for fields that don't fit the unified model (e.g. Bambu-specific cloud status flags). Surfaced in an advanced/debug view only.

### AD-4: Slot availability and missing-slot handling

Decision: each slot has an availability state (available / unavailable / unknown). Unavailable slots cannot be bound to model materials; bindings to a slot that becomes unavailable surface as warnings, with a 'rebind to available slot' affordance.

- **Resolves:** U1 toolhead removal, A1 mini AMS lite spool removed during binding, future toolchanger with parked/disabled tools.

- **Implementation:** see FR-MP-8 — bindings validated at cascade resolution and pre-slice, slice blocked with rebind suggestion on unavailable slot.

### AD-5: Tool offset display and future editability

Decision: tool offsets are read-only in the MVP. The UI displays offsets reported by the printer with a per-slot status indicator (aligned / out-of-tolerance / unknown). The data model permits offset writes; the write path is not wired in MVP and is gated behind a per-printer capability flag (offsets_editable: bool).

- **Resolves:** U1 self-alignment is the source of truth; future printers (Prusa XL, Voron toolchangers) where users edit offsets manually are architecturally supported without rework.

### AD-6: Plate-count guidance (soft UX, not constraint)

Decision: plate count is not constrained per-printer. A1 mini and U1 can both have 1–4 plates. The UX may offer soft hints post-MVP (e.g. 'U1 typically prints multi-material on a single plate; consider consolidating') but the MVP does not gate behavior.

- **Resolves:** lets PlateCycler workflows shine for A1 mini owners while not artificially limiting U1 users who want multi-plate for batch jobs.

### AD-7: Klipper-based U1 — Snapmaker-targeted Moonraker driver, not general-purpose

Decision (acknowledged limitation): the U1 driver speaks vanilla Moonraker over plain HTTP+WS on port 80 — that's what the U1 firmware exposes — but treats the U1's Snapmaker-specific status objects (e.g. `print_task_config.{filament_color_rgba, filament_type}` for per-toolhead filament identity) as load-bearing. A future generic Klipper/Moonraker driver targeting non-Snapmaker hardware is a separate driver, not a generalization of the U1 driver. The Snapmaker-specific pair / mTLS / MQTT control plane (used for the webcam in their ecosystem) is out of MVP scope.

- **Resolves:** U1 firmware updates that change Snapmaker's vendor objects are tracked; upstream Klipper / Moonraker changes affect us only via the standard endpoints (`printer.objects.subscribe`, `/server/files/upload`, `/printer/print/*`) which are stable. A future Voron/RatRig user gets a Moonraker driver, not a 'U1-compatible' driver.

- **Note:** the prior version of this decision claimed U1 used a "Snapmaker HTTP wrapper, not Moonraker." That was incorrect — the U1 exposes standard Moonraker, just with extra Snapmaker-specific status objects layered in. Corrected during PR-7b-6 (PRD §11.3 living-documents).

### AD-8: 3D scene state lives in Rust, not in the renderer

Decision: the authoritative 3D scene model (objects with mesh handles, transforms, hierarchy, selection state, gizmo state, camera state, exclusion-zone data) lives in Rust as a renderer-agnostic data structure. The frontend renderer (Three.js for the MVP) is a read-only view that reflects state into pixels. State mutations flow renderer → Tauri command → Rust state update → Tauri event → renderer re-render. The renderer never holds authoritative state and never mutates state directly.

- **Why now, not when we hit the wall.** Phase 2 carries an explicit risk (PRD §10) that webview 3D performance is insufficient for our target scene sizes, with the documented mitigation "switch to wgpu in a native Tauri window." That mitigation is only cheap if the renderer is a swappable view. If Three.js owns scene state, switching it out means rewriting state management at the same time — a much harder cut. The separation cost is small upfront (a clean API boundary) and dwarfs the cost of unwinding it later.

- **What the state model contains:**
  - Scene graph: per-plate object list with parent/child relationships (modifier meshes, paint volumes nested under their parent object).
  - Per-object: mesh handle (id into a mesh registry; the actual triangle data is owned by Rust), transform (translation/rotation/scale matrices), filament binding, per-object setting overrides, visibility flag, lock flag.
  - Selection: which objects are selected on the active plate.
  - Gizmo: active gizmo kind (move/rotate/scale/mirror/none), target object(s), local-vs-world space, snap settings.
  - Camera: position, look-at, projection mode (perspective/ortho), zoom level.
  - Bed visualization data: per-printer bed bounds, exclusion zones, origin marker location — pulled from the active plate's printer profile.
  - Mesh registry: Rust-side store of triangle data, indexed by mesh handle. The renderer requests mesh bytes for a handle when it needs to upload to GPU; the renderer's GPU buffers are a derived view of the registry, not the source of truth.

- **What flows over the IPC boundary:**
  - Renderer → backend (Tauri commands): user intent — `scene_select(ids)`, `object_translate(id, delta)`, `gizmo_set(kind, ids)`, `camera_orbit(yaw, pitch)`, etc. Commands return updated state.
  - Backend → renderer (Tauri events): authoritative state changes — `scene_changed(diff)`, `selection_changed(ids)`, `mesh_uploaded(handle, bytes)`. The renderer applies the diff to its local mirror.
  - Mesh data crosses once per upload (Rust → renderer); transforms and selections cross every frame at human interaction rates (~60 Hz worst case for orbit, much less for object drag).

- **Where this lives in the module layout:** a new `core/scene` module in PRD §8.2's `core/` tree, distinct from `core/project` (which owns plate/printer binding, persistence) and from the renderer (which is `ui/`'s concern). `core/scene` exposes typed Tauri commands and emits typed events; `ui/` consumes them via a thin Rust→TS type-share layer (Tauri's `specta`/`tauri-specta` is a reasonable choice).

- **Performance contract:** the Rust state model must support ≥1000 objects in a scene without state operations exceeding 5ms p99 (selection, transform application, scene-diff computation). The renderer's frame budget (FR-3D-5: 30fps on 20M-tri scene) is a *renderer* concern; state-side budget is separate.

- **What this is *not*:** it is not a ban on the renderer caching derived data. Three.js's scene graph, GPU buffers, BVH for picking — all fine as renderer-internal caches keyed off the authoritative state. The rule is about *ownership of the truth*, not about avoiding caches. The line is *observable, persisted scene truth* vs. *ephemeral view UI*. Concretely: the gizmo's active transform *mode* (translate/rotate/scale) is renderer-local — it never affects geometry, slice output, or the saved project, so it lives in the viewport (`App`), not `core/scene`. The gizmo *pivot* override was removed from the scene model for now (no pivot-setting UI shipped); re-add a `core/scene` pivot field + setter command when one does. The `rotate_object` mutation still takes an optional explicit-pivot argument as a transform primitive. **Camera state** was likewise removed from `core/scene`: the renderer owns its Three.js camera and frames from the bed, and never synced or restored a persisted camera. To ship "restore per-plate view on reopen," re-add a camera field + a `scene_camera_set` the renderer commits on orbit-end and reads back on load.

- **Out-of-scope for MVP but architecturally enabled:** scriptable scene operations (Lua plugins inspecting/mutating scene state via the same command surface the renderer uses), headless rendering for thumbnails, alternate renderers (a side-by-side wgpu viewport, an SVG top-down view for plate previews) — all become tractable when state is renderer-agnostic.

## 9.3 Known gaps accepted for MVP

- **Mid-edit printer state changes.** If a user changes loaded filament on the printer mid-edit, the app polls and updates on the next cycle. There is no 'attention!' animation drawing the eye to the change. Users notice via the filament panel. Post-MVP enhancement.

- **Time-estimate accuracy.** Per-printer print-time estimates rely on libslic3r's estimator with per-printer profile tuning. We do not validate estimator accuracy against real prints in the MVP beyond order-of-magnitude sanity.

- **Camera integration.** Both printers have cameras. Out of MVP scope; data model and driver trait do not preclude adding camera streams post-MVP.

- **Filament inventory across printers.** If a user has the same spool 'loaded' in two printers' UI (e.g. moved between them and the app didn't see the unload), there is no detection. Post-MVP.

- **Mixed-nozzle-size U1 prints.** Architecturally supported (per AD-2 and FR-SU-7) but not validated with real prints in MVP.

## 9.4 Validation: what these abstractions handle that we are not building yet

These are listed to confirm the architecture generalizes, not as commitments to support. No code is written for them in the MVP.

- Prusa XL (5-toolhead toolchanger): same model as U1 with slot_count=5 and user-editable offsets.

- Voron with ERCF (single hotend + N-slot external feeder): same model as A1 mini with AMS lite, with slot_count = ERCF channels.

- Bambu X1C / P1S with AMS (4 or 16 slots via AMS HUB): same model as A1 mini, larger build volume, more slots.

- Single-extruder simple printer (Ender 3, Mk3): collapses to slot_count=1, no purging (single material), priming structure optional, no toolchange. Connectivity via Moonraker if Klipper, OctoPrint otherwise, or USB-only.

- Mixed-purpose machines (Snapmaker original 3-in-1, future tool-changing CNC/laser hybrid): out of architectural scope; this is a 3D printing slicer, not a fabrication suite.

# 10. Risks and mitigations

| **Risk** | **Likelihood** | **Impact** | **Mitigation** |
| --- | --- | --- | --- |
| Webview 3D performance ceiling exceeded by large meshes | Medium | High | Prototype viewport in week 2 with stress test models. If insufficient, switch to wgpu in native Tauri window. Cost is bounded because scene state lives in Rust independent of the renderer (AD-8 / FR-3D-7); only the rendering layer changes. Estimate 2–3 weeks for the swap given the separation, down from 4–6 weeks if state and renderer were entangled. |
| Snapmaker U1 toolchange G-code edge cases | High | Medium | Build U1 profile from Snapmaker Orca's published profile as starting point. libslic3r already supports toolchanger-style multi-material G-code (Prusa XL pattern). Print test models early. |
| Bambu MQTT protocol changes | Low | Medium | Use community libraries as reference, pin protocol version, document fallbacks. |
| OrcaSlicer submodule churn breaks FFI on bump | Medium | Low | Pin submodule, bump deliberately, not on every upstream commit. |
| Cascade UX requires multiple redesigns | High | Medium | Plan for it: budget 2 design iterations in months 2–3. Test with 5 real users before MVP freeze. |
| AGPL deters commercial plugin authors | Medium | Low | Document plugin licensing clearly. Plugins running in the Lua sandbox arguably are not derivative works of libslic3r. |
| Solo developer burnout / scope creep | Medium | High | Strict MVP freeze date. Non-goals list is enforced. Weekly retrospective with explicit cut list. |
| Hardware testing access (need both printers continuously available) | Low | High | Confirmed: both printers owned by project lead. |

# 11. Working practices for the build phase

This project will be built with heavy use of Claude Code. The following practices are not nice-to-haves — they are the explicit working contract between the project lead and Claude Code, derived from lessons during the planning phase of this project.

## 11.1 Verify hardware and protocol claims before coding

LLMs (including Claude) reason confidently from outdated or wrong priors about specific hardware, third-party protocols, and library internals. The planning conversation that produced this document contained multiple factual errors that the project lead caught — printer architecture, slicer ecosystem, terminology. Those errors are cheap to correct in planning; the equivalent errors in code waste days.

- **Rule:** any code generated that depends on a specific fact about hardware behavior (A1 mini AMS protocol, U1 toolchange macros, Bambu 3MF format), third-party library internals (libslic3r option semantics, mlua API), or printer firmware quirks must cite a verifiable source — published documentation, a referenced community library, or a test that confirms the behavior. 'Claude said so' is not a source.

- **Rule:** when Claude Code suggests an implementation that hinges on an assumption, the assumption is named in a comment or commit message. Code reviews check assumptions, not just logic.

- **Rule:** if an assumption is wrong and gets corrected, the correction goes into the persistent project context (CLAUDE.md or equivalent), not just the conversation. This prevents the same wrong belief from resurfacing in a future session.

## 11.2 Spike before committing

Phase 0 includes a set of engine-validation spikes (see Plan section 'Phase 0.5'). The pattern continues throughout the project: when a phase depends on an assumption about how libslic3r, a printer, or a library actually behaves, that assumption is validated with a small test program before the phase's main work begins. One day of spike work routinely prevents one week of debugging.

## 11.3 Living documents

This PRD and the Execution Plan are not commitments frozen at kickoff. They are living documents. When a phase surfaces information that contradicts an architectural decision, capability matrix entry, or FR, the document is updated and the change is committed alongside the code change that motivated it. An outdated PRD is worse than no PRD because it gives false confidence — Claude Code reading a stale doc will reason from stale facts.

## 11.4 Correction posture

During the build phase, the project lead is expected to correct Claude Code's reasoning at the detail level without expecting pushback. Claude Code is expected to incorporate corrections promptly, update persistent context where appropriate, and not relitigate decisions once made unless new information warrants it. The collaboration model is 'expert + capable assistant', not 'pair of equals' — the project lead has hardware-in-hand and domain ownership that Claude Code does not have.

## 11.5 Project context to maintain

A CLAUDE.md at the project root captures the durable context Claude Code needs each session. Suggested contents:

- Pointer to this PRD and the Execution Plan as the source of truth for requirements and phasing.

- Pointer to the profiles strategy document (docs/profiles.md or similar) as the source of truth for the rule cascade design, TOML schema, resolution semantics, and translation adapter. The PRD §6.1 names the requirements; the strategy doc owns the design.

- Printer facts that have already been corrected once and should not need re-correcting: U1 is a 4-toolhead toolchanger (not IDEX), A1 mini uses AMS lite filament-swap (not toolchange), purging vs priming are independent capabilities, etc.

- Snapmaker Orca is a fork of OrcaSlicer, not Cura. Snapmaker U1 ships supported by Snapmaker Orca; unmodified OrcaSlicer also works.

- orca-slicer-ffi is owned in-house (this project's author wrote it). FFI extensions are first-party work, not external dependencies.

- platecycler is owned in-house. The MVP ships it as a Lua plugin using the compose hook.

- Licensing: AGPL-3.0-or-later. Plugin licensing model is documented separately.

- No telemetry, no analytics, no network calls except to user-configured printers. This is a product principle, not a default to be overridden.

# 12. Open questions

- Project file format: RESOLVED — extend .3mf in our own metadata namespace. Decision driver: existing Bambu/Orca/Snapmaker users need a frictionless migration path, and Bambu's printer-send format requires 3MF anyway. See FR-MP-4.

- Configuration format: RESOLVED — rule cascade in TOML, with [[rule]] full form and section shorthand. Designed in the profiles strategy document. See FR-CAS-1 through FR-CAS-17.

- Cascade resolution semantics: RESOLVED — two-phase model. Authored cascade (default → printer → build_plate → filament) uses specificity-then-source-order. User and project files apply as CSS-!important-style absolute override tiers on top: user is one tier, project is a higher tier, both win unconditionally over the authored cascade. Project overrides user. See FR-CAS-3.

- 3MF preset import: RESOLVED — import as a single flat overlay rule (no when.* predicates). See FR-CAS-17.

- Cascade open items deferred to post-MVP (from profiles strategy doc): cross-dimension specificity outranking (e.g. should plate-type beat filament-type?), negative conditions (when.filament.type != 'ABS'), numeric/range predicates (when.nozzle.diameter >= 0.6), libslic3r option-rename migration tooling.

- Distribution model: GitHub releases plus self-hosted flatpak repo for MVP. Flathub submission deferred to post-MVP per Phase 9.

- Naming: working title 'Multi-Printer Slicer' is not the final name.

- Telemetry: zero telemetry is a strong default, but crash reporting opt-in might be valuable. Decide before beta.

- Profile sharing: importing Orca profiles is in scope, but is exporting in-scope? Cheap to add, useful for hedging the user-lock-in concern.