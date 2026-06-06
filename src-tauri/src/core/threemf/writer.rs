//! 3MF project writer.
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
//! 3D/3dmodel.model       — one <object> per scene object (solos)
//!                          plus a <components>-wrapped <object> per
//!                          multi-volume group; <build> emits one
//!                          <item> per solo + per group.
//! Metadata/model_settings.config
//!                        — per-object name + extruder hint (BBS-
//!                          flavor; OrcaSlicer accepts it too).
//!                          Groups emit one outer <object> with N
//!                          <part> children.
//! Metadata/n3o_project_settings.config
//!                        — our namespace placeholder; not yet
//!                          consumed on read.
//! ```
//!
//! We deliberately *don't* split meshes into sibling
//! `3D/Objects/object_N.model` parts the way Bambu Studio sometimes
//! does — single-file inline is simpler and OrcaSlicer reads it
//! identically. The reader handles both shapes; the writer picks
//! the simpler one.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write;
use std::path::Path;

use zip::write::SimpleFileOptions;
use zip::CompressionMethod;
use zip::ZipWriter;

use super::{Project3mf, ProjectObject};
use crate::core::scene::loaders::LoadError;
use crate::core::scene::state::{GroupId, NewMesh};

const N3O_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Write a [`Project3mf`] to `output` as a 3MF zip container.
///
/// The output is structurally equivalent to what the reader
/// (`load_3mf`) produces — re-reading the written file yields a
/// `Project3mf` with the same mesh data + objects + per-part
/// extruder hints + plate assignments (within floating-point
/// precision the writer emits).
pub fn write_3mf(project: &Project3mf, output: &Path) -> Result<(), LoadError> {
    write_3mf_with_extras(project, &std::collections::BTreeMap::new(), output)
}

