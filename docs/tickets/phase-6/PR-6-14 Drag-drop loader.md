# PR-6-14 — Drag-drop external `.gcode` + `.gcode.3mf` loader

Status: ❌ open.

**Scope.** Drop zone overlay on the preview viewport. Accepts
`.gcode` and `.gcode.3mf` files. For `.gcode.3mf`, unpacks the
embedded gcode + metadata via the existing threemf module.
Loads the result into the preview registry (PR-6-7) and mounts
the preview.

**Acceptance criteria.**

- New module `src/preview/DropZone.tsx`:
  ```tsx
  interface DropZoneProps {
    onLoaded: (handle: PreviewHandle) => void;
    onError: (message: string) => void;
  }
  ```

  Renders an invisible overlay that captures drop events
  when the user is dragging a file over the preview region.
  Visual feedback: dashed border + "Drop .gcode or
  .gcode.3mf here" message during drag.

- **Drop handling:**
  - Accept `.gcode` and `.gcode.3mf` extensions.
  - Reject anything else with `onError("only .gcode and
    .gcode.3mf files supported")`.
  - For `.gcode`: invoke `preview_load(path)` (PR-6-7).
  - For `.gcode.3mf`: invoke a new
    `preview_load_gcode_3mf(path)` (added in this ticket).

- **New Tauri command** (in `core/preview/commands.rs`):
  ```rust
  /// Unpack a .gcode.3mf, extract the first plate's embedded
  /// gcode, load it via the standard preview pipeline.
  /// Returns the same PreviewLoadResponse as `preview_load`
  /// plus the extracted plate metadata + thumbnail bytes.
  #[tauri::command]
  pub fn preview_load_gcode_3mf(
      path: String,
      registry: State<Arc<PreviewRegistry>>,
  ) -> Result<PreviewLoadGcode3mfResponse, String>;

  pub struct PreviewLoadGcode3mfResponse {
      pub preview: PreviewLoadResponse,
      pub plate_count: u32,
      pub plate_metadata: BBSPlateMetadata,  // type from PR-3-10
      pub thumbnail_png: Option<Vec<u8>>,
  }
  ```

  Implementation: reuse PR-3-9 part 1's threemf reader to
  open the container; reuse PR-3-10's plate-metadata
  extractor; the embedded gcode lives at a known path inside
  the 3MF (e.g. `Metadata/plate_1.gcode` per Bambu's
  convention).

- **Multi-plate `.gcode.3mf`:** open question per the
  index. MVP behavior: load the first plate. Log a tracing
  warning if `plate_count > 1`. Surface a "this 3MF has N
  plates; showing plate 1" badge in the panel chrome.

- **Stats panel integration:** when a `.gcode.3mf` loads,
  the full-job stats panel additionally surfaces the
  extracted plate metadata (estimated time, AMS bindings)
  and the thumbnail (small inline image above the time
  breakdown).

- Tests (Rust + frontend):
  - **`.gcode` round-trip:** drop a small synthetic
    `.gcode`, assert `preview_load` is invoked + handle
    flows to `onLoaded`.
  - **`.gcode.3mf` round-trip:** use a synthetic 3MF
    container with embedded gcode, assert
    `preview_load_gcode_3mf` extracts correctly.
  - **Bad extension:** dropping a `.png` triggers
    `onError`.
  - **Tauri backend smoke** (in the existing
    `phase3_smoke`-style integration test): synthesize a
    `.gcode.3mf` via PR-3-10's writer, then load it via
    the new command, assert preview handle is valid.

- **Drop zone scoping:** active only in preview mode.
  Drops on the 3D viewport (scene mode) are a separate
  flow (existing PR-2-3 mesh import via dialog).

**Effort.** ~1.5 days. The drop-zone UI is fast; the
`.gcode.3mf` unwrap reuses existing code; the metadata
surfacing in the stats panel is the main integration
work.

**Dependencies.** PR-6-7 (preview registry + commands),
PR-6-12 (stats panels — extend with the .gcode.3mf
metadata surface), PR-3-9 part 1 (threemf reader),
PR-3-10 (.gcode.3mf writer + metadata types).

**Out of scope.**

- Multi-plate plate-picker UI for `.gcode.3mf` (deferred
  per the index's open question; MVP loads plate 1).
- Recent files menu for external G-code (Phase 9).
- Save-to-disk after preview tweaks (preview is read-only).

**Cut candidate.** Drop `.gcode` (raw) support → save ~0.5
days. Keep `.gcode.3mf` only. Per Exec Plan cut LAST;
hurts the standalone-preview-as-viewer story. **Not
recommended** — user signed off on both.
