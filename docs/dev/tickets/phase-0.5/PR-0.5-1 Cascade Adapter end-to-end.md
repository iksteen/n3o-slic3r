# PR-0.5-1 — Cascade adapter end-to-end

Status: ✅ done. Finding doc: `docs/dev/spikes/spike-1-cascade-adapter.md`.

**Scope.** The walking-skeleton of the Phase 1 architecture: a real
OrcaSlicer device profile, converted into our cascade format, fed
through a stub resolver and stub adapter, dispatched to libslic3r,
producing valid gcode. Throwaway code; the goal is to find FFI gaps
and dispatch-quirk surprises early, not to build the production
resolver.

The seed config **must** be a converted OrcaSlicer device profile —
not a hand-rolled minimum config. PR-0-5 confirmed that
`Print::validate()` rejects FullPrintConfig defaults before slicing
starts; the spike's value comes from exercising the full round-trip
against config shapes we'll actually see in Phase 1+.

**Acceptance criteria.**

- `external/OrcaSlicer/resources/profiles/BBL/machine/Bambu Lab A1
  mini 0.4 nozzle.json` is converted into a TOML rule cascade. The
  conversion script (Rust or Python, doesn't matter) lives at
  `scripts/spikes/convert_orca_profile.<ext>` and is committed.
  Output cascade lands at
  `examples/cascades/bambu-a1-mini-spike1.toml`.
- The cascade is composed of a default rule + at least one filament
  rule + at least one plate-type rule (per `docs/dev/profiles.md`
  "Worked example"). Specificity-based resolution is exercised by
  having two rules match the test context with different
  specificities.
- A stub resolver in `src-tauri/src/core/cascade/spike1.rs` (or a
  standalone `examples/` binary) reads the cascade, resolves it
  against a context object, and produces a flat `BTreeMap<String,
  String>` of resolved (key, serialized value) pairs. No
  `!important` tier handling needed for this spike — just authored
  cascade with specificity.
- A stub adapter consumes the flat map, applies the dimensional
  expansion documented in `docs/dev/profiles.md` (bed temp at minimum),
  and emits a `slic3r_ffi::Config` ready for `Print::apply`.
- `slic3r_ffi::slice()` produces a non-empty G-code file at
  `/tmp/spike1.gcode` against the
  `external/OrcaSlicer/resources/handy_models/OrcaCube_v2.3mf`
  model (or another known-good 3MF — record which).
- The finding doc at `docs/dev/spikes/spike-1-cascade-adapter.md`
  documents:
  - the cascade vocabulary actually used (which Orca keys mapped
    1:1, which needed dispatch normalization, which were unused);
  - any FFI surface gaps encountered (missing `Config::set` accepts,
    enum serialization quirks beyond the coEnums ones, etc.);
  - any libslic3r dispatch quirks discovered beyond those already
    in `docs/dev/libslic3r-workarounds.md` (and updates to that doc if
    new ones turn up).

**Effort.** 1–2 days. The Orca-profile conversion is the unknown;
the resolver and adapter stubs are mechanical.

**Dependencies.** Phase 0 complete (FFI link, scene state, core/
modules in place).

**Out of scope.** Production resolver (Phase 1). Two-phase
`!important` resolution (Phase 1). Translation manifest as a TOML
file (Phase 1; the spike's manifest can be hardcoded Rust).
Multiple device profiles (this spike does one device end-to-end;
mixed-nozzle is PR-0.5-2, multi-color AMS is PR-0.5-3). Any UI
work. Any unit tests beyond "did it slice."
