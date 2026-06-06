# PR-3-10 — `.gcode.3mf` writer (Bambu sliced format)

Status: ✅ shipped — `core/threemf/sliced.rs` emits the Bambu sliced bundle from a `SlicedProjectInput { plates, printer_model, file_metadata }`. Per-plate: `Metadata/plate_<N>.gcode` (verbatim bytes), `Metadata/plate_<N>.gcode.md5` (self-contained MD5 impl — no dep added; algorithm is small + Bambu-firmware-specific), `Metadata/plate_<N>.json` (PR-3-3's summary + AMS bindings serialized), optional `Metadata/plate_<N>.png` thumbnail. Main 3dmodel.model carries BambuStudio namespace metadata. 7 unit tests including byte-equal G-code round-trip, multi-plate, MD5 reference vector, AMS-binding JSON. End-to-end Bambu validation defers to Phase 7a's first real print.

**Scope.** The variant of 3MF that the Bambu A1 mini driver
(Phase 7a) sends to the printer: a 3MF container with an embedded
G-code blob plus Bambu's metadata extensions (plate thumbnails,
filament aggregates, print time, AMS bindings). Per FR-MP-4b: the
A1 mini driver wraps slice output as `.gcode.3mf`; the U1 driver
doesn't use this path.

This is the writer-side of the `.gcode.3mf` format. The reader for
drag-drop preview is Phase 6.

PR-0.5-3's spike already inventoried the metadata fields BBS
populates. Use that inventory as the schema source of truth.

**Acceptance criteria.**

- `core/threemf/sliced.rs` (or as a submodule of `core/threemf/writer`):
  - `pub fn write_sliced_3mf(input: SlicedProjectInput, output:
    &Path) -> Result<(), WriteError>`.
  - `pub struct SlicedProjectInput {`
    - `pub scene: SceneState, // geometry + objects (mirrors PR-3-9)`
    - `pub plates: Vec<SlicedPlate>,`
    - `pub printer: PrinterProfile,`
    - `pub project_metadata: ProjectMetadata,`
    - `}`
  - `pub struct SlicedPlate {`
    - `pub plate_id: u32,`
    - `pub gcode_path: PathBuf, // libslic3r output from PR-3-2`
    - `pub summary: PlateSummary, // PR-3-3`
    - `pub thumbnail: Option<DynamicImage>, // RGBA, libslic3r emits one`
    - `pub ams_bindings: Vec<AmsBinding>, // model material → AMS slot`
    - `}`

- Emits the following per-plate (from PR-0.5-3 inventory):
  - `Metadata/plate_<N>.gcode` — the G-code body itself.
  - `Metadata/plate_<N>.png` — plate thumbnail.
  - `Metadata/plate_<N>.gcode.md5` — checksum the firmware verifies.
  - `Metadata/plate_<N>.json` — Bambu's per-plate aggregate (print
    time, filament usage, AMS bindings).

- Bambu-namespace metadata in the main `3dmodel.model`:
  - `<metadata name="BambuStudio:3mfVersion">1</metadata>` — required.
  - `<metadata name="Application">n3o-slic3r-<version></metadata>` —
    so the printer logs it correctly.
  - The AMS-binding metadata block (per PR-0.5-3's finding).

- Validation: at the end of `write_sliced_3mf`, the writer
  re-opens the file via PR-2-4's reader (or a phase-2-tagged
  Phase 6 reader if that lands first) and asserts the embedded
  G-code byte-matches the input. If it doesn't, the writer
  returns `Err` and deletes the partial file rather than ship a
  corrupt bundle.

- Tests:
  - End-to-end: build a small scene, slice it via PR-3-2,
    wrap it via this writer, unzip the result, assert the
    expected files exist and the G-code body byte-matches the
    pre-wrap output.
  - Bambu-side validity: open the resulting `.gcode.3mf` in
    Bambu Studio (manual test, document inline) and confirm the
    plate preview renders + the print time matches what the
    summary said. Phase 7a re-validates with a real print.

**Effort.** ~3 days. The container layer is shared with PR-3-9;
this ticket's complexity is the Bambu-specific metadata population
+ the validation that BBS accepts it. Phase 7a's first real-print
test will surface any field we got wrong.

**Dependencies.** PR-3-9 (writer container infrastructure), PR-3-3
(summary data), PR-3-2 (slice output).

**Cut candidate (per Execution Plan §5).** Skip the complex
metadata extensions — emit minimum-viable `.gcode.3mf` with G-code
body, plate JSON, and thumbnail only. Saves ~2 days. **Risk:** the
A1 mini may reject the bundle or fail to display the plate preview
correctly. Phase 7a's real-print test would catch this; tradeoff is
worth taking only if the schedule is already cutting into Phase 4.

**Out of scope.** `.gcode.3mf` for the U1 — U1 doesn't accept this
format; Phase 7b sends raw `.gcode`. The reader path for preview
drag-drop — Phase 6. Filament-state population (AMS bindings get a
real value here; PR-3-10 emits `bindings: vec![]` if Phase 7c's
sync data isn't available; Phase 7c re-fills the population).
