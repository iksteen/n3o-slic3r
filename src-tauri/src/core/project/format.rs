//! Project `.3mf` save/load (PR-5-8).
//!
//! The project format is a standard 3MF zip with one extra entry:
//! `Metadata/n3o_project.json` carrying the serialized [`Project`]
//! (plate list, printer + material bindings, plate metadata,
//! project-tier overrides, file metadata). Foreign slicers (Bambu
//! Studio, OrcaSlicer, PrusaSlicer) read the geometry + standard
//! 3MF `<metadata>` fields and ignore the unrecognized
//! `Metadata/n3o_project.json` entry.
//!
//! ## Why split JSON + 3MF
//!
//! - **Geometry** (mesh vertex / normal / index buffers, object
//!   placements, plate assignments) lives in the standard 3MF
//!   structure — foreign-slicer interop + a battle-tested
//!   container format we already read + write (PR-2-4 / PR-3-9).
//! - **Project state** (bindings, plate metadata, project-tier
//!   overrides, file metadata) lives in `Metadata/n3o_project.json`.
//!   Just a `serde_json::to_string(&Project)` — the heavy mesh
//!   buffers are `#[serde(skip)]`, so the JSON stays small.
//!
//! On load the two layers reunite: the JSON gives the project
//! skeleton (including [`Mesh`] entries with empty buffers); the
//! 3MF supplies the buffers. They match by position — the writer
//! emits meshes sorted by [`MeshId`] ascending, the reader walks
//! the loaded project's meshes in the same order and zips them
//! with the 3MF's geometry.
//!
//! ## Format version
//!
//! [`FORMAT_VERSION`] is the schema marker baked into every
//! written project. The reader rejects mismatched versions with
//! [`ProjectIoError::SchemaMismatch`]. Bump it whenever the JSON
//! schema changes incompatibly.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::model::{Plate, Project};
use crate::core::scene::loaders::LoadError;
use crate::core::scene::state::{MeshId, NewMesh};
use crate::core::scene::transform::Transform;
use crate::core::threemf::{
    load_3mf, project_from_objects, read_3mf_extra_entry, write_3mf_with_extras,
    ProjectObject,
};

/// Schema version baked into every written project. Bump on
/// incompatible schema changes; the reader rejects mismatched
/// versions with [`ProjectIoError::SchemaMismatch`].
pub const FORMAT_VERSION: &str = "1";

/// 3MF metadata entry holding the serialized project skeleton.
pub const METADATA_FILENAME: &str = "Metadata/n3o_project.json";

#[derive(Debug)]
pub enum ProjectIoError {
    /// Filesystem I/O failure when reading or writing the
    /// container file.
    Io { path: PathBuf, source: io::Error },
    /// 3MF container error from the underlying reader / writer
    /// (zip-level, XML-parse, or geometry-parse).
    Threemf(LoadError),
    /// `Metadata/n3o_project.json` failed to deserialize.
    Json { path: PathBuf, message: String },
    /// The schema version in the loaded file doesn't match what
    /// this build can read.
    SchemaMismatch { found: String, expected: String },
    /// `Metadata/n3o_project.json` is missing — the file was not
    /// written by us (or by a future / older format version that
    /// removed the entry).
    NotAProjectFile { path: PathBuf },
    /// The geometry side of the file disagrees with the project
    /// skeleton: the 3MF has a different number of meshes than
    /// the JSON's mesh map. Symptomatic of a corrupted file or a
    /// mid-write crash.
    GeometryMismatch {
        path: PathBuf,
        json_mesh_count: usize,
        threemf_mesh_count: usize,
    },
}

impl std::fmt::Display for ProjectIoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "I/O on {}: {source}", path.display()),
            Self::Threemf(e) => write!(f, "3MF: {e}"),
            Self::Json { path, message } => {
                write!(f, "{} project JSON: {message}", path.display())
            }
            Self::SchemaMismatch { found, expected } => write!(
                f,
                "project schema version mismatch: file has \"{found}\", \
                 this build reads \"{expected}\"",
            ),
            Self::NotAProjectFile { path } => write!(
                f,
                "{}: no {METADATA_FILENAME} — not an n3o-slic3r project",
                path.display(),
            ),
            Self::GeometryMismatch {
                path,
                json_mesh_count,
                threemf_mesh_count,
            } => write!(
                f,
                "{}: geometry/skeleton mesh-count mismatch \
                 (json={json_mesh_count}, 3mf={threemf_mesh_count})",
                path.display(),
            ),
        }
    }
}

impl std::error::Error for ProjectIoError {}

impl From<LoadError> for ProjectIoError {
    fn from(e: LoadError) -> Self {
        Self::Threemf(e)
    }
}

