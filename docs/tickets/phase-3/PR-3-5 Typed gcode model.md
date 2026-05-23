# PR-3-5 — Typed G-code model

Status: ❌ open.

**Scope.** The `Line` enum that PR-3-6's parser produces and PR-3-7's
serializer consumes — the typed sequence shared by the G-code preview
(Phase 6) and the plugin host (Phase 8). Per FR-PL-4: plugins see
typed values, never raw strings.

Lives in `core/gcode/model.rs` (stub already exists; this ticket
fills it).

**Acceptance criteria.**

- `pub enum Line` with the four content variants from Execution Plan
  §5 + an `Other` escape hatch:
  - `Move(Move)` — `G0` / `G1` / `G2` / `G3` extrusion or travel.
  - `Comment(Comment)` — `;…` lines, with a sub-variant for the
    structured comments libslic3r emits (`;TYPE:`, `;LAYER:`,
    `;Z:`, etc.).
  - `LayerChange(LayerChange)` — synthetic, emitted when the
    parser detects a layer boundary (Z-change paired with the
    Orca-style `;LAYER:` marker, with a fallback heuristic when
    only Z-change is observed).
  - `ToolChange(ToolChange)` — `T0`/`T1`/.../`Tn`, with the
    target extruder index extracted.
  - `Other(Other)` — preserved verbatim raw string + the
    original byte offset so PR-3-7's serializer can re-emit
    untouched.

- `pub struct Move {`
  - `pub command: MoveCommand, // G0 | G1 | G2 | G3`
  - `pub target: Position, // each axis is Option<f32> — missing means "unchanged"`
  - `pub extrusion: Option<f32>, // E parameter`
  - `pub feedrate: Option<u32>, // F parameter, mm/min`
  - `pub arc_center: Option<[f32; 2]>, // I/J for G2/G3`
  - `pub raw_offset: u64, // byte offset in the source; preserved for serializer round-trip`
  - `}`

- `pub struct Comment { pub text: String, pub semantic:
  Option<SemanticComment>, pub raw_offset: u64 }`. `SemanticComment`
  enum tags the comments the parser/serializer cares about:
  `FeatureType(FeatureType)`, `Layer(u32)`, `Z(f32)`,
  `ExtruderTemp(f32)`, `BedTemp(f32)`, `ToolChange(u8)`,
  `EstimatedTime(Duration)`, `FilamentUsed(u8, FilamentMass)`,
  ... Anything not matched stays as `None` and the text is
  preserved verbatim.

- `pub enum FeatureType { Perimeter, ExternalPerimeter, Infill,
  SolidInfill, TopSolid, Bridge, Support, Skirt, BrimSkirt, Travel,
  Other(String) }` — matches FR-GP-3's listed feature types. Free-
  form strings for forward compat with future feature labels.

- Positions are `[Option<f32>; 4]` for (X, Y, Z, E) so a partial
  move (e.g. `G1 X10` without Y/Z/E) round-trips exactly.

- All types `Debug + Clone + PartialEq + Serialize + Deserialize`.

- `impl Line { pub fn raw_offset(&self) -> u64 }` — every variant
  carries its source offset so PR-3-7 can re-emit in document
  order without re-parsing.

- Unit tests:
  - Construct each variant, serde-round-trip via JSON.
  - `FeatureType` parses from comment string ("perimeter" →
    `FeatureType::Perimeter`, "external perimeter" →
    `ExternalPerimeter`, ...) — table-driven with the canonical
    Orca strings.

**Effort.** ~1.5 days. Mostly type design + the FeatureType
parsing table. No I/O.

**Dependencies.** None — this is a leaf module. PR-3-6 + PR-3-7
both consume it.

**Out of scope.** Anything to do with parsing or emission — those
are PR-3-6 / PR-3-7. Per-segment timing (Phase 6 derives this from
flow + feedrate). Tool-coordinate transforms (those are libslic3r's
job, not ours).
