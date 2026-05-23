//! 3MF project writer (PR-3-9).
//!
//! Inverse of the reader in `mod.rs`: given a [`Project3mf`] (or any
//! data shaped like one), emit a valid 3MF Core spec container that
//! OrcaSlicer / Bambu Studio can open and that the reader here
//! round-trips structurally.
//!
//! Layout we emit (matches OrcaSlicer's minimum-viable shape):
//!
//! ```text
//! [Content_Types].xml
//! _rels/.rels
//! 3D/3dmodel.model       — one <object> per mesh + <build> with
//!                          one <item> per scene object
//! Metadata/model_settings.config
//!                        — per-object name + extruder hint (BBS-flavor;
//!                          OrcaSlicer accepts it too)
//! Metadata/n3o_project_settings.config
//!                        — our namespace placeholder. Phase 5
//!                          populates with cascade overrides etc.
//! ```
//!
//! We deliberately *don't* split meshes into sibling
//! `3D/Objects/object_N.model` parts the way Bambu Studio sometimes
//! does — single-file inline is simpler and OrcaSlicer reads it
//! identically. The reader handles both shapes; the writer picks
//! the simpler one.

use std::fs::File;
use std::io::Write;
use std::path::Path;

use zip::write::SimpleFileOptions;
use zip::CompressionMethod;
use zip::ZipWriter;

use super::{Project3mf, ProjectObject};
use crate::core::scene::loaders::LoadError;
use crate::core::scene::state::NewMesh;

const N3O_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Write a [`Project3mf`] to `output` as a 3MF zip container.
///
/// The output is structurally equivalent to what the reader
/// (`load_3mf`) produces — re-reading the written file yields a
/// `Project3mf` with the same mesh data + objects + per-part
/// extruder hints + plate assignments (within floating-point
/// precision the writer emits).
pub fn write_3mf(project: &Project3mf, output: &Path) -> Result<(), LoadError> {
    let file = File::create(output).map_err(|e| LoadError::Io {
        path: output.into(),
        source: e,
    })?;
    let mut zip = ZipWriter::new(file);
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    write_entry(&mut zip, "[Content_Types].xml", &content_types_xml(), opts, output)?;
    write_entry(&mut zip, "_rels/.rels", &rels_xml(), opts, output)?;
    write_entry(
        &mut zip,
        "3D/3dmodel.model",
        &model_xml(project),
        opts,
        output,
    )?;
    write_entry(
        &mut zip,
        "Metadata/model_settings.config",
        &model_settings_xml(project),
        opts,
        output,
    )?;
    write_entry(
        &mut zip,
        "Metadata/n3o_project_settings.config",
        &n3o_settings_xml(),
        opts,
        output,
    )?;

    zip.finish().map_err(|e| LoadError::Parse {
        path: output.into(),
        message: format!("finalize zip: {e}"),
    })?;
    Ok(())
}

fn write_entry(
    zip: &mut ZipWriter<File>,
    name: &str,
    body: &str,
    opts: SimpleFileOptions,
    output: &Path,
) -> Result<(), LoadError> {
    zip.start_file(name, opts).map_err(|e| LoadError::Parse {
        path: output.into(),
        message: format!("start_file {name}: {e}"),
    })?;
    zip.write_all(body.as_bytes()).map_err(|e| LoadError::Io {
        path: output.into(),
        source: e,
    })?;
    Ok(())
}

fn content_types_xml() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
 <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
 <Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/>
 <Default Extension="config" ContentType="application/vnd.ms-package.3dmanufacturing-config+xml"/>
</Types>
"#
    .into()
}

fn rels_xml() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
 <Relationship Target="/3D/3dmodel.model" Id="rel-1" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/>
</Relationships>
"#
    .into()
}

