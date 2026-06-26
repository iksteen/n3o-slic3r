//! Native project save/load — the `.n3o` container.
//!
//! A `.n3o` file is a plain zip with our own entries:
//!
//! - `project.json` — `serde_json` of the [`Project`] (plates, bindings,
//!   material maps, overrides, groups, **objects with stable ids**, and `Mesh`
//!   headers). The heavy vertex/index/paint buffers are `#[serde(skip)]`,
//!   so the JSON stays small. Wrapped in a [`ProjectFile`] that adds the
//!   `format_version` marker + the writing build's stamp.
//! - `geometry/<MeshId>.bin` — one tight binary blob per mesh, carrying its
//!   buffers. Geometry is keyed by `MeshId`: an object references its mesh by id,
//!   and load fills that mesh's buffers from its blob. Shared geometry (cloned
//!   objects → one `MeshId`) is one blob shared by all.
//!
//! Foreign Bambu/Orca `.3mf` projects are imported through a separate path
//! ([`crate::core::threemf::load_3mf`] + the importer); `read_project` detects a
//! `.3mf` handed to it and returns [`ProjectIoError::ForeignProject`] so the
//! "open project" surface routes it to the importer.
//!
//! Derived / transient state is not stored: the bed + exclusion zones (a pure
//! function of the bound printer) are re-derived on load via
//! `Plate::set_printer`; the live selection and `source_path` are
//! `#[serde(skip)]`. Cascade overrides persist as **logical** keys (the adapter
//! owns the libslic3r translation), so the file isn't coupled to option names.

use std::fs::File;
use std::io::{self, Read, Seek, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use super::model::Project;
use crate::core::scene::state::{Mesh, MeshId};

/// Schema version of the `.n3o` container. The reader rejects any other version
/// with [`ProjectIoError::SchemaMismatch`]. Bump on incompatible changes.
///
/// `"2"` dropped the per-vertex `normals` chunk that `"1"` geometry blobs
/// carried — nothing consumes stored normals (the renderer and libslic3r
/// recompute them). `"1"` files are not read; they fail with a clean version
/// mismatch rather than being silently mis-deserialized.
pub const FORMAT_VERSION: &str = "2";

/// The zip entry holding the serialized project skeleton.
const PROJECT_ENTRY: &str = "project.json";

#[derive(Debug)]
pub enum ProjectIoError {
    /// Filesystem / zip I/O failure reading or writing the container.
    Io { path: PathBuf, source: io::Error },
    /// `project.json` failed to (de)serialize.
    Json { path: PathBuf, message: String },
    /// The container's `format_version` doesn't match this build.
    SchemaMismatch { found: String, expected: String },
    /// Not an n3o project and not a recognizable foreign 3MF either.
    NotAProjectFile { path: PathBuf },
    /// A foreign OrcaSlicer / Bambu Studio `.3mf` — openable via the importer,
    /// not as a native project. Distinguished so the UI can route it there.
    ForeignProject { path: PathBuf },
}

impl std::fmt::Display for ProjectIoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "I/O on {}: {source}", path.display()),
            Self::Json { path, message } => {
                write!(f, "{}: {message}", path.display())
            }
            Self::SchemaMismatch { found, expected } => write!(
                f,
                "project schema version mismatch: file has \"{found}\", \
                 this build reads \"{expected}\"",
            ),
            Self::NotAProjectFile { path } => {
                write!(f, "{}: not an n3o-slic3r project", path.display())
            }
            Self::ForeignProject { path } => write!(
                f,
                "{} is an OrcaSlicer / Bambu Studio project, not a native n3o \
                 project — open it via the importer.",
                path.file_name()
                    .map(|n| n.to_string_lossy())
                    .unwrap_or_default(),
            ),
        }
    }
}

impl std::error::Error for ProjectIoError {}

/// `project.json` payload: the serialized project plus the version marker and
/// the writing build's stamp. Borrowing variant for write (no `Project` clone),
/// owning variant for read.
#[derive(Serialize)]
struct ProjectFileRef<'a> {
    format_version: &'a str,
    app_name: &'a str,
    app_version: &'a str,
    project: &'a Project,
}

