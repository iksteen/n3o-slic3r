# PR-3-8 — Header metadata parser

Status: ❌ open. **Cut candidate** per Execution Plan §5.

**Scope.** Multi-slicer-flavor header parser: extracts estimated
time, filament use, slicer of origin, layer count, etc. from the
comment block at the top of a G-code file. Handles Orca / Bambu
Studio / PrusaSlicer / Cura dialects.

Owns part of FR-GP-11 (preview reads embedded slicer metadata).

PR-3-3 (`build_summary`) already ships a libslic3r-specific scan
for the slice-output summary. This ticket generalizes that to
foreign G-code (Cura, PrusaSlicer, etc.) so the Phase 6 preview can
ingest external files with the same data model.

**Acceptance criteria.**

- `core/gcode/header.rs`:
  - `pub struct HeaderMetadata {`
    - `pub slicer: Option<SlicerOrigin>, // Orca | BambuStudio | PrusaSlicer | Cura | Unknown(String)`
    - `pub slicer_version: Option<String>,`
    - `pub estimated_time: Option<Duration>,`
    - `pub filament_used: Vec<(u8 /* extruder */, FilamentUsage)>,`
    - `pub layer_count: Option<u32>,`
    - `pub object_count: Option<u32>,`
    - `pub bbox_min: Option<[f32; 3]>,`
    - `pub bbox_max: Option<[f32; 3]>,`
    - `pub raw_settings: HashMap<String, String>, // raw "; key = value" pairs not otherwise typed`
    - `}`
  - `pub struct FilamentUsage { pub grams: Option<f64>, pub
    meters: Option<f64>, pub volume_mm3: Option<f64> }`

- `pub fn parse_header<R: BufRead>(input: R) -> HeaderMetadata` —
  reads up to N (configurable, default 4096) lines or until the
  first non-comment line, whichever comes first. Returns a
  best-effort `HeaderMetadata` — never errors, since foreign
  G-code may have any subset of fields.

- Dispatch table: a `Vec<(Regex, fn(&Captures, &mut
  HeaderMetadata))>` of recognized patterns. Each entry documented
  with the slicer it came from + the spike fixture where we
  verified it.

- Reuses PR-3-3's libslic3r-specific catalog as one entry in the
  table; document the overlap so changes propagate.

- Tests:
  - Each supported slicer: one canonical fixture under
    `examples/gcode-fixtures/<slicer>.gcode` (a 50-line header
    snippet is enough — we don't need full real files). Assert the
    expected typed fields populate.
  - Mixed metadata: a file with Orca + Cura tokens (e.g., copied
    from a Bambu print preview that round-tripped through Cura)
    parses without crashing.
  - Empty / no-header file: parser returns
    `HeaderMetadata::default()` with no panics.

**Effort.** ~1 day. Pattern catalog + fixtures is the bulk.

**Dependencies.** PR-3-6 (recognizes some of the same comment
prefixes; either share a regex catalog or document the duplication).

**Cut decision.** Execution Plan §5 lists this as a cut candidate
(defer to Phase 6, where the preview needs it anyway). Recommend
**keeping it in Phase 3**: PR-3-3's libslic3r scan is half this
ticket already; spending the extra day here unblocks Phase 6's
drag-drop external G-code flow without revisiting the parser later.
If Phase 3 runs tight, drop the Cura + Prusa flavors and leave only
Orca / Bambu Studio — those are the slicers we author against.

**Out of scope.** Validating that the header matches the body (e.g.,
that `; layer count = 247` agrees with the parsed body's 247 layer
changes). Phase 6 may add a consistency check; not Phase 3.