fn model_xml(project: &Project3mf) -> String {
    let mut out = String::with_capacity(8 * 1024 + project.meshes.len() * 1024);
    out.push_str(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <model unit=\"millimeter\" xml:lang=\"en-US\" \
         xmlns=\"http://schemas.microsoft.com/3dmanufacturing/core/2015/02\">\n",
    );

    // File-level metadata. Application is emitted from us so that
    // round-tripping through Bambu Studio / OrcaSlicer (which lower-
    // cases nothing) shows the user where the file came from.
    out.push_str(&format!(
        " <metadata name=\"Application\">n3o-slic3r-{N3O_VERSION}</metadata>\n"
    ));
    // Pass-through file_metadata that the reader extracted. Each
    // value is XML-encoded.
    for (k, v) in &project.file_metadata {
        if k == "Application" {
            continue;
        }
        out.push_str(&format!(
            " <metadata name=\"{}\">{}</metadata>\n",
            xml_escape_attr(k),
            xml_escape_text(v),
        ));
    }

    out.push_str(" <resources>\n");
    // One <object> per mesh, with an inline <mesh>. We assign
    // object ids 1..=meshes.len(). The build items below reference
    // these ids per scene object.
    for (idx, mesh) in project.meshes.iter().enumerate() {
        let object_id = idx as u32 + 1;
        write_object_with_mesh(&mut out, object_id, mesh);
    }
    out.push_str(" </resources>\n");

    out.push_str(" <build>\n");
    for obj in &project.objects {
        let object_id = obj.mesh_idx as u32 + 1;
        let transform = transform_to_3mf_string(&obj.transform);
        out.push_str(&format!(
            "  <item objectid=\"{object_id}\" transform=\"{transform}\" printable=\"1\"/>\n"
        ));
    }
    out.push_str(" </build>\n");

    out.push_str("</model>\n");
    out
}

fn write_object_with_mesh(out: &mut String, object_id: u32, mesh: &NewMesh) {
    out.push_str(&format!(
        "  <object id=\"{object_id}\" type=\"model\">\n   <mesh>\n    <vertices>\n"
    ));
    for chunk in mesh.vertices.chunks_exact(3) {
        // Emit at 6 decimals — sufficient for the project's
        // float-precision floor. The reader parses as f32 so
        // anything beyond 7 sig figs is round-trip noise.
        out.push_str(&format!(
            "     <vertex x=\"{:.6}\" y=\"{:.6}\" z=\"{:.6}\"/>\n",
            chunk[0], chunk[1], chunk[2]
        ));
    }
    out.push_str("    </vertices>\n    <triangles>\n");
    for tri in mesh.indices.chunks_exact(3) {
        out.push_str(&format!(
            "     <triangle v1=\"{}\" v2=\"{}\" v3=\"{}\"/>\n",
            tri[0], tri[1], tri[2]
        ));
    }
    out.push_str("    </triangles>\n   </mesh>\n  </object>\n");
}

/// Emit a Transform as the 12-float 3MF transform attribute. Reader's
/// `parse_transform_attr` is the inverse — see
/// `core_spec.rs` for the column-major-with-last-column-omitted
/// convention.
fn transform_to_3mf_string(t: &crate::core::scene::transform::Transform) -> String {
    let m = t.matrix;
    // glam column-major: cols (a,b,c), (d,e,f), (g,h,i), (tx,ty,tz).
    // 3MF format: "a b c d e f g h i tx ty tz".
    let a = m[0];
    let b = m[1];
    let c = m[2];
    let d = m[4];
    let e = m[5];
    let f = m[6];
    let g = m[8];
    let h = m[9];
    let i = m[10];
    let tx = m[12];
    let ty = m[13];
    let tz = m[14];
    format!("{a} {b} {c} {d} {e} {f} {g} {h} {i} {tx} {ty} {tz}")
}

