# Slice buffer-load: feeding libslic3r geometry in-memory

## Problem

The slice path feeds plate geometry to libslic3r by **writing a temporary
`.3mf` file and re-parsing it**:

1. `slice::input::build_slice_input` builds a `Project3mf` from the plate's
   objects, then `threemf::write_3mf` serializes it to a temp `.3mf`
   (vertices/triangles as XML text, zipped).
2. The worker (`slice::orchestrator`) calls `Model::load(temp_path)`, which
   makes libslic3r **parse that 3MF back** into a `Model`.

So geometry we already hold in RAM round-trips through XML twice — our
serialize and libslic3r's parse — plus a disk write and (before the `Arc`
buffer change) a `NewMesh` deep-copy. For a large model (e.g. stormtrooper)
this is the bulk of the "prepare" stage and the worker's "loading" stage.

Nothing crosses the IPC bridge in any of this — it's all Rust/C++ side. The
3MF is purely an in-process serialization format we're paying for needlessly.

## Approach

Build libslic3r's `Model` **directly from the in-memory buffers** via a new
FFI entry point, skipping the temp `.3mf` entirely on the slice path. The
geometry-ingest pattern is already proven in `slic3r_ffi.cpp`
(`slic3r_orient_mesh` builds an `indexed_triangle_set` → `TriangleMesh` from
raw `float`/`uint32` arrays).

Combined with the `Arc<Vec<_>>` mesh buffers, the slice path then does **zero
geometry copies on our side** — the buffers flow from the scene straight to
the FFI; the only unavoidable copy is libslic3r ingesting them into its
`TriangleMesh`.

### The paint de-risk

The one feared part — multi-material face painting — is nearly free. Our
`Mesh.paint_colors` are the **opaque BBS `paint_color` hex strings**,
preserved verbatim through import/round-trip. libslic3r's own 3MF reader does
nothing more than extract that hex string per triangle and call
`ModelVolume::mmu_segmentation_facets.set_triangle_from_string(i, hex)`. So we
call the exact same API with the exact same strings — no paint codec to
replicate.

## New FFI surface (`crates/slic3r-ffi`)

One C function, mirroring `slic3r_orient_mesh`'s ingest:

```c
slic3r_status slic3r_model_add_object(
    slic3r_model_t* m, const char* name,
    const float* verts, size_t vcount,            // object-local, flat XYZ
    const uint32_t* indices, size_t tcount,       // flat triangle triples
    const double transform[16],                   // object→world, row-major 4x4
    int extruder,                                 // 1-based → config["extruder"]
    const char* const* paint_hex, size_t paint_count, // per-triangle BBS hex; "" unpainted
    char** out_err);
```

C++ body (all standard libslic3r API — see `docs` research notes):
- `indexed_triangle_set` from the arrays → `TriangleMesh` (as `slic3r_orient_mesh`).
- `Model::add_object()` + `ModelObject::add_volume(std::move(mesh))`.
- `obj->config.set_key_value("extruder", new ConfigOptionInt(extruder))`.
- `obj->add_instance()` + `inst->set_transformation(Transform3d)` from the
  4x4 (object→world).
- If any `paint_hex[i]` is non-empty: `vol->mmu_segmentation_facets.reserve(n)`,
  `set_triangle_from_string(i, paint_hex[i])` in increasing `i`, `shrink_to_fit()`.

Rust wrapper: `Model::add_object(name, &[f32], &[u32], &Transform, extruder, &[String])`
fed straight from the `Arc` buffers (no copy our side).

## Data-flow changes

- `SliceJobInput` / `ResolvedJob`: replace `model_path: String` with a list of
  per-object geometry (name, vertex/index `Arc`s, object→world transform,
  extruder, paint). `build_plate_geometry` already computes exactly this
  (it's what it serializes into the `Project3mf`); it just travels in-memory
  now instead of via a temp file.
- Worker: replace `Model::load(path)` with a loop of `model.add_object(...)`,
  then `add_default_instances` is unnecessary (we add instances explicitly).
  `paint_filament_remap` still applies post-build (it rewrites the loaded
  facets — unchanged). No temp file, no cleanup hook on the slice path.

## Kept as-is

- 3MF **import** (`orca_import` / reading external `.3mf` projects).
- G-code **export** bundle for send (`write_sliced_3mf`).
This change only swaps the internal slice feed.

## Risks

1. **Transform mapping** — our glam matrix → Eigen `Transform3d` must
   reproduce what the 3MF instance-transform path produced. Caught by the
   parity gate.
2. **Loader bookkeeping** — confirm nothing the 3MF loader does that slicing
   needs is missed. Research says only `add_default_instances` (sidestepped by
   adding instances explicitly); repair / `ensure_on_bed` aren't required.
3. **Empty paint** — a fully-unpainted volume must leave
   `mmu_segmentation_facets` empty (skip `set_triangle_from_string` for `""`).

## Verification & rollout

1. **Parity gate first.** A test builds the same geometry two ways — (a)
   `write_3mf` + `Model::load`, (b) `Model::add_object` — slices both with an
   identical config, and asserts **byte-identical G-code**. Cover plain,
   single-material, multi-material, and **painted** fixtures.
2. Switch the production slice path to buffer-load.
3. Once parity holds, delete the temp-`.3mf` write from the slice path. The
   `write_3mf` writer stays only if a non-test caller remains (currently the
   slice temp is its only production caller, so it can go with it).

## Status

Implemented per the stages above; the parity gate is
`tests/slice_buffer_load_parity.rs`.
