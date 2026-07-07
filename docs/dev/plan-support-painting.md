# Paint-on Supports (manual tree supports) — implementation plan

## Context

n3o can slice with tree supports, but there is no way to *paint* where supports
go. Orca's workflow — `support_type = tree(manual)`, paint enforcer regions,
only painted areas grow trees — is the target. n3o already pipes per-triangle
hex paint strings end-to-end for MMU color (`Mesh.paint_colors` → .n3o →
slice input → FFI `apply_paint` → `mmu_segmentation_facets`); support paint is
the *same string format* into the sibling `supported_facets`. The missing
pieces are the interactive brush, the storage/wiring, and the FFI.

**Decided scope:** sphere + circle brush AND smart fill (angle-bounded seed
fill); enforce (LMB) / block (RMB) / erase (Shift+LMB); adjustable radius
(slider + Ctrl-wheel); per-stroke undo. **Sub-triangle Orca parity**: painting
goes through libslic3r's `TriangleSelector` via a new FFI paint session — no
brush-geometry reimplementation in Rust.

Verified premises:
- `TriangleSelector.cpp` is already compiled into the FFI's libslic3r;
  `slic3r_ffi.cpp` already includes `TriangleSelector.hpp`.
- Signatures at `external/OrcaSlicer/src/libslic3r/TriangleSelector.hpp`:
  `select_patch` (:306), `seed_fill_select_triangles` (:314) +
  `seed_fill_apply_on_triangles` (:374), `SinglePointCursor::cursor_factory`
  (:114, takes mesh-local center + camera, world radius, CIRCLE|SPHERE,
  mesh→world trafo, default `ClippingPlane()` = inactive).
- `FacetsAnnotation` has private ctors → the session owns a one-volume
  `Model` internally and uses `vol->supported_facets` for
  `set()`/`get_triangle_as_string()` (exact Orca hex output, zero
  reimplementation).
- Winding flip: `slic3r_model_add_object`/`add_volume` flip triangles when
  `mesh.volume() < 0` (slic3r_ffi.cpp:1058/1141). The paint session MUST apply
  the identical flip so selector state, stored strings, and slice-time facets
  index the same winding (flip preserves triangle order, so strings stay
  index-aligned). Sub-triangle split encodings are winding-relative —
  load-bearing.
- Slicing consumption already exists in libslic3r: `TreeSupport.cpp:997`,
  `PrintObject::project_and_append_custom_facets`, manual modes
  (`normal(manual)` / `tree(manual)`) use ONLY painted enforcers.
- `support_type` already renders in the Process panel via generic enum
  introspection — no settings-panel work.

## Phase 1 — FFI paint session

`crates/slic3r-ffi/ffi/slic3r_ffi.h` + `.cpp` (follow existing idiom: opaque
handle, `slic3r_status`, `catch(...)` + `set_err`, `slic3r_free_string_array`
/ `slic3r_cut_mesh_free` for buffer frees):

```c
slic3r_paint_session_t* slic3r_paint_session_new(
    const float* verts, size_t vcount, const uint32_t* indices, size_t tcount,
    const char* const* paint_hex, size_t paint_count, char** out_err);
void slic3r_paint_session_free(slic3r_paint_session_t*);
slic3r_status slic3r_paint_session_stroke(s, int32_t facet_start,
    const float hit[3], const float camera_pos[3], const double trafo[16],
    float radius, uint32_t cursor_type /*0=CIRCLE 1=SPHERE*/,
    uint32_t new_state /*0=NONE 1=ENFORCER 2=BLOCKER*/,
    int32_t push_undo, char** out_err);
slic3r_status slic3r_paint_session_fill(s, int32_t facet_start,
    const float hit[3], const double trafo[16], float seed_fill_angle_deg,
    uint32_t new_state, int32_t push_undo, char** out_err);
int  slic3r_paint_session_undo(s);            // pops TriangleSplittingData snapshot
slic3r_status slic3r_paint_session_serialize(s, char*** out_paint,
    size_t* out_count, char** out_err);        // per-triangle hex, "" = unpainted
slic3r_status slic3r_paint_session_facets(s, uint32_t state,
    float** out_verts, size_t* out_vcount, uint32_t** out_indices,
    size_t* out_tcount, char** out_err);       // selector.get_facets(state)
```