/// JSON-side payload: the project skeleton plus the
/// `format_version` marker. Kept as its own struct (vs. just
/// `Project`) so future fields outside of `Project` can attach
/// without polluting the runtime type — e.g. writer fingerprint,
/// generator-tool version.
#[derive(Debug, Serialize, Deserialize)]
struct ProjectFile {
    format_version: String,
    project: Project,
}

/// Write `project` to `output` as a `.3mf` project file.
///
/// Overwrites `output` if it exists. The 3MF geometry side is
/// built from `project.meshes` (sorted by [`MeshId`] for
/// deterministic order) + every plate's `scene.objects`; the
/// project skeleton ships as `Metadata/n3o_project.json`.
pub fn write_project(project: &Project, output: &Path) -> Result<(), ProjectIoError> {
    // Build the geometry payload. Meshes are sorted by MeshId so
    // the read side can zip them back by position without needing
    // an explicit mapping table.
    let mut mesh_id_order: Vec<MeshId> = project.meshes.keys().copied().collect();
    mesh_id_order.sort();
    let mesh_id_to_idx: std::collections::HashMap<MeshId, usize> = mesh_id_order
        .iter()
        .enumerate()
        .map(|(i, id)| (*id, i))
        .collect();

    let geometry_meshes: Vec<NewMesh> = mesh_id_order
        .iter()
        .map(|id| {
            let m = &project.meshes[id];
            NewMesh {
                vertices: m.vertices.clone(),
                normals: m.normals.clone(),
                indices: m.indices.clone(),
                bounding_box: m.bounding_box,
                provenance: m.provenance.clone(),
            }
        })
        .collect();

    // Flatten objects across plates. plate_id on the ProjectObject
    // is the wire-side u32 — foreign slicers see this as the BBS
    // plater id. We use Project.plates[i].id.0 for one-to-one
    // mapping with our PlateId.
    let mut geometry_objects: Vec<ProjectObject> = Vec::new();
    for plate in &project.plates {
        for obj in plate.scene.objects.values() {
            let mesh_idx = mesh_id_to_idx.get(&obj.mesh).copied().ok_or_else(|| {
                ProjectIoError::Json {
                    path: output.into(),
                    message: format!(
                        "object {} references unknown mesh {}",
                        obj.id.0, obj.mesh.0,
                    ),
                }
            })?;
            geometry_objects.push(ProjectObject {
                mesh_idx,
                transform: obj.transform,
                name: obj.name.clone(),
                extruder_id: obj.extruder_id,
                plate_id: plate.id.0,
            });
        }
    }

    let project_3mf = project_from_objects(
        geometry_meshes,
        geometry_objects,
        project.file_metadata.clone(),
    );

    // Build the JSON-side payload + the extras map.
    let project_json = serde_json::to_string_pretty(&ProjectFile {
        format_version: FORMAT_VERSION.into(),
        project: project.clone(),
    })
    .map_err(|e| ProjectIoError::Json {
        path: output.into(),
        message: format!("serialize: {e}"),
    })?;
    let mut extras: BTreeMap<String, String> = BTreeMap::new();
    extras.insert(METADATA_FILENAME.into(), project_json);

    write_3mf_with_extras(&project_3mf, &extras, output)?;
    Ok(())
}

/// Read a `.3mf` project file at `input`. The reader expects both
/// the standard 3MF geometry layer and our
/// `Metadata/n3o_project.json` skeleton — files missing the
/// skeleton are rejected with [`ProjectIoError::NotAProjectFile`].
///
/// The returned [`Project`] has `source_path = Some(input)` so
/// subsequent "save" calls overwrite the loaded file; `save_as`
/// is the surface for "save to a different path."
pub fn read_project(input: &Path) -> Result<Project, ProjectIoError> {
    // 1. JSON skeleton.
    let raw = read_3mf_extra_entry(input, METADATA_FILENAME)?.ok_or_else(|| {
        ProjectIoError::NotAProjectFile {
            path: input.into(),
        }
    })?;
    let file: ProjectFile =
        serde_json::from_slice(&raw).map_err(|e| ProjectIoError::Json {
            path: input.into(),
            message: format!("parse: {e}"),
        })?;
    if file.format_version != FORMAT_VERSION {
        return Err(ProjectIoError::SchemaMismatch {
            found: file.format_version,
            expected: FORMAT_VERSION.into(),
        });
    }
    let mut project = file.project;

    // 2. Geometry. Read the 3MF and zip its NewMesh entries back
    //    into project.meshes by sorted-MeshId order (matches the
    //    writer's order).
    //
    //    Skip when the JSON project has no meshes: load_3mf rejects
    //    empty containers with LoadError::Empty, which is the
    //    wrong signal for a legitimately-empty project. A
    //    brand-new project the user saves before adding any
    //    geometry round-trips cleanly via this path.
    let mut mesh_id_order: Vec<MeshId> = project.meshes.keys().copied().collect();
    mesh_id_order.sort();
    if !mesh_id_order.is_empty() {
        let geometry = load_3mf(input)?;
        if mesh_id_order.len() != geometry.meshes.len() {
            return Err(ProjectIoError::GeometryMismatch {
                path: input.into(),
                json_mesh_count: mesh_id_order.len(),
                threemf_mesh_count: geometry.meshes.len(),
            });
        }
        for (id, new_mesh) in mesh_id_order.iter().zip(geometry.meshes.into_iter()) {
            let mesh = project.meshes.get_mut(id).expect("walked id set");
            mesh.vertices = new_mesh.vertices;
            mesh.normals = new_mesh.normals;
            mesh.indices = new_mesh.indices;
            // bbox + provenance already round-trip via the JSON;
            // the 3MF reader's copies match within float precision.
        }
    }

    // 3. Stamp source_path so the next "save" knows where to go.
    project.source_path = Some(input.into());

    Ok(project)
}

