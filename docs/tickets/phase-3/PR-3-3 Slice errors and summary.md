# PR-3-3 — Slice errors with offending-setting attribution + post-slice summary

Status: ❌ open.

**Scope.** Two related concerns: (a) when libslic3r fails mid-slice,
the user sees a typed error that names the offending setting where
possible; (b) when a plate slice succeeds, the user sees a summary
(time estimate, filament usage, layer count) extracted from the
post-slice state.

Owns PRD FR-SL-3 (error surfacing) and FR-SL-4 (post-slice summary).

**Acceptance criteria.**

- `core/slice/errors.rs`:
  - `pub enum SliceError` with typed variants for the common
    libslic3r failure modes:
    - `InvalidConfig { setting_key, reason, raw_message }`
    - `InvalidGeometry { reason, raw_message }`
    - `OutOfBounds { plate_id, raw_message }`
    - `Cancelled`
    - `Unknown { raw_message }`
  - `fn classify_libslic3r_error(raw: &str) -> SliceError` — a
    table-driven pattern matcher. Includes setting-name extraction
    via regex match against libslic3r's "Option `xxx` is invalid"
    / "Setting `yyy` failed validation" prefixes (catalog them
    from `external/OrcaSlicer/src/libslic3r/PrintConfig.cpp` while
    writing this).

- `core/slice/summary.rs`:
  - `pub struct PlateSummary {`
    - `pub estimated_time_seconds: u64,`
    - `pub filament_used_grams: HashMap<u8 /* extruder */, f64>,`
    - `pub filament_used_meters: HashMap<u8, f64>,`
    - `pub filament_used_cost_cents: HashMap<u8, u64>, // cut candidate per execution plan`
    - `pub layer_count: u32,`
    - `pub object_count: u32,`
    - `pub gcode_lines: u64,`
    - `pub bbox_min: [f32; 3],`
    - `pub bbox_max: [f32; 3],`
    - `pub output_path: PathBuf,`
    - `}`
  - `fn build_summary(gcode_path: &Path) -> Result<PlateSummary,
    io::Error>` — reads the G-code header comments libslic3r emits
    (`; estimated printing time = `, `; filament used [g] = `, etc).
    Parser is *not* PR-3-8's generic header-metadata parser — that
    one handles foreign slicers; this one targets libslic3r's
    specific format and is a fast 100-line scan. PR-3-8 may later
    consume the same regex catalog; document the overlap.

- Wire into PR-3-2:
  - On FFI slice failure, run `classify_libslic3r_error` over the
    error string before emitting `slice:job_failed`. The event
    payload carries the typed `SliceError`.
  - On FFI slice success, call `build_summary(output_path)` and
    include the `PlateSummary` in `slice:plate_finished`.

- Tests:
  - Error classification: feed known libslic3r error strings (from
    spike fixtures + a small catalog of representative messages,
    documented inline) through `classify_libslic3r_error` and
    assert the right typed variant + extracted setting key.
  - Summary parser: feed `examples/spike1/`'s sliced G-code header
    through `build_summary`, assert the time + filament numbers
    match the same fixture's known values.

**Effort.** ~2 days. Error classification is the harder half — needs
a representative catalog of libslic3r error messages. The summary
parser is straightforward.

**Dependencies.** PR-3-2 (orchestrator wires both into events).

**Out of scope.** Per-extruder cost calculation beyond filament
density × volume (price-per-spool / brand DB is Phase 5+). Surfacing
hint actions (e.g. "auto-fix: enable supports") — Phase 4 UI work.

**Cut candidate.** `filament_used_cost_cents` (~1 day savings).
Drop the field; summary still useful without cost.