Session struct: `{ Model model; ModelVolume* vol; TriangleSelector selector;
std::vector<TriangleSplittingData> undo_stack /* cap ~64 */ }`. Apply the
volume<0 winding flip in `_new`. Validate incoming strings with the existing
`paint_string_well_formed` (slic3r_ffi.cpp:642). `push_undo` snapshots
`selector.serialize()` before applying (frontend sets it on pointerdown →
one drag = one undo step). Fill flow: `seed_fill_select_triangles(hit, facet,
trafo_no_translate, ClippingPlane(), angle, 0.f, /*force_reselection=*/true)`
then `seed_fill_apply_on_triangles(state)`.

Rust wrapper in `crates/slic3r-ffi/src/lib.rs`: `PaintSession` (Drop,
`unsafe impl Send` like `Model`), `BrushKind{Circle,Sphere}`,
`PaintState{None,Enforcer,Blocker}`, methods `new/stroke/fill/undo/serialize/
facets`, boundary validation mirroring `validate_paint`/`validate_indices`.

Test `crates/slic3r-ffi/tests/paint_smoke.rs` (pattern of `cut_smoke.rs`):
cube → sphere stroke → serialize has non-empty strings + facets(Enforcer)
non-empty; undo → empty; smart fill at 30° fills exactly one cube face, not
neighbors; round-trip: re-seeding a new session with serialized strings
reproduces facets.

## Phase 2 — slice wiring (prove the support path via gcode early)

- `slic3r_model_add_object` / `slic3r_model_add_volume` gain
  `const char* const* support_hex, size_t support_count`. Generalize
  `apply_paint` (slic3r_ffi.cpp:693) to take a `FacetsAnnotation&` target;
  call for both `mmu_segmentation_facets` and `supported_facets`.
- Rust: sys decls + `Model::add_object`/`add_volume` (lib.rs:1134/1208) +
  call sites `src-tauri/src/core/slice/orchestrator.rs:449/468/487`,
  `crates/slic3r-ffi/tests/api.rs`, `phase_s_smoke.rs`,
  `src-tauri/examples/slice_repro.rs` (mechanical `&[]`).
- `SliceObject.support_paint: Option<Arc<Vec<String>>>`
  (`src-tauri/src/core/slice/input.rs:42`); `build_slice_input` copies it.
- Test: cantilever fixture (box + overhanging lip), `enable_support=true`,
  `support_type=tree(manual)`, enforcer paint on the overhang underside
  (use serialize output from Phase 1 for realistic strings) → slice →
  **grep gcode for the support feature marker** (verify the exact marker
  string against existing gcode assertions in the phase6 fixtures when
  implementing); negative control: no paint → no support lines.
  (House rule: verify via gcode, not green tests.)

## Phase 3 — persistence

- `Mesh.support_paint: Option<Arc<Vec<String>>>`
  (`src-tauri/src/core/scene/state.rs:82`, `#[serde(skip)]` like
  paint_colors) + `NewMesh.support_paint` (~8 mechanical construction sites:
  stl.rs, obj.rs, primitives.rs, threemf/mod.rs, geometry.rs, tests).
  `register_mesh` copies it. `MeshHeader` unchanged (paint never crosses the
  JSON wire).
- `.n3o` (`src-tauri/src/core/project/format.rs`): postcard blobs are
  positional → bump `FORMAT_VERSION` "2"→"3"; reader accepts {"2","3"} and
  decodes the legacy blob struct for "2" (support_paint=None); writer emits
  "3". Back-compat test against a real v2 file (repo has `test.n3o`,
  `lil-stormy.n3o`).
- 3MF import (`src-tauri/src/core/threemf/core_spec.rs:256`): in the
  `b"triangle"` arm read `paint_supports` (BBS attr) AND
  `slic3rpe:custom_supports` (Prusa-style) — same dual-attr pattern as the
  existing paint_color pair; thread through `ObjectBody::Mesh` →
  `threemf/mod.rs` → `NewMesh`. orca_import goes through this same seam
  (verified).
- Commit mutation `Project::apply_support_paint` (new, in
  `src-tauri/src/core/project/mutation/geometry.rs`): register a mesh clone
  (Arc-shared vertex/index/paint_colors) with new support_paint under a
  **fresh MeshId**, repoint `SceneObject.mesh`, prune the old mesh, emit
  MeshLoaded + ObjectUpdated. Fresh-MeshId avoids stale renderer GpuMesh
  cache and makes global undo/redo work with the existing history snapshots.
