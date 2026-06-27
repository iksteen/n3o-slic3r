# Feeding libslic3r geometry in-memory

The slice path builds libslic3r's `Model` **directly from the in-memory mesh
buffers** — no temp `.3mf`, no XML serialize/parse. The geometry never crosses
the IPC bridge: `SliceObject`'s `Arc<Vec<_>>` buffers flow from the scene
straight into the FFI, and the only unavoidable copy is libslic3r ingesting them
into its `TriangleMesh`. The native project format is `.n3o`; 3MF is import-only
(`orca_import`), and the printer send-format is a *sliced* `.gcode.3mf`
(`write_sliced_3mf`) — neither involves the slice geometry feed.

## FFI surface (`crates/slic3r-ffi`)

Three construction shims over libslic3r, all ingesting raw `float`/`uint32`
arrays the way `slic3r_orient_mesh` does:

- `slic3r_model_add_object(m, name, verts, indices, transform[16], extruder,
  paint_hex, overrides)` — one `ModelObject` with a single volume. The world
  transform rides the **instance**; the volume stays centered. This is the solo
  path, matching the `.3mf` loader's non-component path.
- `slic3r_model_add_group(name) → object_index` — an empty `ModelObject` with an
  **identity instance**, for a multi-volume group.
- `slic3r_model_add_volume(object_index, …)` — appends one `ModelVolume` to a
  group, with the member's **world transform on the volume** and
  `extruder`/overrides on the volume config (each member prints with its own
  filament).

Shared C helpers keep the three consistent: `its_from_buffers` (array → mesh,
flips inverted winding to match the loader), `apply_paint`, `apply_overrides`.
The Rust wrapper bounds-checks indices (libslic3r indexes the vertex array
unchecked) and marshals the paint/override `CString`s via `ObjectStrings`.

### Paint is nearly free

`Mesh.paint_colors` are the **opaque BBS `paint_color` hex strings**, preserved
verbatim through import. libslic3r's own 3MF reader does nothing more than call
`ModelVolume::mmu_segmentation_facets.set_triangle_from_string(i, hex)` per
triangle — so `apply_paint` calls the exact same API with the exact same
strings. No paint codec to replicate. An unpainted volume leaves
`mmu_segmentation_facets` empty (`""` entries skipped).

### Group transform composition

A scene **group** is one `ModelObject` carrying N `ModelVolume`s (e.g. the two
halves of a cut model must slice as one object, or the regions "float"). It
mirrors the `.3mf` writer's `<components>` shape: identity build-item, each
component carrying the volume's world transform. To produce geometry identical
to what loading such a `.3mf` would, `add_volume` reproduces the loader's
component path (`bbs_3mf.cpp`): `add_volume(mesh)` (default
`modify_to_center_geometry`) centers the mesh and bakes a compensating
translation into the volume transform, then `set_transformation(world * current)`
composes the placement onto it.

## Worker assembly (`slice::orchestrator`)

`build_model_objects` buckets the plate's objects into build units via
`build_units` — first-appearance order, a one-member group collapses to a solo
(mirroring the writer's `Layout`). Then:

- solo unit → `Model::add_object`
- group unit → `Model::add_group` + one `Model::add_volume` per member

`paint_filament_remap` still applies post-build (it rewrites the painted facet
states for toolchanger filament routing). No temp file, no cleanup hook.

## Verification

- **Forward correctness:** `tests/slice_orchestrator.rs`
  (`resolved_bed_temp_reaches_the_engine`, `user_tier_override_reaches_the_engine`)
  and the smoke tests slice through this path and assert the resolved values +
  per-object overrides reach the G-code. `phase_s_smoke`'s
  `imported_object_override_reaches_the_engine_end_to_end` imports a foreign 3MF
  with an object-scoped `layer_height` override (fixture
  `cube-override-pair.py`) and asserts it changes the slice.
- **Grouping logic:** `build_units` is pure over `o.group` and unit-tested
  (first-appearance order, one-member-group-as-solo).
- **Group transform composition:** `grouped_member_rotation_composes_in_world_space`
  slices a rotated group member two ways — via the volume transform and with that
  transform baked into the vertices (identity transform, the order-insensitive
  oracle) — and asserts the extruded footprints match. A rotation doesn't commute
  with `add_volume`'s centering, so this pins the `world * current` order
  (verified to fail if the multiplication is flipped).
- **FFI construction:** `add_group_then_volumes_builds_a_multivolume_object`
  exercises the group shims + out-of-range rejection.