#[derive(Deserialize)]
struct ProjectFile {
    format_version: String,
    #[serde(default)]
    #[allow(dead_code)]
    app_name: String,
    #[serde(default)]
    #[allow(dead_code)]
    app_version: String,
    project: Project,
}

/// Write `project` to `output` as a `.n3o` file (overwrites if it exists).
pub fn write_project(project: &Project, output: &Path) -> Result<(), ProjectIoError> {
    let file = File::create(output).map_err(|e| ProjectIoError::Io {
        path: output.into(),
        source: e,
    })?;
    let mut zip = ZipWriter::new(file);
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    let project_json = serde_json::to_vec_pretty(&ProjectFileRef {
        format_version: FORMAT_VERSION,
        app_name: env!("CARGO_PKG_NAME"),
        app_version: env!("CARGO_PKG_VERSION"),
        project,
    })
    .map_err(|e| ProjectIoError::Json {
        path: output.into(),
        message: format!("serialize: {e}"),
    })?;
    zip_write(&mut zip, PROJECT_ENTRY, &project_json, opts, output)?;

    // One geometry blob per mesh, keyed by MeshId. Objects reference meshes by
    // id, so load resolves buffers by id — no ordering involved.
    for (id, mesh) in &project.meshes {
        let blob = pack_geometry(mesh).map_err(|e| ProjectIoError::Json {
            path: output.into(),
            message: format!("encode geometry for mesh {}: {e}", id.0),
        })?;
        zip_write(&mut zip, &format!("geometry/{}.bin", id.0), &blob, opts, output)?;
    }
    zip.finish().map_err(|e| ProjectIoError::Json {
        path: output.into(),
        message: format!("finalize zip: {e}"),
    })?;
    Ok(())
}

