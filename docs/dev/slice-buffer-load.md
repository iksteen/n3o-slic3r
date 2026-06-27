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

## Grouped (multi-volume) objects — the temp-3MF fallback stays

A scene **group** is one libslic3r `ModelObject` carrying **N `ModelVolume`s**
(e.g. the two halves of a cut model must slice as one object, or the regions
"float" — see `cube_halves_slices_as_one_multivolume_object_no_floating_warning`).
`slic3r_model_add_object` builds one `ModelObject` with a *single* volume, so it
cannot represent a group. Until an FFI `add_volume` (append a volume to an
existing object) lands, **plates that contain a grouped object fall back to the
temp-`.3mf` path**, which collapses each group into one multi-volume
`ModelObject`. `build_slice_input` detects this and sets `force_temp_3mf`.

So the temp-`.3mf` path is **not deleted** — it remains the correctness path for
grouped plates (and the parity comparator). `write_3mf` stays. The buffer-load
fast path covers the common ungrouped case.

## Verification & rollout — as implemented

- **Buffer-load is the default**; `SliceObject` (Arc-shared buffers) flows from
  `build_slice_input` → the worker's `Model::add_object` loop. No temp file, no
  XML serialize/parse for ungrouped plates.
- **Grouped plates** auto-route to the temp-`.3mf` fallback (`force_temp_3mf`).
- **Parity gate:** `tests/slice_buffer_load_parity.rs` slices the same plate
  both ways (single, two-object, painted/MMU) and asserts **byte-identical
  G-code**. Caveat: libslic3r's seam/feedrate output is nondeterministic under
  multithreading (two *identical* slices can differ ~1 run in 6), so the test
  slices each path twice and only asserts `buffer == temp_3mf` on a run where
  each path agrees with itself — it never false-fails and catches a real
  divergence. Parity verified byte-identical 8/8 single-threaded
  (`RAYON_NUM_THREADS=1 taskset -c 0`). The forward correctness guard for the
  buffer-load path is the existing `slice_orchestrator` G-code-value tests
  (`resolved_bed_temp_reaches_the_engine`, `user_tier_override_reaches_the_engine`),
  which now run through buffer-load.

## Future work

An FFI `slic3r_model_add_volume(object, …)` would let groups buffer-load too,
retiring the temp-`.3mf` fallback entirely.
