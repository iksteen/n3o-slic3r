# PR-3-6 — Streaming G-code parser

Status: ❌ open.

**Scope.** Reader that turns a G-code byte stream into an iterator of
PR-3-5's typed `Line` values. Streams — does not buffer the whole
file in memory — and recognizes the structured comments libslic3r
emits so the parsed output is *informationally* equivalent to the
input (PR-3-7 proves this byte-for-byte).

Lives in `core/gcode/parser.rs`.

**Acceptance criteria.**

- `pub fn parse_lines<R: BufRead>(input: R) -> impl Iterator<Item =
  Result<Line, ParseError>>` — streaming, exits on EOF without
  reading the whole file at once. Production callers use this
  through a wrapper that materializes into a `Vec<Line>` for the
  preview (Phase 6) or iterates lazily for plugins (Phase 8).

- `pub fn parse_str(src: &str) -> Vec<Line>` — convenience for
  tests and small G-code snippets.

- Recognizes:
  - `G0` / `G1` / `G2` / `G3` with all parameters (X/Y/Z/E/F/I/J).
  - `T<n>` tool changes (with optional whitespace before the
    number; `T 0` is rare but valid).
  - `M0` / `M1` pauses (preserved as `Other`).
  - Comments via `;` and `(...)` (Marlin/Klipper both used).
  - Structured comments libslic3r emits:
    - `; TYPE: <feature>` → `SemanticComment::FeatureType(...)`
    - `;LAYER:N` and `; CHANGE_LAYER` → triggers `Line::LayerChange`
      synthesis adjacent to the move that follows.
    - `;Z:1.2` → `SemanticComment::Z`
    - `; estimated printing time = …`
    - `; filament used [g] = …`
    - `; printer_model = …` etc.

- Feature-type annotation: when a `Move` follows a `;TYPE:` comment
  (with no intervening `;TYPE:` reset), the move's effective
  feature is the most recent declared type. Expose this via an
  iterator-adapter `parse_with_feature_context` that yields
  `(Line, Option<FeatureType>)` pairs; the bare `parse_lines`
  doesn't do the carry-over since the type lives on the comment,
  not the move.

- `LayerChange` synthesis: the parser inserts a synthetic
  `Line::LayerChange` exactly once per detected boundary. Boundary
  detection rules, in priority order:
  1. `; CHANGE_LAYER` or `;LAYER:N` comment seen.
  2. Z-axis advance in a `G0`/`G1` move when the previous move was
     on a different Z and the move included a Z parameter.
  3. Heuristic: a `G1` with `E < 0` (retract) followed by a `G0`
     with no E (travel) followed by a `G1` with a Z change — the
     extruder retracted, traveled, lifted. Lowest priority; only
     fires when (1) and (2) didn't.

- `pub struct ParseError {`
  - `pub byte_offset: u64,`
  - `pub line_number: u32,`
  - `pub kind: ParseErrorKind, // InvalidNumber, UnexpectedToken, IoError`
  - `pub raw_line: String,`
  - `}` — production callers log and continue (the iterator can
    yield `Err` then keep going), tests assert specific kinds.

- Lenient defaults: unknown commands → `Line::Other`, not `Err`.
  Malformed numbers within a recognized command → `Err` with
  byte offset.

- Tests:
  - Spike fixtures: parse `examples/spike1/*.gcode`,
    `examples/spike2/*.gcode`, `examples/spike3/*.gcode`. Each
    must parse to completion with zero `Err` results. Assert
    layer counts match the in-comment `; total layers count =`
    value where present.
  - Per-variant micro-tests: hand-written 5–10 line snippets
    exercising each `Line` variant.
  - Feature carry-over: `;TYPE:perimeter\nG1 X10\nG1 X20\n;TYPE:infill\nG1 X30`
    yields features `[Perimeter, Perimeter, Infill]` via
    `parse_with_feature_context`.
  - Streaming behavior: parse a 50 MB synthetic fixture, assert
    peak memory stays under 50 MB (i.e., the iterator doesn't
    secretly buffer).

- Performance gate: parse 50 MB of G-code in < 3 s on the dev rig
  per Execution Plan §5 exit criteria. Land as
  `src-tauri/tests/gcode_parser_perf.rs` mirroring the
  `cascade_perf` + `scene_state_perf` pattern.

**Effort.** ~3 days. The line scanner + parameter extraction is
~1 day; the structured-comment dispatch + feature-context iterator
is ~0.5 day; tests + perf gate is the rest.

**Dependencies.** PR-3-5 (model types).

**Out of scope.** Color / flow / per-segment time calculations —
Phase 6 derives those from the parsed model. Round-tripping (PR-3-7).
Non-libslic3r dialect quirks beyond `;TYPE:` / `;LAYER:`. Cura's
`;TIME_ELAPSED` and Prusa's `;HEIGHT:` come in via PR-3-8 (header
metadata parser) where the multi-slicer comment dialect lives.