/// Read a `.n3o` project at `input`.
///
/// A `.3mf` handed here returns [`ProjectIoError::ForeignProject`] (route to the
/// importer); anything else without our `project.json` is `NotAProjectFile`.
pub fn read_project(input: &Path) -> Result<Project, ProjectIoError> {
    let file = File::open(input).map_err(|e| ProjectIoError::Io {
        path: input.into(),
        source: e,
    })?;
    let mut zip = ZipArchive::new(file).map_err(|_| ProjectIoError::NotAProjectFile {
        path: input.into(),
    })?;

    // Our container has project.json; a foreign 3MF has 3D/3dmodel.model.
    let raw = match read_zip_entry(&mut zip, PROJECT_ENTRY) {
        Some(bytes) => bytes,
        None => {
            let is_3mf = zip.by_name("3D/3dmodel.model").is_ok();
            return Err(if is_3mf {
                ProjectIoError::ForeignProject { path: input.into() }
            } else {
                ProjectIoError::NotAProjectFile { path: input.into() }
            });
        }
    };

    let file: ProjectFile = serde_json::from_slice(&raw).map_err(|e| ProjectIoError::Json {
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

    // Fill each mesh's buffers from its geometry blob.
    let ids: Vec<MeshId> = project.meshes.keys().copied().collect();
    for id in ids {
        let blob = read_zip_entry(&mut zip, &format!("geometry/{}.bin", id.0)).ok_or_else(|| {
            ProjectIoError::Json {
                path: input.into(),
                message: format!("missing geometry for mesh {}", id.0),
            }
        })?;
        let g: GeometryBlob =
            postcard::from_bytes(&blob).map_err(|e| ProjectIoError::Json {
                path: input.into(),
                message: format!("geometry for mesh {}: {e}", id.0),
            })?;
        let mesh = project.meshes.get_mut(&id).expect("id from keys");
        mesh.vertices = g.vertices;
        mesh.indices = g.indices;
        mesh.paint_colors = g.paint_colors;
    }

    // Re-derive the bed + exclusion zones we don't persist (pure function of the
    // bound printer profile) through the one binding path. An instance/profile
    // that no longer resolves leaves the plate bound-but-bedless.
    for plate in &mut project.plates {
        let Some(instance_id) = plate.printer_instance_id().map(str::to_owned) else {
            continue;
        };
        let Some(instance) = crate::core::printer::lookup_instance(&instance_id) else {
            continue;
        };
        let Some(profile) = crate::core::printer::lookup(&instance.vendor_profile_ref) else {
            continue;
        };
        plate.set_printer(Some(instance_id), Some(&profile));
    }

    project.source_path = Some(input.into());
    Ok(project)
}

// ── zip helpers ────────────────────────────────────────────────

fn zip_write(
    zip: &mut ZipWriter<File>,
    name: &str,
    body: &[u8],
    opts: SimpleFileOptions,
    output: &Path,
) -> Result<(), ProjectIoError> {
    zip.start_file(name, opts).map_err(|e| ProjectIoError::Json {
        path: output.into(),
        message: format!("start_file {name}: {e}"),
    })?;
    zip.write_all(body).map_err(|e| ProjectIoError::Io {
        path: output.into(),
        source: e,
    })?;
    Ok(())
}

fn read_zip_entry<R: Read + Seek>(zip: &mut ZipArchive<R>, name: &str) -> Option<Vec<u8>> {
    let mut f = zip.by_name(name).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    Some(buf)
}

// ── geometry blob ──────────────────────────────────────────────
//
// One per mesh, serialized with postcard (compact binary serde). Borrowed
// shape for writing (no buffer copy), owned for reading.

#[derive(Serialize)]
struct GeometryBlobRef<'a> {
    vertices: &'a [f32],
    indices: &'a [u32],
    paint_colors: &'a Option<Vec<String>>,
}

#[derive(Deserialize)]
struct GeometryBlob {
    vertices: Vec<f32>,
    indices: Vec<u32>,
    paint_colors: Option<Vec<String>>,
}

fn pack_geometry(m: &Mesh) -> Result<Vec<u8>, postcard::Error> {
    postcard::to_allocvec(&GeometryBlobRef {
        vertices: &m.vertices,
        indices: &m.indices,
        paint_colors: &m.paint_colors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::printer::profile::BoundingBox;
    use crate::core::scene::state::{MeshProvenance, NewMesh, NewSceneObject, ObjectId};
    use crate::core::scene::transform::Transform;

    fn tmp() -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("n3o-fmt-{}-{nanos}.n3o", std::process::id()))
    }

    fn triangle() -> NewMesh {
        NewMesh {
            vertices: vec![0.0, 0.0, 0.0, 10.0, 0.0, 0.0, 0.0, 10.0, 0.0],
            indices: vec![0, 1, 2],
            paint_colors: None,
            bounding_box: BoundingBox {
                min: [0.0, 0.0, 0.0],
                max: [10.0, 10.0, 0.0],
            },
            provenance: MeshProvenance::Primitive("tri".into()),
        }
    }

    fn marked_triangle(marker: f32) -> NewMesh {
        let mut t = triangle();
        t.vertices[0] = marker;
        t
    }

    #[test]
    fn pack_unpack_geometry_round_trips() {
        let mut p = Project::default();
        let mut nm = triangle();
        nm.paint_colors = Some(vec!["".into(), "3".into(), "12".into()]);
        let id = p.register_mesh(nm);
        let blob = pack_geometry(&p.meshes[&id]).expect("pack");
        let g: GeometryBlob = postcard::from_bytes(&blob).expect("unpack");
        assert_eq!(g.vertices, p.meshes[&id].vertices);
        assert_eq!(g.indices, p.meshes[&id].indices);
        assert_eq!(g.paint_colors, Some(vec!["".into(), "3".into(), "12".into()]));
        // truncated blob is a clean error, not a panic.
        assert!(postcard::from_bytes::<GeometryBlob>(&blob[..3]).is_err());
    }

    #[test]
    fn round_trip_minimal_default_project() {
        let p = Project::default();
        let path = tmp();
        write_project(&p, &path).expect("write");
        let parsed = read_project(&path).expect("read");
        assert_eq!(parsed.plates.len(), 1);
        assert_eq!(parsed.uuid, p.uuid);
        assert_eq!(parsed.source_path, Some(path.clone()));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn round_trip_geometry_buffers_and_paint() {
        let mut p = Project::default();
        let mut nm = triangle();
        nm.paint_colors = Some(vec!["4".into()]);
        let mesh_id = p.register_mesh(nm);
        let obj = p.register_object(NewSceneObject::at_origin(mesh_id, "tri"));

        let path = tmp();
        write_project(&p, &path).expect("write");
        let parsed = read_project(&path).expect("read");

        // Ids are stable across save/load — resolve directly.
        assert!(parsed.plates[0].scene.objects.contains_key(&obj));
        let m = &parsed.meshes[&mesh_id];
        assert_eq!(m.vertices.len(), 9);
        assert_eq!(m.indices, vec![0, 1, 2]);
        assert_eq!(m.paint_colors, Some(vec!["4".into()]));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn round_trip_shared_mesh_stays_shared() {
        let mut p = Project::default();
        let mesh_id = p.register_mesh(marked_triangle(77.0));
        let a = p.register_object(NewSceneObject::at_origin(mesh_id, "orig"));
        let b = p.register_object(NewSceneObject::at_origin(mesh_id, "copy"));

        let path = tmp();
        write_project(&p, &path).expect("write");
        let parsed = read_project(&path).expect("read");

        assert_eq!(parsed.meshes.len(), 1, "one distinct mesh, one blob");
        assert_eq!(parsed.plates[0].scene.objects[&a].mesh, mesh_id);
        assert_eq!(parsed.plates[0].scene.objects[&b].mesh, mesh_id);
        assert_eq!(parsed.meshes[&mesh_id].vertices[0], 77.0);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn round_trip_object_overrides() {
        // Overrides serialize directly in project.json (logical keys) — no gate,
        // no FFI, ids stable.
        let mut p = Project::default();
        let mesh_id = p.register_mesh(triangle());
        let obj = p.register_object(NewSceneObject::at_origin(mesh_id, "tri"));
        let plate = p.active_plate().id;
        p.object_override_set(plate, obj, "layer_height".into(), "0.10".into())
            .unwrap();

        let path = tmp();
        write_project(&p, &path).expect("write");
        let parsed = read_project(&path).expect("read");

        assert_eq!(
            parsed.plates[0].scene.object_overrides[&obj]["layer_height"],
            "0.10",
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn round_trip_groups_and_names() {
        let mut p = Project::default();
        let mesh_id = p.register_mesh(triangle());
        let a = p.register_object(NewSceneObject::at_origin(mesh_id, "lower"));
        let b = p.register_object(NewSceneObject::at_origin(mesh_id, "upper"));
        p.group_objects(&[a, b], "Bracket".into()).unwrap();
        let gid = p.active_plate().scene.objects[&a].group.unwrap();

        let path = tmp();
        write_project(&p, &path).expect("write");
        let parsed = read_project(&path).expect("read");

        let plate = &parsed.plates[0];
        assert_eq!(plate.scene.objects[&a].group, Some(gid));
        assert_eq!(plate.scene.objects[&b].group, Some(gid));
        assert_eq!(plate.scene.groups[&gid].name, "Bracket");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn round_trip_visibility() {
        let mut p = Project::default();
        let m = p.register_mesh(triangle());
        let shown = p.register_object(NewSceneObject::at_origin(m, "shown"));
        let hidden = p.register_object(NewSceneObject::at_origin(m, "hidden"));
        p.plates[0].scene.objects.get_mut(&hidden).unwrap().visible = false;

        let path = tmp();
        write_project(&p, &path).expect("write");
        let parsed = read_project(&path).expect("read");

        assert!(parsed.plates[0].scene.objects[&shown].visible);
        assert!(!parsed.plates[0].scene.objects[&hidden].visible);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn round_trip_multi_plate_and_bindings() {
        let mut p = Project::default();
        p.user_overrides.insert("travel_speed".into(), "300".into());
        p.file_metadata.insert("Title".into(), "Fixture".into());
        p.plates[0].set_printer(Some("bambi".into()), None);
        p.plates[0]
            .project_overrides
            .insert("layer_height".into(), "0.12".into());
        let m = p.register_mesh(triangle());
        p.register_object(NewSceneObject::at_origin(m, "on1"));
        let (id2, _) = p.add_plate(None);
        p.set_active_plate(id2).unwrap();
        p.register_object(NewSceneObject::at_origin(m, "on2"));
        p.set_active_plate(p.plates[0].id).unwrap();

        let path = tmp();
        write_project(&p, &path).expect("write");
        let parsed = read_project(&path).expect("read");

        assert_eq!(parsed.plates.len(), 2);
        assert_eq!(parsed.active_plate, 0);
        assert_eq!(parsed.plates[0].scene.objects.len(), 1);
        assert_eq!(parsed.plates[1].scene.objects.len(), 1);
        assert_eq!(parsed.plates[0].printer_instance_id(), Some("bambi"));
        assert_eq!(parsed.plates[0].project_overrides["layer_height"], "0.12");
        assert_eq!(parsed.user_overrides["travel_speed"], "300");
        assert_eq!(parsed.file_metadata["Title"], "Fixture");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn derived_and_transient_state_not_in_project_json() {
        let p = Project::default();
        let path = tmp();
        write_project(&p, &path).expect("write");
        let file = File::open(&path).unwrap();
        let mut zip = ZipArchive::new(file).unwrap();
        let raw = read_zip_entry(&mut zip, PROJECT_ENTRY).unwrap();
        let json = String::from_utf8(raw).unwrap();
        for absent in ["\"bed\"", "\"exclusion_zones\"", "\"selection\"", "\"source_path\""] {
            assert!(!json.contains(absent), "{absent} must not be persisted");
        }
        assert!(json.contains("\"format_version\""));
        assert!(json.contains(env!("CARGO_PKG_VERSION")));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn read_project_flags_a_foreign_3mf() {
        use crate::core::threemf::write_3mf;
        // A plain 3MF (no project.json) → ForeignProject so the UI routes it to
        // the importer.
        let proj3mf = crate::core::threemf::project_from_objects(
            vec![triangle()],
            vec![crate::core::threemf::ProjectObject {
                mesh_idx: 0,
                transform: Transform::IDENTITY,
                name: "x".into(),
                extruder_id: None,
                plate_id: 1,
                group: None,
                overrides: Default::default(),
            }],
            std::collections::BTreeMap::new(),
        );
        let path = tmp();
        write_3mf(&proj3mf, &path).expect("write 3mf");
        let err = read_project(&path).unwrap_err();
        assert!(matches!(err, ProjectIoError::ForeignProject { .. }), "got {err:?}");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn read_project_rejects_non_zip() {
        let path = tmp();
        std::fs::write(&path, b"not a zip").unwrap();
        let err = read_project(&path).unwrap_err();
        assert!(matches!(err, ProjectIoError::NotAProjectFile { .. }));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn read_project_rejects_schema_mismatch() {
        // A container whose project.json declares a future version.
        let body = serde_json::to_vec(&serde_json::json!({
            "format_version": "999",
            "project": Project::default(),
        }))
        .unwrap();
        let path = tmp();
        {
            let f = File::create(&path).unwrap();
            let mut zip = ZipWriter::new(f);
            let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
            zip.start_file(PROJECT_ENTRY, opts).unwrap();
            zip.write_all(&body).unwrap();
            zip.finish().unwrap();
        }
        let err = read_project(&path).unwrap_err();
        assert!(matches!(err, ProjectIoError::SchemaMismatch { .. }));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn write_then_overwrite_clobbers() {
        let p = Project::default();
        let path = tmp();
        write_project(&p, &path).expect("write 1");
        write_project(&p, &path).expect("write 2");
        read_project(&path).expect("read");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn load_keeps_object_ids_stable() {
        let mut p = Project::default();
        let m = p.register_mesh(triangle());
        let a = p.register_object(NewSceneObject::at_origin(m, "a"));
        let _ = ObjectId(0); // import sanity
        let path = tmp();
        write_project(&p, &path).expect("write");
        let parsed = read_project(&path).expect("read");
        assert!(parsed.plates[0].scene.objects.contains_key(&a), "object id is stable");
        std::fs::remove_file(&path).ok();
    }
}