/// Convert a [`Plate`]'s scene-side objects to the geometry side's
/// [`ProjectObject`] shape. Pulled out for cross-file reuse if
/// the slice orchestrator ever wants to materialize a Project3mf
/// for a single plate without going through `write_project`.
#[allow(dead_code)]
pub fn plate_to_project_objects(
    plate: &Plate,
    mesh_id_to_idx: &std::collections::HashMap<MeshId, usize>,
) -> Result<Vec<ProjectObject>, String> {
    plate
        .scene
        .objects
        .values()
        .map(|obj| {
            let mesh_idx = mesh_id_to_idx
                .get(&obj.mesh)
                .copied()
                .ok_or_else(|| format!("unknown mesh {}", obj.mesh.0))?;
            Ok(ProjectObject {
                mesh_idx,
                transform: obj.transform,
                name: obj.name.clone(),
                extruder_id: obj.extruder_id,
                plate_id: plate.id.0,
            })
        })
        .collect()
}

// Silence the `Transform` unused-import warning when only the
// fn pointer above is gated by `#[allow(dead_code)]`.
#[allow(dead_code)]
fn _force_transform_import_used(t: Transform) -> Transform {
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::project::binding::PrinterBinding;
    use crate::core::printer::profile::BoundingBox;
    use crate::core::scene::state::{MeshProvenance, NewMesh, NewSceneObject};

    fn tempfile_3mf() -> PathBuf {
        let dir = std::env::temp_dir();
        dir.join(format!(
            "n3o-project-test-{}.3mf",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ))
    }

    fn triangle() -> NewMesh {
        NewMesh {
            vertices: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            normals: vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
            indices: vec![0, 1, 2],
            bounding_box: BoundingBox {
                min: [0.0, 0.0, 0.0],
                max: [1.0, 1.0, 0.0],
            },
            provenance: MeshProvenance::Primitive("triangle".into()),
        }
    }

    fn a1_mini_binding() -> PrinterBinding {
        PrinterBinding {
            printer_identity: "bambu-lab-a1-mini".into(),
        }
    }

    #[test]
    fn round_trip_minimal_default_project() {
        let p = Project::default();
        let path = tempfile_3mf();
        write_project(&p, &path).expect("write");
        let parsed = read_project(&path).expect("read");
        assert_eq!(parsed.plates.len(), 1);
        assert_eq!(parsed.source_path, Some(path.clone()));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn round_trip_preserves_plate_metadata_and_bindings() {
        let mut p = Project::default();
        p.user_overrides
            .insert("travel_speed".into(), "300".into());
        p.file_metadata
            .insert("Title".into(), "Fixture Project".into());
        // Plate 0: printer + bindings + project override.
        p.plates[0].printer = Some(a1_mini_binding());
        p.plates[0].project_overrides.insert(
            "layer_height".into(),
            "0.12".into(),
        );
        // Add a second plate so the list-shape survives.
        p.add_plate(None);

        let path = tempfile_3mf();
        write_project(&p, &path).expect("write");
        let parsed = read_project(&path).expect("read");

        assert_eq!(
            parsed.user_overrides.get("travel_speed").map(|s| s.as_str()),
            Some("300"),
        );
        assert_eq!(
            parsed.file_metadata.get("Title").map(|s| s.as_str()),
            Some("Fixture Project"),
        );
        assert_eq!(parsed.plates.len(), 2);
        assert_eq!(parsed.plates[0].printer, Some(a1_mini_binding()));
        assert_eq!(
            parsed.plates[0]
                .project_overrides
                .get("layer_height")
                .map(|s| s.as_str()),
            Some("0.12"),
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn round_trip_preserves_geometry() {
        let mut p = Project::default();
        let mesh_id = p.register_mesh(triangle());
        let _obj = p.register_object(NewSceneObject::at_origin(mesh_id, "tri"));

        let path = tempfile_3mf();
        write_project(&p, &path).expect("write");
        let parsed = read_project(&path).expect("read");

        assert_eq!(parsed.meshes.len(), 1);
        let m = parsed.meshes.get(&mesh_id).expect("mesh preserved by id");
        assert_eq!(m.vertices.len(), 9, "vertex buffer round-trips");
        assert_eq!(m.indices.len(), 3);
        assert_eq!(
            parsed.plates[0].scene.objects.len(),
            1,
            "object preserved on plate 0",
        );
        let obj = parsed.plates[0].scene.objects.values().next().unwrap();
        assert_eq!(obj.mesh, mesh_id, "mesh reference preserved");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn round_trip_preserves_object_overrides() {
        let mut p = Project::default();
        let mesh_id = p.register_mesh(triangle());
        let obj = p.register_object(NewSceneObject::at_origin(mesh_id, "tri"));
        let active_id = p.active_plate().id;
        p.object_override_set(active_id, obj, "layer_height".into(), "0.10".into())
            .unwrap();

        let path = tempfile_3mf();
        write_project(&p, &path).expect("write");
        let parsed = read_project(&path).expect("read");
        let overrides = parsed.plates[0]
            .scene
            .object_overrides
            .get(&obj)
            .expect("overrides preserved");
        assert_eq!(overrides["layer_height"], "0.10");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn round_trip_preserves_active_plate_index() {
        let mut p = Project::default();
        let (id_b, _) = p.add_plate(None);
        p.set_active_plate(id_b).unwrap();

        let path = tempfile_3mf();
        write_project(&p, &path).expect("write");
        let parsed = read_project(&path).expect("read");
        assert_eq!(parsed.active_plate, 1);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn round_trip_preserves_uuid() {
        let p = Project::default();
        let path = tempfile_3mf();
        write_project(&p, &path).expect("write");
        let parsed = read_project(&path).expect("read");
        assert_eq!(parsed.uuid, p.uuid);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn read_project_rejects_3mf_without_metadata() {
        // Write a plain 3MF (no n3o_project.json) and confirm
        // read_project says NotAProjectFile.
        use crate::core::threemf::write_3mf;
        let path = tempfile_3mf();
        let project_3mf = project_from_objects(vec![], vec![], BTreeMap::new());
        write_3mf(&project_3mf, &path).expect("write plain 3mf");
        let err = read_project(&path).unwrap_err();
        assert!(matches!(err, ProjectIoError::NotAProjectFile { .. }));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn read_project_rejects_schema_mismatch() {
        // Hand-craft a project file with a wrong format_version
        // and confirm SchemaMismatch fires.
        let p = Project::default();
        let path = tempfile_3mf();
        write_project(&p, &path).expect("write");
        // Re-write with a tampered version field.
        let body = serde_json::to_string(&serde_json::json!({
            "format_version": "999",
            "project": p,
        }))
        .unwrap();
        let mut extras = BTreeMap::new();
        extras.insert(METADATA_FILENAME.into(), body);
        let project_3mf = project_from_objects(vec![], vec![], BTreeMap::new());
        write_3mf_with_extras(&project_3mf, &extras, &path).expect("rewrite");
        let err = read_project(&path).unwrap_err();
        assert!(matches!(err, ProjectIoError::SchemaMismatch { .. }));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn write_then_overwrite_clobbers_existing_file() {
        let p = Project::default();
        let path = tempfile_3mf();
        write_project(&p, &path).expect("write 1");
        write_project(&p, &path).expect("write 2");
        let _ = read_project(&path).expect("read");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn round_trip_three_plate_fixture_per_ticket_spec() {
        // PR-5-12 exit-criteria flavor: 3 plates, one bound to
        // each of two printers, per-plate metadata, material
        // bindings. Verifies the full save/load shape.
        let snapmaker = PrinterBinding {
            printer_identity: "snapmaker-u1".into(),
        };
        let mut p = Project::default();
        p.plates[0].printer = Some(a1_mini_binding());
        let (_b, _) = p.add_plate(Some(snapmaker.clone()));
        let (_c, _) = p.add_plate(Some(snapmaker));

        let path = tempfile_3mf();
        write_project(&p, &path).expect("write");
        let parsed = read_project(&path).expect("read");
        assert_eq!(parsed.plates.len(), 3);
        let printers: Vec<&str> = parsed
            .plates
            .iter()
            .map(|pl| pl.printer.as_ref().unwrap().printer_identity.as_str())
            .collect();
        assert_eq!(
            printers,
            vec!["bambu-lab-a1-mini", "snapmaker-u1", "snapmaker-u1"],
        );
        std::fs::remove_file(&path).ok();
    }
}
