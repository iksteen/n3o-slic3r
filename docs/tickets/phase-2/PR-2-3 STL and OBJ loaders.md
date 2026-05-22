# PR-2-3 — STL + OBJ loaders

Status: ❌ open.

**Scope.** Mesh loaders for the two most common file formats. STL
is ubiquitous; OBJ shows up frequently when users export from CAD
that prefers it. Both produce a `Mesh` (from PR-2-1) that the scene
state owns. Loading runs in Rust, populates the registry via
`scene_load_mesh` (PR-2-2), and emits the `scene:mesh_loaded` +
`scene:object_added` events.

Format detection is by extension first, magic-byte sniff second
(STL has both ASCII and binary variants; the loader picks based on
the file's header).

**Acceptance criteria.**

- `pub fn load_stl<P: AsRef<Path>>(path: P) -> Result<Mesh, LoadError>`:
  - Handles both ASCII (`solid …`) and binary STL.
  - Computes per-vertex normals if the file has only per-face data
    (binary STL convention is per-face; we want per-vertex for
    smooth shading in Three.js).
  - Bounding box computed during load — drives the camera framing
    and exclusion-zone collision check in PR-2-6.

- `pub fn load_obj<P: AsRef<Path>>(path: P) -> Result<Mesh, LoadError>`:
  - Wavefront OBJ — vertices, normals, faces. Material library
    (`.mtl`) ignored for MVP; PR-2-7 can re-introduce when the
    object library cares about textures.
  - Multi-group OBJ files load as a single Mesh; the cascade
    adapter (PR-1-6) doesn't yet support per-volume extruder
    assignment from OBJ groups. (3MF carries that — PR-2-4.)

- 50 MB STL test fixture loads in < 3 seconds on the project lead's
  laptop (per Phase 2 exit criteria).

- Loader errors (truncated file, NaN coordinates, malformed face
  index) surface as typed `LoadError` variants with file:line where
  applicable. The frontend renders them in the upload UI.

- Unit tests against small fixtures (≤ 1 KB ASCII STL, ≤ 5 KB
  binary STL, ≤ 5 KB OBJ) covering: happy path, normals computed
  correctly, bounding box correct, ASCII vs binary detection,
  malformed file rejected with named error.

- Integration test: `scene_load_mesh` command with a 50 MB STL
  produces a `Mesh` + `Object` in the scene state, vs. the
  fixture's known bounding box.

**Effort.** ~3 days. Binary STL + normals computation + edge-case
input handling are the bulk; OBJ is mechanical after STL.

**Dependencies.** PR-2-1 (Mesh type), PR-2-2 (scene_load_mesh
command).

**Out of scope.** STEP loader — needs OCCT, deferred per PRD §3.2.
3MF loader — that's its own ticket (PR-2-4) because of the
project-format metadata. Materials / textures — PR-2-7 if needed.
GPU-resident mesh allocation — Three.js does that on its side.
