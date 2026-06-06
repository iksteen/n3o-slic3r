# PR-5-8 — Project `.3mf` save/load (extended namespace)

Status: ❌ open.

**Scope.** Persist the full project state (plates, printer
bindings, plate metadata, material bindings, project-tier
overrides, file metadata) to a `.3mf` file, and reload byte-
equivalent on the other side. The format extends the standard
3MF namespace with n3o-slic3r-specific metadata; foreign
slicers (Bambu Studio, OrcaSlicer, PrusaSlicer) read the
geometry + standard fields and ignore our extensions.

Owns FR-MP-4. Reuses PR-3-9's `write_3mf` + PR-2-4's
`load_3mf` for the geometry round-trip; this ticket adds the
project-metadata layer.

**Acceptance criteria.**

- New `core/project/format.rs`:
  - `write_project(project: &Project, path: &Path) ->
     Result<(), ProjectIoError>` — wraps PR-3-9's
    `write_3mf` for the geometry, then writes
    `Metadata/n3o_project.json` containing the serialized
    project (plates + bindings + metadata + overrides).
  - `read_project(path: &Path) -> Result<Project,
     ProjectIoError>` — calls `load_3mf` for the geometry,
    then reads + parses the project metadata. Returns a
    populated `Project` with `source_path` set.
  - Round-trip invariant: `read_project(write_project(p))
     == p` for every fixture project.

- New 3MF metadata file: `Metadata/n3o_project.json`. Schema
  versioning via a `"format_version"` field at the top
  level (start at "1"); future format changes bump it.
  Document the schema in `docs/dev/3mf-format-notes.md` under
  a new "n3o-slic3r project extensions" section.

- Namespace decision: use `n3o-slic3r` as the metadata
  namespace prefix. Per the PRD §11 (open questions),
  this is "extend the standard 3MF metadata namespace
  for project state." The metadata key under the standard
  3MF `<metadata>` element format is
  `n3o-slic3r:project-format-version`.

- Tauri commands:
  - `project_save(path: String) -> Result<(), String>` —
    overwrites; emits `project:saved { path }`.
  - `project_load(path: String) -> Result<Project, String>`
    — replaces the in-memory project state; emits
    `project:loaded { path }` so frontend re-syncs.
  - `project_save_as(path: String) -> Result<(), String>`
    — same as `project_save` but also updates
    `Project.source_path`.

- File-dialog hooks via `tauri-plugin-dialog`: the
  frontend opens a save / load dialog and passes the
  picked path through.

- Tests:
  - 3-plate fixture (A1 mini + U1 + U1) with per-plate
    cycle counts + material bindings + project overrides
    + object overrides round-trips byte-equivalent in
    the JSON metadata.
  - Geometry round-trips structurally (PR-2-4 / PR-3-9
    contract — mesh counts + object counts + plate
    assignments preserved).
  - Foreign-slicer compatibility: write a project,
    open + re-save through Bambu Studio (manual test
    documented), assert the geometry + standard 3MF
    fields are preserved and our metadata is ignored
    (it survives the round-trip if BBS doesn't strip
    `<metadata>` elements; otherwise it drops cleanly).

**Effort.** ~3 days. The serde wrapper is fast; the
3MF metadata file plumbing + the round-trip tests +
the foreign-slicer compatibility check are the bulk.

**Dependencies.** PR-5-1 (project types serializable),
PR-3-9 (`write_3mf`), PR-2-4 (`load_3mf`).

**Out of scope.** Cascade-file embedding inside the
project `.3mf` (the cascade lives separately in
`profiles/cascades/`; the project references its
identity, not its content). Send-format wrapping for
specific printers (Phase 7a/b). Project sharing /
import-from-URL (Phase 9).

**Cut candidate.** None — this is the FR-MP-4
requirement; can't ship Phase 5 without it.

**Design reference.** No mockup counterpart — the file
format work is invisible to the user. The 3MF
extension pattern follows OrcaSlicer / Bambu Studio
themselves (look at how they extend the standard 3MF
metadata with their own keys).