/// Same as [`write_3mf`] but appends extra entries to the zip
/// container. The project-save path uses this to embed an n3o
/// JSON skeleton (`Metadata/n3o_project.json`) alongside the
/// standard 3MF geometry — foreign slicers (Bambu Studio,
/// OrcaSlicer) ignore unrecognized `Metadata/*` entries, so the
/// 3MF stays interoperable.
///
/// `extras` keys are container-relative paths (e.g.
/// `"Metadata/n3o_project.json"`); values are the raw body
/// strings. Entries colliding with the writer's own outputs (e.g.
/// `3D/3dmodel.model`) clash at zip-author time; callers should
/// pick distinct names.
pub fn write_3mf_with_extras(
    project: &Project3mf,
    extras: &std::collections::BTreeMap<String, String>,
    output: &Path,
) -> Result<(), LoadError> {
    let file = File::create(output).map_err(|e| LoadError::Io {
        path: output.into(),
        source: e,
    })?;
    let mut zip = ZipWriter::new(file);
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    write_entry(
        &mut zip,
        "[Content_Types].xml",
        &content_types_xml(),
        opts,
        output,
    )?;
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
    for (name, body) in extras {
        write_entry(&mut zip, name, body, opts, output)?;
    }

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

/// One entry in [`Layout::build_units`] — what the writer emits as
/// either a single flat `<item>` (solo) or as a `<components>`-grouped
/// `<object>` + `<item>` (group). Both [`model_xml`] and
/// [`model_settings_xml`] consume the same layout so the leaf
/// `<object id>` ↔ part `<part id>` ↔ group `<object id>` ids stay
/// consistent across the two files.
enum BuildUnit {
    /// One ProjectObject that's its own ModelObject. `object_idx` is
    /// the index into `project.objects`; the leaf resource id is
    /// `object_idx + 1`.
    Solo { object_idx: usize },
    /// >=2 ProjectObjects collapsed into one ModelObject with N
    /// > ModelVolumes. The wrapper resource id is `group_resource_id`
    /// > (allocated after all leaf ids); each member's leaf resource
    /// > id is `object_idx + 1` and is referenced by both
    /// > `<component objectid=>` (in 3dmodel.model) and
    /// > `<part id=>` (in model_settings.config).
    Group {
        group_resource_id: u32,
        member_indices: Vec<usize>,
    },
}

/// Pre-computed grouping pass used by [`model_xml`] and
/// [`model_settings_xml`]. Buckets `project.objects` by `group`:
/// `None` and single-member groups become [`BuildUnit::Solo`]; groups
/// with ≥2 members become [`BuildUnit::Group`] with a resource id
/// allocated above all leaf ids (so 3MF's "components reference
/// previously-declared objects" ordering rule holds).
struct Layout {
    build_units: Vec<BuildUnit>,
}

impl Layout {
    fn from_project(project: &Project3mf) -> Self {
        // Walk the object list once, recording the first occurrence
        // of each group and the leaf indices that belong to it.
        // Solos and unique groups are emitted in their original
        // position; multi-member groups attach to the position of
        // their first member to keep build-item order deterministic.
        let mut group_order: Vec<Option<GroupId>> = Vec::new();
        let mut group_members: std::collections::BTreeMap<GroupId, Vec<usize>> =
            std::collections::BTreeMap::new();
        for (idx, obj) in project.objects.iter().enumerate() {
            match obj.group {
                None => group_order.push(None),
                Some(gid) => {
                    let entry = group_members.entry(gid).or_default();
                    if entry.is_empty() {
                        group_order.push(Some(gid));
                    }
                    entry.push(idx);
                }
            }
        }

        // Group resource ids start above all leaf ids. Leaves use
        // `idx + 1`; the first group is `project.objects.len() + 1`.
        let mut next_group_id = project.objects.len() as u32 + 1;
        let mut build_units: Vec<BuildUnit> = Vec::with_capacity(group_order.len());
        let mut leaf_cursor = 0usize;
        for entry in group_order {
            match entry {
                None => {
                    build_units.push(BuildUnit::Solo {
                        object_idx: leaf_cursor,
                    });
                    leaf_cursor += 1;
                }
                Some(gid) => {
                    let members = group_members.remove(&gid).expect("group recorded");
                    if members.len() == 1 {
                        // A "group" of one is just a solo — emit as
                        // flat, no wrapper. Keeps the output minimal
                        // for projects that authored a unique
                        // group but only one member ended up in it.
                        build_units.push(BuildUnit::Solo {
                            object_idx: members[0],
                        });
                    } else {
                        let group_resource_id = next_group_id;
                        next_group_id += 1;
                        build_units.push(BuildUnit::Group {
                            group_resource_id,
                            member_indices: members,
                        });
                    }
                    leaf_cursor += 1;
                }
            }
        }
        Layout { build_units }
    }
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

    let layout = Layout::from_project(project);

    out.push_str(" <resources>\n");
    // Pass 1 — leaf objects (mesh-bearing). Every ProjectObject gets
    // its own <object id="N" type="model"> regardless of solo/group
    // status; group wrappers below reference these via <components>.
    //
    // Object id = `obj_idx + 1`.
    //
    // **Do not dedup meshes here.** model_settings metadata
    // (extruder hint, name) is keyed by `<object id>`, so two scene
    // objects sharing geometry still need distinct resources or
    // libslic3r collapses their metadata into one entry. Mesh-share
    // dedup would also require the build-item ↔ model-settings id
    // schemes to stay aligned across non-deterministic source
    // orderings — the previous attempt at it produced silent
    // material-color swaps between random object pairs on a 4-cube
    // 4-AMS print (HashMap iteration order vs sorted mesh-id
    // order). The on-disk mesh duplication is the price for
    // per-instance metadata correctness.
    for (obj_idx, obj) in project.objects.iter().enumerate() {
        let object_id = obj_idx as u32 + 1;
        let mesh = &project.meshes[obj.mesh_idx];
        write_object_with_mesh(&mut out, object_id, mesh);
    }
    // Pass 2 — group wrapper objects. Each is a meshless <object>
    // with a <components> child that references the group's leaf
    // objects. Per-volume world transforms live on the component
    // entries; the wrapper itself + the build item are at identity.
    // (A future "group has its own transform" refactor would factor
    // out a shared parent transform — for now this is simpler and
    // round-trip correct.)
    for unit in &layout.build_units {
        if let BuildUnit::Group {
            group_resource_id,
            member_indices,
        } = unit
        {
            out.push_str(&format!(
                "  <object id=\"{group_resource_id}\" type=\"model\">\n   <components>\n"
            ));
            for &member_idx in member_indices {
                let leaf_id = member_idx as u32 + 1;
                let xform = transform_to_3mf_string(&project.objects[member_idx].transform);
                out.push_str(&format!(
                    "    <component objectid=\"{leaf_id}\" transform=\"{xform}\"/>\n"
                ));
            }
            out.push_str("   </components>\n  </object>\n");
        }
    }
    out.push_str(" </resources>\n");

    out.push_str(" <build>\n");
    for unit in &layout.build_units {
        match unit {
            BuildUnit::Solo { object_idx } => {
                let object_id = *object_idx as u32 + 1;
                let transform = transform_to_3mf_string(&project.objects[*object_idx].transform);
                out.push_str(&format!(
                    "  <item objectid=\"{object_id}\" transform=\"{transform}\" printable=\"1\"/>\n"
                ));
            }
            BuildUnit::Group {
                group_resource_id, ..
            } => {
                // Group transform is identity — per-component
                // <component transform=> carries each volume's full
                // world placement (see Pass 2 above).
                let identity =
                    transform_to_3mf_string(&crate::core::scene::transform::Transform::IDENTITY);
                out.push_str(&format!(
                    "  <item objectid=\"{group_resource_id}\" transform=\"{identity}\" printable=\"1\"/>\n"
                ));
            }
        }
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
    for (i, tri) in mesh.indices.chunks_exact(3).enumerate() {
        // Re-emit the opaque BBS `paint_color` (MMU color-painting) string
        // for painted faces so libslic3r segments them to their filaments on
        // load. Indexed by triangle position — preserved 1:1 from read.
        let paint = mesh
            .paint_colors
            .as_ref()
            .and_then(|p| p.get(i))
            .filter(|s| !s.is_empty());
        match paint {
            Some(p) => out.push_str(&format!(
                "     <triangle v1=\"{}\" v2=\"{}\" v3=\"{}\" paint_color=\"{}\"/>\n",
                tri[0],
                tri[1],
                tri[2],
                xml_escape_attr(p),
            )),
            None => out.push_str(&format!(
                "     <triangle v1=\"{}\" v2=\"{}\" v3=\"{}\"/>\n",
                tri[0], tri[1], tri[2]
            )),
        }
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

/// Emit per-object config overrides as `<metadata key=.. value=../>` lines
/// at `indent`. On load libslic3r folds object-level metadata into
/// `ModelObject::config` and part-level into `ModelVolume::config` (same
/// channel as the `extruder` hint), so a solo object's overrides go on its
/// `<object>` and a group member's on its `<part>`.
fn push_override_metadata(out: &mut String, indent: &str, overrides: &BTreeMap<String, String>) {
    for (key, value) in overrides {
        out.push_str(&format!(
            "{indent}<metadata key=\"{}\" value=\"{}\"/>\n",
            xml_escape_attr(key),
            xml_escape_attr(value),
        ));
    }
}

fn model_settings_xml(project: &Project3mf) -> String {
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<config>\n");
    // One BBS-style outer <object> per build unit (solo or group),
    // with <part> children — one per leaf for groups, one for solos.
    // Outer <object id=> matches the resource id the build item
    // references; each <part id=> matches the leaf object's
    // <object id> in 3dmodel.model. BBS's reader uses the part id
    // (`ID_ATTR` in bbs_3mf.cpp's `_handle_start_config_volume`) to
    // correlate the part with its component subobject.
    let layout = Layout::from_project(project);
    for unit in &layout.build_units {
        match unit {
            BuildUnit::Solo { object_idx } => {
                let object_id = *object_idx as u32 + 1;
                let obj = &project.objects[*object_idx];
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
                // Per-object overrides at the <object> (ModelObject::config) level.
                push_override_metadata(&mut out, "    ", &obj.overrides);
                // Emit a single <part> too so the BBS-flavor reader
                // (which keys per-volume extruder off <part>) picks
                // up the hint even on simple single-volume objects.
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
            BuildUnit::Group {
                group_resource_id,
                member_indices,
            } => {
                out.push_str(&format!("  <object id=\"{group_resource_id}\">\n"));
                // Group-level name = first member's name as a
                // reasonable default. (Future: surface an
                // explicit group name field on Project3mf.)
                let first = &project.objects[member_indices[0]];
                out.push_str(&format!(
                    "    <metadata key=\"name\" value=\"{}\"/>\n",
                    xml_escape_attr(&first.name),
                ));
                for &member_idx in member_indices {
                    let leaf_id = member_idx as u32 + 1;
                    let obj = &project.objects[member_idx];
                    out.push_str(&format!(
                        "    <part id=\"{leaf_id}\" subtype=\"normal_part\">\n"
                    ));
                    out.push_str(&format!(
                        "      <metadata key=\"name\" value=\"{}\"/>\n",
                        xml_escape_attr(&obj.name),
                    ));
                    if let Some(extruder) = obj.extruder_id {
                        out.push_str(&format!(
                            "      <metadata key=\"extruder\" value=\"{extruder}\"/>\n"
                        ));
                    }
                    // Group member overrides at the <part> (ModelVolume::config) level.
                    push_override_metadata(&mut out, "      ", &obj.overrides);
                    out.push_str("    </part>\n");
                }
                out.push_str("  </object>\n");
            }
        }
    }
    // Plate stanza so the BBS-flavor reader populates
    // `plate_assignments`. One <model_instance> per build unit on
    // plate 1 (groups contribute one, not one-per-member).
    out.push_str("  <plate>\n");
    out.push_str("    <metadata key=\"plater_id\" value=\"1\"/>\n");
    for unit in &layout.build_units {
        let (object_id, plate_id) = match unit {
            BuildUnit::Solo { object_idx } => (
                *object_idx as u32 + 1,
                project.objects[*object_idx].plate_id,
            ),
            BuildUnit::Group {
                group_resource_id,
                member_indices,
            } => (
                *group_resource_id,
                project.objects[member_indices[0]].plate_id,
            ),
        };
        if plate_id == 1 {
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
    // `docs/dev/3mf-format-notes.md` so the round-trip stays stable
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

/// Assemble a [`Project3mf`] from a flat (meshes, objects, file
/// metadata) tuple. Pre-computes the per-plate object-index
/// listing the writer needs.
///
/// Lives here (not on `SceneState`) to keep the layering clean —
/// `core/threemf` doesn't import `core/scene` beyond the
/// Mesh/Transform types it already uses through `NewMesh`.
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
            paint_colors: None,
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
    fn paint_color_round_trips_through_write_and_read() {
        // Two triangles, one painted with an opaque BBS state string, one
        // not. The writer must re-emit `paint_color` per triangle and the
        // reader recover it 1:1 by triangle index — the MMU-painting
        // round-trip libslic3r relies on to segment painted faces.
        let mesh = NewMesh {
            vertices: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0],
            normals: vec![0.0; 12],
            indices: vec![0, 1, 2, 1, 3, 2],
            paint_colors: Some(vec!["4".to_string(), String::new()]),
            bounding_box: BoundingBox {
                min: [0.0, 0.0, 0.0],
                max: [1.0, 1.0, 0.0],
            },
            provenance: MeshProvenance::Primitive("painted".into()),
        };
        let project = project_from_objects(
            vec![mesh],
            vec![ProjectObject {
                mesh_idx: 0,
                transform: Transform::IDENTITY,
                name: "tri".into(),
                extruder_id: Some(1),
                plate_id: 1,
                group: None,
                overrides: std::collections::BTreeMap::new(),
            }],
            std::collections::BTreeMap::new(),
        );
        let path = tempfile_3mf();
        write_3mf(&project, &path).expect("write");
        let reloaded = super::super::load_3mf(&path).expect("re-read");
        let pc = reloaded.meshes[0]
            .paint_colors
            .as_ref()
            .expect("paint survived round-trip");
        assert_eq!(pc.len(), 2, "one entry per triangle");
        assert_eq!(pc[0], "4", "painted face's state preserved");
        assert_eq!(pc[1], "", "unpainted face stays unpainted");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn per_object_overrides_emit_as_object_metadata() {
        let project = project_from_objects(
            vec![one_triangle_mesh()],
            vec![ProjectObject {
                mesh_idx: 0,
                transform: Transform::translation(glam::Vec3::ZERO),
                name: "tri".into(),
                extruder_id: Some(1),
                plate_id: 1,
                group: None,
                overrides: std::collections::BTreeMap::from([(
                    "layer_height".to_string(),
                    "0.3".to_string(),
                )]),
            }],
            std::collections::BTreeMap::new(),
        );
        let xml = model_settings_xml(&project);
        assert!(
            xml.contains("<metadata key=\"layer_height\" value=\"0.3\"/>"),
            "per-object override must appear as object-level metadata:\n{xml}",
        );
    }

    #[test]
    fn per_object_overrides_round_trip_through_write_and_read() {
        // The inverse of `per_object_overrides_emit_as_object_metadata`:
        // overrides the writer emits as object metadata must come back on
        // load (bbs_meta collects them, apply_bbs_metadata merges them onto
        // the ProjectObject). Closes the Orca-import read gap end to end.
        let overrides = std::collections::BTreeMap::from([
            ("layer_height".to_string(), "0.3".to_string()),
            ("wall_loops".to_string(), "5".to_string()),
        ]);
        let project = project_from_objects(
            vec![one_triangle_mesh()],
            vec![ProjectObject {
                mesh_idx: 0,
                transform: Transform::translation(glam::Vec3::ZERO),
                name: "tri".into(),
                extruder_id: Some(1),
                plate_id: 1,
                group: None,
                overrides: overrides.clone(),
            }],
            std::collections::BTreeMap::new(),
        );
        let path = tempfile_3mf();
        write_3mf(&project, &path).expect("write");
        let reloaded = super::super::load_3mf(&path).expect("re-read");
        assert_eq!(reloaded.objects.len(), 1);
        assert_eq!(
            reloaded.objects[0].overrides, overrides,
            "per-object overrides must survive write → read",
        );
        let _ = std::fs::remove_file(&path);
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
                group: None,
                overrides: Default::default(),
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
    fn round_trips_two_objects_with_distinct_metadata_even_when_sharing_a_mesh() {
        // Per-object metadata in model_settings (extruder hint,
        // name) is keyed by `<object id="N">`, so two scene objects
        // that share mesh geometry but need distinct extruder hints
        // must each get their own `<object>` resource. The writer
        // duplicates the mesh content on disk to preserve per-
        // instance metadata correctness — see the resources-loop
        // docstring in `model_xml` for why dedup isn't safe here.
        let project = project_from_objects(
            vec![one_triangle_mesh()],
            vec![
                ProjectObject {
                    mesh_idx: 0,
                    transform: Transform::translation(glam::Vec3::new(10.0, 0.0, 0.0)),
                    name: "a".into(),
                    extruder_id: Some(1),
                    plate_id: 1,
                    group: None,
                    overrides: Default::default(),
                },
                ProjectObject {
                    mesh_idx: 0,
                    transform: Transform::translation(glam::Vec3::new(30.0, 0.0, 0.0)),
                    name: "b".into(),
                    extruder_id: Some(2),
                    plate_id: 1,
                    group: None,
                    overrides: Default::default(),
                },
            ],
            std::collections::BTreeMap::new(),
        );
        let path = tempfile_3mf();
        write_3mf(&project, &path).expect("write");
        let reloaded = super::super::load_3mf(&path).expect("re-read");

        // Two scene objects → two `<object>` resources → two meshes
        // on reload (geometry duplicated). The point of the test is
        // that the per-object metadata round-trips with the right
        // object even though both reference the same geometry.
        assert_eq!(reloaded.meshes.len(), 2);
        assert_eq!(reloaded.objects.len(), 2);
        assert_eq!(reloaded.objects[0].extruder_id, Some(1));
        assert_eq!(reloaded.objects[1].extruder_id, Some(2));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn round_trips_a_multi_volume_group() {
        // A ProjectObject pair sharing a group should round-trip
        // as one ModelObject with two ModelVolumes (BBS-style
        // <components> + <part> children) — the libslic3r "floating
        // regions" check fires per-ModelObject, so a stacked
        // multi-volume object MUST come through as one object or the
        // upper volume reads as freestanding-above-the-bed.
        let g = GroupId::fresh();
        let project = project_from_objects(
            vec![one_triangle_mesh(), one_triangle_mesh()],
            vec![
                ProjectObject {
                    mesh_idx: 0,
                    transform: Transform::IDENTITY,
                    name: "lower".into(),
                    extruder_id: Some(1),
                    plate_id: 1,
                    group: Some(g),
                    overrides: Default::default(),
                },
                ProjectObject {
                    mesh_idx: 1,
                    transform: Transform::translation(glam::Vec3::new(0.0, 0.0, 10.0)),
                    name: "upper".into(),
                    extruder_id: Some(2),
                    plate_id: 1,
                    group: Some(g),
                    overrides: Default::default(),
                },
            ],
            std::collections::BTreeMap::new(),
        );
        let path = tempfile_3mf();
        write_3mf(&project, &path).expect("write grouped");
        let reloaded = super::super::load_3mf(&path).expect("re-read");

        // Reload still flattens components back to two ProjectObjects
        // (the loader's API surfaces leaves), but the BBS metadata
        // says one outer object with two parts — so both volumes
        // share a group and the extruder hints come through in
        // document order. plate_assignments lists one outer object
        // for plate 1, not two.
        assert_eq!(reloaded.objects.len(), 2);
        assert_eq!(reloaded.objects[0].extruder_id, Some(1));
        assert_eq!(reloaded.objects[1].extruder_id, Some(2));
        assert!(
            reloaded.objects[0].group.is_some()
                && reloaded.objects[0].group == reloaded.objects[1].group,
            "both volumes should share a group on reload"
        );

        // Inspect the written XML directly — the loader flattens
        // components back to leaves so it can't tell us how many
        // outer <object> wrappers + build items the writer produced.
        // The actual fix-the-floating-volume promise is "ONE build
        // item, ONE outer model_settings object with TWO parts" —
        // that's what libslic3r sees.
        let mut zip = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
        let mut model_xml = String::new();
        std::io::Read::read_to_string(
            &mut zip.by_name("3D/3dmodel.model").unwrap(),
            &mut model_xml,
        )
        .unwrap();
        let mut settings_xml = String::new();
        std::io::Read::read_to_string(
            &mut zip.by_name("Metadata/model_settings.config").unwrap(),
            &mut settings_xml,
        )
        .unwrap();

        assert_eq!(
            model_xml.matches("<item ").count(),
            1,
            "one build item for the group, not one-per-volume",
        );
        assert_eq!(
            model_xml.matches("<components>").count(),
            1,
            "group wrapper emits a <components> element",
        );
        assert_eq!(
            model_xml.matches("<component ").count(),
            2,
            "two component references for the two volumes",
        );
        assert_eq!(
            settings_xml.matches("<object id=").count(),
            1,
            "one outer model_settings <object>, not one-per-volume",
        );
        assert_eq!(
            settings_xml.matches("<part ").count(),
            2,
            "two <part> children carrying per-volume name + extruder",
        );

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
                group: None,
                overrides: Default::default(),
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