fn model_settings_xml(project: &Project3mf) -> String {
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<config>\n");
    // For now we emit one BBS-style <object> per scene object so
    // the per-object name + extruder hint round-trips. Multi-volume
    // (<part>) emission is a Phase 5 concern when multi-material
    // projects start authoring inside this app.
    for (obj_idx, obj) in project.objects.iter().enumerate() {
        let object_id = obj_idx as u32 + 1;
        out.push_str(&format!("  <object id=\"{object_id}\">\n"));
        out.push_str(&format!(
            "    <metadata key=\"name\" value=\"{}\"/>\n",
            xml_escape_attr(&obj.name),
        ));
        if let Some(extruder) = obj.extruder_id {
            out.push_str(&format!(
                "    <metadata key=\"extruder\" value=\"{extruder}\"/>\n"
            ));
        }
        // Emit a single <part> too so the BBS-flavor reader (which
        // looks at parts for per-volume extruder) picks up the
        // hint even on simple single-volume objects. Multi-volume
        // emission is a Phase 5 concern.
        out.push_str("    <part id=\"1\" subtype=\"normal_part\">\n");
        out.push_str(&format!(
            "      <metadata key=\"name\" value=\"{}\"/>\n",
            xml_escape_attr(&obj.name),
        ));
        if let Some(extruder) = obj.extruder_id {
            out.push_str(&format!(
                "      <metadata key=\"extruder\" value=\"{extruder}\"/>\n"
            ));
        }
        out.push_str("    </part>\n");
        out.push_str("  </object>\n");
    }
    // Plate stanza so the BBS-flavor reader populates
    // `plate_assignments`. Phase 5 wires per-plate object lists.
    out.push_str("  <plate>\n");
    out.push_str("    <metadata key=\"plater_id\" value=\"1\"/>\n");
    for (obj_idx, obj) in project.objects.iter().enumerate() {
        if obj.plate_id == 1 {
            let object_id = obj_idx as u32 + 1;
            out.push_str("    <model_instance>\n");
            out.push_str(&format!(
                "      <metadata key=\"object_id\" value=\"{object_id}\"/>\n"
            ));
            out.push_str("    </model_instance>\n");
        }
    }
    out.push_str("  </plate>\n");
    out.push_str("</config>\n");
    out
}

fn n3o_settings_xml() -> String {
    // Placeholder for Phase 5 cascade overrides + plate-printer
    // bindings + project-wide metadata. The reader doesn't
    // consume this yet; Phase 5 will. Schema documented in
    // `docs/3mf-format-notes.md` so the round-trip stays stable
    // across writer revisions.
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <n3o_project version=\"1\" writer=\"n3o-slic3r-{N3O_VERSION}\">\n\
         </n3o_project>\n",
    )
}

/// XML-encode a string for use inside an element's text content.
fn xml_escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

/// XML-encode a string for use inside a double-quoted attribute value.
fn xml_escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