- Cut tool drops support paint in v1 (deferred-cut FFI only carries MMU
  paint) — acceptable; paint after cutting.
- Milestone: an Orca 3MF with painted supports slices correctly in n3o with
  zero UI.

## Phase 4 — viewport tool

- Picking: extend `nearest_hit` (`src-tauri/src/viewport_render.rs:855`) to
  return the winning triangle index. Paint commands raycast server-side per
  stroke sample (frontend streams PickRequest-shaped pointer positions).
- Session state: `src-tauri/src/paint_session.rs`,
  `PaintToolState(Mutex<Option<ActivePaint>>)` registered alongside
  `ViewportState` (lib.rs:70). Tauri commands: `paint_open(object_id)`,
  `paint_stroke(pick, radius, brush, action, apply, new_stroke)` (hover uses
  apply:false to place the cursor ring), `paint_fill(pick, angle, action,
  new_stroke)`, `paint_undo`, `paint_apply` (serialize → no-op if unchanged →
  else `apply_support_paint`), `paint_cancel`.
- Rendering: renderer gains `paint_overlay` (per-state vertex buffers from
  `PaintSession::facets`, rebuilt after each stroke/fill/undo under the
  ViewportState mutex — `viewport_invalidate_tower` precedent; 0.001·normal
  offset; enforcer blue / blocker red). Cursor ring: `FrameRequest` gains
  `#[serde(default)] paint: Option<PaintPreview{hit,normal,radius,state}>`
  drawn via the existing line_pipe (exactly how `cut: Option<CutPreview>`
  flows). Applied paint outside the tool: extend `upload_mesh` partition to
  include the support dominant state (`decode_dominant_states` works
  verbatim on support strings) and tint those groups.
- Frontend: `"paint"` in `ViewportToolMode`
  (`src/viewport/useViewportTools.ts:13`); `src/viewport/usePaintSession.ts`
  (radius 0.4–8 default 1.0, smart-fill angle 0–90 default 30 — Orca's
  constants) + `src/viewport/PaintPanel.tsx` mirroring SplitPanel (floating
  overlay, Esc=cancel, Apply/Cancel, radius+angle sliders, brush toggle,
  enforce/block/erase implicit via mouse buttons). New drag mode `"paint"`
  in `WgpuViewport.tsx:51`; LMB=enforce RMB=block Shift+LMB=erase (RMB pan
  suppressed while tool active; middle-drag still pans); Ctrl+wheel=radius;
  stroke samples throttled by an in-flight flag (send next sample only after
  the previous invoke resolves).
- Undo gating: `useUndoRedo(split.active || paint.active)` (App.tsx:104);
  Ctrl+Z while the tool is open routes to `paint_undo`.
- Panel hint: if `enable_support` off or `support_type` is an auto mode, show
  a one-liner with a quick-set button writing plate-tier overrides
  (`enable_support=true`, `support_type=tree(manual)`) via the existing
  override command. No auto-magic on apply.

## Verification

- Per phase: `cargo test -p slic3r-ffi` (Phase 1–2), `cargo test -p
  n3o-slic3r` (Phase 3), full `cargo test --workspace` + `npx vitest run` +
  `npx tsc --noEmit` before each commit (N3O_SLIC3R_FFI_CMAKE_CONFIG=Release).
- Phase 2 gcode grep is the real proof: painted enforcer + tree(manual) →
  support feature lines present; unpainted → absent.
- Phase 4 ends with a manual visual pass (brush feel, cursor ring, overlay
  colors, undo, apply→slice).

## Risks

- `FacetsAnnotation` private ctor — mitigated via session-owned Model;
  fallback: hand-roll the bitstream→hex walk (FFI already has the inverse in
  `paint_string_well_formed`).
- Winding-flip parity between session and slice path is load-bearing
  (sub-triangle encodings are winding-relative); covered by the round-trip
  smoke test.
- Postcard v2→v3 dual decode must be tested against a real v2 file.
- Per-stroke `get_facets` extraction is O(mesh) — in-flight throttle bounds
  it; optimize later only if large meshes lag.
- Orca pin bumps can shift TriangleSelector signatures — paint_smoke.rs makes
  that fail loudly.
