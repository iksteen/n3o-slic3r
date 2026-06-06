# PR-6-7 — Preview Tauri commands

Status: ✅ shipped.

**Scope.** Wire PR-6-4 (IR), PR-6-5 (colors), PR-6-6 (stats)
into Tauri commands the frontend renderer (PR-6-8) consumes.
Establishes the preview handle registry pattern: load once,
reference by handle, drop on close.

**Acceptance criteria.**

- New module `core/preview/commands.rs` + `core/preview/registry.rs`.

- **Registry:**
  ```rust
  pub struct PreviewRegistry {
      slots: Mutex<HashMap<PreviewHandle, LoadedPreview>>,
      next_id: AtomicU64,
  }

  pub struct LoadedPreview {
      pub source_path: PathBuf,
      pub header: HeaderMetadata,
      pub geometry: PreviewGeometry,
      pub layer_stats: Vec<PerLayerStats>,
      pub job_stats: FullJobStats,
      /// Original line stream for hover inspection back-refs
      /// (PR-6-11). Memory-heavy on 50MB gcode; keep as Vec<Line>
      /// for now and consider arena allocation if profiling shows
      /// hot.
      pub lines: Vec<gcode::Line>,
  }

  pub struct PreviewHandle(pub u64);
  ```

  Registered as Tauri-managed state in lib.rs.

- **Commands:**
  ```rust
  /// Load a .gcode file. Parses + builds IR + computes stats
  /// off-main on a tokio blocking task. Returns a handle for
  /// subsequent queries.
  #[tauri::command]
  pub fn preview_load(
      path: String,
      registry: State<Arc<PreviewRegistry>>,
  ) -> Result<PreviewLoadResponse, String>;

  pub struct PreviewLoadResponse {
      pub handle: PreviewHandle,
      pub header: HeaderMetadata,
      pub layer_count: u32,
      pub bounding_box: BoundingBox,
      pub job_stats: FullJobStats,
  }

  /// Return binary vertex/color/layer buffers for the
  /// requested handle + color mode. Mirrors the
  /// `scene_mesh_buffers` pattern (binary Response) so 50MB
  /// G-code → ~36MB binary buffer skips JSON stringification.
  ///
  /// Buffer layout (little-endian):
  ///   [positions_f32 …]    extrusions (N segments × 6 floats)
  ///   [colors_f32 …]       same length
  ///   [layer_index_f32 …]  same length / 3
  ///   [travel_positions_f32 …]  (M segments × 6 floats)
  ///   [travel_layer_index_f32 …]
  ///   [retraction_positions_f32 …]  (K points × 3 floats)
  ///   [retraction_layer_f32 …]
  /// Lengths derive from PreviewLoadResponse counts; caller
  /// computes offsets.
  #[tauri::command]
  pub fn preview_buffers(
      handle: PreviewHandle,
      color_mode: ColorMode,
      palette: Palette,
      registry: State<Arc<PreviewRegistry>>,
  ) -> Result<Response, String>;

  /// Return per-layer stats as JSON for the stats panel
  /// (PR-6-12). Small payload (one row per layer).
  #[tauri::command]
  pub fn preview_layer_stats(
      handle: PreviewHandle,
      registry: State<Arc<PreviewRegistry>>,
  ) -> Result<Vec<PerLayerStats>, String>;

  /// Hover inspection (PR-6-11): given a segment index in
  /// the extrusions array, return the original gcode line +
  /// position + speed + feature + layer.
  #[tauri::command]
  pub fn preview_segment_detail(
      handle: PreviewHandle,
      segment_index: u32,
      registry: State<Arc<PreviewRegistry>>,
  ) -> Result<SegmentDetail, String>;

  pub struct SegmentDetail {
      pub source_line_text: String,
      pub start: [f32; 3],
      pub end: [f32; 3],
      pub speed: f32,
      pub feature: FeatureType,
      pub layer_index: u32,
      pub tool: u8,
      pub extrusion_mm: f32,
  }

  /// Drop a loaded preview. Frees the geometry + line stream
  /// (50MB G-code frees ~250MB of RAM).
  #[tauri::command]
  pub fn preview_drop(
      handle: PreviewHandle,
      registry: State<Arc<PreviewRegistry>>,
  ) -> Result<(), String>;
  ```

- **Off-main parsing:** `preview_load` runs the parse + IR
  build + stats compute inside `tokio::task::spawn_blocking`.
  The Tauri command awaits the join handle. Surface the
  `Result` as `Err(String)` on parse / IO failures.

- **lib.rs registration:** all five commands registered.
  Arc<PreviewRegistry> initialized at startup like the
  existing `JobRegistry` pattern.

- Tests:
  - **Round-trip:** load a small synthetic gcode, request
    buffers in each color mode, assert buffer lengths match
    expected segment counts.
  - **Drop releases memory:** load + drop + assert the
    registry entry is gone (the GC-style assert is "can't
    look up the handle after drop").
  - **Concurrent loads:** two `preview_load` calls in
    parallel produce distinct handles + distinct geometries.
  - **Bad path** returns a clean error string.

**Effort.** ~1.5 days. Pattern-matches `scene_mesh_buffers`
+ `JobRegistry`; the binary buffer packing is the only
novel work.

**Dependencies.** PR-6-4 (IR), PR-6-5 (colors), PR-6-6
(stats), Phase 3 `gcode::parse_str` + `parse_header`.

**Out of scope.**

- Reading `.gcode.3mf` containers — PR-6-14 (drag-drop) does
  the unwrap; this command takes a path to a raw `.gcode`.
- Stats refresh on settings change — preview is static; new
  slice → new preview load.
- Streaming load progress events — load is a one-shot;
  spinner in the UI (PR-6-15) hides the latency.

**Cut candidate.** None.