/// Used by tests + Phase 5 to assemble a `Project3mf` from a live
/// scene. Pulled out so the writer doesn't depend on `SceneState`
/// directly (keeps the layering clean — `core/threemf` doesn't
/// import `core/scene` beyond the Mesh/Transform types it already
/// uses through `NewMesh`).
pub fn project_from_objects(
    meshes: Vec<NewMesh>,
    objects: Vec<ProjectObject>,
    file_metadata: std::collections::BTreeMap<String, String>,
) -> Project3mf {
    let mut plate_assignments: std::collections::BTreeMap<u32, Vec<usize>> = Default::default();
    for (idx, obj) in objects.iter().enumerate() {
        plate_assignments.entry(obj.plate_id).or_default().push(idx);
    }
    Project3mf {
        meshes,
        objects,
        plate_assignments,
        printer_hint: file_metadata.get("Application").cloned(),
        embedded_settings: None,
        file_metadata,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::printer::profile::BoundingBox;
    use crate::core::scene::state::{MeshProvenance, NewMesh};
    use crate::core::scene::transform::Transform;
    use std::path::PathBuf;

    fn one_triangle_mesh() -> NewMesh {
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

    fn tempfile_3mf() -> PathBuf {
        let dir = std::env::temp_dir();
        dir.join(format!(
            "n3o-test-{}.3mf",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ))
    }

    #[test]
    fn round_trips_a_single_triangle_project() {
        let project = project_from_objects(
            vec![one_triangle_mesh()],
            vec![ProjectObject {
                mesh_idx: 0,
                transform: Transform::translation(glam::Vec3::new(10.0, 20.0, 0.0)),
                name: "tri".into(),
                extruder_id: Some(2),
                plate_id: 1,
            }],
            std::collections::BTreeMap::new(),
        );
        let path = tempfile_3mf();
        write_3mf(&project, &path).expect("write");
        let reloaded = super::super::load_3mf(&path).expect("re-read");

        assert_eq!(reloaded.meshes.len(), 1);
        assert_eq!(reloaded.objects.len(), 1);
        assert_eq!(reloaded.objects[0].extruder_id, Some(2));
        assert_eq!(reloaded.objects[0].name, "tri");
        // Translation lives at column-major indices 12/13/14.
        let tx = reloaded.objects[0].transform.matrix;
        assert!((tx[12] - 10.0).abs() < 1e-4);
        assert!((tx[13] - 20.0).abs() < 1e-4);
        assert!(tx[14].abs() < 1e-4);
        assert_eq!(reloaded.plate_assignments.get(&1).map(|v| v.len()), Some(1));
        // Vertex data round-trips (float precision is the only
        // possible drift; we emit 6 decimals so 0/1 stay exact).
        assert_eq!(reloaded.meshes[0].vertices, project.meshes[0].vertices);
        assert_eq!(reloaded.meshes[0].indices, project.meshes[0].indices);

        // Clean up.
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn round_trips_two_objects_sharing_a_mesh() {
        let project = project_from_objects(
            vec![one_triangle_mesh()],
            vec![
                ProjectObject {
                    mesh_idx: 0,
                    transform: Transform::translation(glam::Vec3::new(10.0, 0.0, 0.0)),
                    name: "a".into(),
                    extruder_id: Some(1),
                    plate_id: 1,
                },
                ProjectObject {
                    mesh_idx: 0,
                    transform: Transform::translation(glam::Vec3::new(30.0, 0.0, 0.0)),
                    name: "b".into(),
                    extruder_id: Some(2),
                    plate_id: 1,
                },
            ],
            std::collections::BTreeMap::new(),
        );
        let path = tempfile_3mf();
        write_3mf(&project, &path).expect("write");
        let reloaded = super::super::load_3mf(&path).expect("re-read");

        // Both build items point at the same `<object>` so the
        // reader dedupes to one mesh.
        assert_eq!(reloaded.meshes.len(), 1);
        assert_eq!(reloaded.objects.len(), 2);
        assert_eq!(reloaded.objects[0].extruder_id, Some(1));
        assert_eq!(reloaded.objects[1].extruder_id, Some(2));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn round_trips_file_metadata() {
        let mut meta = std::collections::BTreeMap::new();
        meta.insert("Title".to_owned(), "test project".to_owned());
        meta.insert("Designer".to_owned(), "n3o team & friends".to_owned());
        let project = project_from_objects(
            vec![one_triangle_mesh()],
            vec![ProjectObject {
                mesh_idx: 0,
                transform: Transform::IDENTITY,
                name: "tri".into(),
                extruder_id: None,
                plate_id: 1,
            }],
            meta,
        );
        let path = tempfile_3mf();
        write_3mf(&project, &path).expect("write");
        let reloaded = super::super::load_3mf(&path).expect("re-read");

        assert_eq!(
            reloaded.file_metadata.get("Title").map(|s| s.as_str()),
            Some("test project"),
        );
        // The reader should preserve the ampersand round-trip via
        // the XML escape in the writer.
        assert_eq!(
            reloaded.file_metadata.get("Designer").map(|s| s.as_str()),
            Some("n3o team & friends"),
        );
        let _ = std::fs::remove_file(&path);
    }
}
