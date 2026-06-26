//! `.3mf` reader + writer.
//!
//! Three roles in one module:
//!
//! - **Project reader** (this file's `load_3mf`): loads a
//!   `.3mf` as a *project* — geometry, object placements, per-part
//!   extruder assignments, plate metadata — into a [`Project3mf`]
//!   that the `scene_load_3mf` Tauri command ingests into scene
//!   state.
//! - **Project writer** (`writer::write_3mf`): the inverse,
//!   producing an OrcaSlicer-compatible 3MF from a `Project3mf`.
//!   Feeds slice geometry to libslic3r (`core::slice::input`) and is
//!   the test oracle for the reader. (Native save uses `.n3o`, not
//!   3MF — see `core::project::format`.)
//! - **Sliced `.gcode.3mf` writer** (`sliced::write_sliced_3mf`):
//!   the Bambu A1 mini send-format. Embeds G-code bodies + plate
//!   metadata + thumbnails with BambuStudio-namespace metadata.
//!   Consumed by the Phase 7a driver.
//!
//! Per PRD §8.2 this module lives at `core/threemf/` — project import,
//! the slice-geometry feed, the G-code preview's `.gcode.3mf`
//! drag-drop, and the Bambu driver all take stable deps on it.
//!
//! The 3MF Core spec models geometry as a forest of `<object>`
//! resources. Each object is either a leaf `<mesh>` or a tree of
//! `<component>` references; `<build><item>` entries select the
//! roots that appear on the plate. We flatten that forest into one
//! `NewMesh` per leaf and one `ProjectObject` per build instance
//! (after recursive component expansion), composing the build-item
//! transform with each component-chain transform along the way.
//!
//! Cross-file references via the Production Extension's
//! `p:path="/3D/Objects/object_N.model"` are resolved by walking
//! sibling .model parts in the same zip.
//!
//! BBS/Orca extensions surface through `Metadata/model_settings.config`
//! (parsed) and `Metadata/project_settings.config` (preserved as a
//! raw string for now — Phase 5 will parse it for cascade-suggestion
//! UX).
//!
//! PrusaSlicer-flavor 3MF (`Slic3r_PE_model.config`) is detected and
//! rejected with a guidance message; the migration path for those
//! users is to re-export through OrcaSlicer. Worth noting that
//! BBS/Orca already understand PrusaSlicer projects on import, so
//! adopting that round-trip is one keystroke for the user.

mod bbs_meta;
mod container;
mod core_spec;
pub mod paint;
mod slice_info;
mod sliced;
mod writer;

pub use paint::{decode_dominant_states, referenced_states};
pub use sliced::{
    fixture_input, md5_hex, read_sliced_3mf, write_sliced_3mf, AmsBinding, SlicedPlate,
    SlicedPlateMetadata, SlicedPlateRead, SlicedProjectInput, SlicedRead,
};
pub use writer::{project_from_objects, write_3mf};

/// Open a `.3mf` and read a single named entry, returning `None`
/// when the entry is absent — for peeking at custom metadata (e.g.
/// detecting a foreign project's `Metadata/project_settings.config`)
/// without re-parsing the geometry.
pub fn read_3mf_extra_entry(path: &Path, entry: &str) -> Result<Option<Vec<u8>>, LoadError> {
    let mut container = container::Container::open(path)?;
    container.read_opt(entry)
}

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use crate::core::scene::loaders::{compute_bounding_box, compute_vertex_normals, LoadError};
use crate::core::scene::state::{GroupId, MeshProvenance, NewMesh};
use crate::core::scene::transform::Transform;

#[derive(Debug)]
pub struct Project3mf {
    /// Meshes extracted from the 3MF. `ProjectObject.mesh_idx`
    /// indexes into this vec.
    pub meshes: Vec<NewMesh>,
    /// One entry per `<build><item>` after recursive `<component>`
    /// expansion. Each carries its placement transform (item ×
    /// component-chain) and BBS/Orca per-part metadata if present.
    pub objects: Vec<ProjectObject>,
    /// Plate assignments, 1-based plater id → list of indices into
    /// `objects`. Empty when no `Metadata/model_settings.config` is
    /// present; in that case the caller should treat every object
    /// as belonging to plate 1.
    pub plate_assignments: BTreeMap<u32, Vec<usize>>,
    /// Best-effort printer hint from the file — currently surfaced
    /// from the file-level `<metadata name="Application">` value.
    /// MVP UI uses this only as informational text in the load
    /// confirmation dialog; the cascade resolver doesn't consume it.
    pub printer_hint: Option<String>,
    /// Raw `Metadata/project_settings.config` body if present.
    /// Phase 5 (Settings UI) parses this to suggest a matching
    /// cascade.
    pub embedded_settings: Option<String>,
    /// File-level `<metadata>` from the 3MF Core (Title, Designer,
    /// License, ...). Useful for the load-confirmation dialog and
    /// for honoring CC-BY-NC attribution requirements.
    pub file_metadata: BTreeMap<String, String>,
}

#[derive(Debug)]
pub struct ProjectObject {
    pub mesh_idx: usize,
    pub transform: Transform,
    pub name: String,
    /// Per-part extruder assignment from BBS/Orca metadata.
    /// `None` = default extruder (slicer-side decision).
    pub extruder_id: Option<u8>,
    /// Plater id this part belongs to (1-based, matches BBS).
    pub plate_id: u32,
    /// Multi-volume group identity. ProjectObjects sharing the same
    /// `Some(GroupId)` are volumes of one logical ModelObject. `None` =
    /// solo. The writer collapses each group into a single
    /// `<object>` with `<components>` (and per-volume `<part>`
    /// metadata) so libslic3r reads it as one ModelObject with N
    /// ModelVolumes, not N freestanding objects. The writer maps each
    /// `GroupId` to an integer 3MF resource id at emit time.
    pub group: Option<GroupId>,
    /// Per-object libslic3r config overrides (key → serialized value).
    /// The writer emits these as `<metadata>` in the object's (or, for a
    /// group member, the part's) `model_settings.config` stanza, where
    /// libslic3r folds them into `ModelObject`/`ModelVolume::config` on
    /// load — the same channel per-object `extruder` rides. Empty for
    /// every path except the slice-input builder, which fills it from the
    /// plate's `object_overrides` (scope-gated to object/region keys).
    pub overrides: BTreeMap<String, String>,
}

pub fn load_3mf(path: &Path) -> Result<Project3mf, LoadError> {
    let mut container = container::Container::open(path)?;

    // PrusaSlicer-flavor detection: presence of `Slic3r_PE_model.config`
    // means we'd need to read different metadata. Out of scope for
    // MVP — fail with a guidance message.
    if container
        .read_opt("Metadata/Slic3r_PE_model.config")?
        .is_some()
    {
        return Err(LoadError::Parse {
            path: path.into(),
            message: "PrusaSlicer-flavor 3MF detected — re-save through OrcaSlicer for full \
                project metadata support"
                .into(),
        });
    }

    let main_bytes = container.read("3D/3dmodel.model")?;
    let main = core_spec::parse_model(&main_bytes, path)?;

    // Collect (model_path, ModelDoc) pairs. "" represents the root
    // 3dmodel.model; sibling files keyed by their `p:path` value
    // canonicalized to lowercased no-leading-slash form so component
    // references match regardless of how BBS wrote them.
    let mut docs: HashMap<String, core_spec::ModelDoc> = HashMap::new();
    docs.insert(String::new(), main.clone());
    collect_referenced_parts(&main, &mut container, &mut docs, path)?;

    // Flatten <build><item> entries into ProjectObjects, expanding
    // <components> recursively. Mesh dedup is keyed by
    // (source_model_path, object_id) so two build items pointing at
    // the same component tree share a single NewMesh.
    let mut meshes: Vec<NewMesh> = Vec::new();
    let mut mesh_idx_by_source: HashMap<(String, u32), usize> = HashMap::new();
    let mut objects: Vec<ProjectObject> = Vec::new();

    for item in &main.build_items {
        if !item.printable {
            // Non-printable build items (e.g., assembly fixtures)
            // are skipped — they aren't part of the plate.
            continue;
        }
        flatten_build_item(
            item,
            &docs,
            &mut meshes,
            &mut mesh_idx_by_source,
            &mut objects,
            path,
        )?;
    }

    if objects.is_empty() {
        return Err(LoadError::Empty { path: path.into() });
    }

    // BBS/Orca metadata enrichment. Each part in model_settings
    // corresponds (in document order) to a leaf instance produced
    // by flattening — we apply names + extruders by walking the
    // newly-built `objects` list in the same order BBS used.
    if let Some(ms_bytes) = container.read_opt("Metadata/model_settings.config")? {
        let settings = bbs_meta::parse_model_settings(&ms_bytes, path)?;
        apply_bbs_metadata(&settings, &mut objects);
    }

    let embedded_settings = container
        .read_opt("Metadata/project_settings.config")?
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());

    let mut file_metadata = BTreeMap::new();
    for (k, v) in &main.metadata {
        file_metadata.insert(k.clone(), v.clone());
    }
    let printer_hint = file_metadata.get("Application").cloned();

    let mut plate_assignments: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    for (idx, obj) in objects.iter().enumerate() {
        plate_assignments.entry(obj.plate_id).or_default().push(idx);
    }

    Ok(Project3mf {
        meshes,
        objects,
        plate_assignments,
        printer_hint,
        embedded_settings,
        file_metadata,
    })
}

/// Walk every `<component p:path="...">` reference and load the
/// pointed-to .model side file. BBS recurses one level in practice
/// (3dmodel.model points at Objects/object_N.model, which is leaf
/// meshes only); the implementation is recursive in case the spec
/// is exercised more deeply.
fn collect_referenced_parts(
    doc: &core_spec::ModelDoc,
    container: &mut container::Container,
    docs: &mut HashMap<String, core_spec::ModelDoc>,
    source: &Path,
) -> Result<(), LoadError> {
    for obj in doc.objects.values() {
        if let core_spec::ObjectBody::Components(comps) = &obj.body {
            for c in comps {
                if let Some(part_path) = &c.path {
                    let key = canonicalize(part_path);
                    if docs.contains_key(&key) {
                        continue;
                    }
                    let bytes = container.read(part_path)?;
                    let part_doc = core_spec::parse_model(&bytes, source)?;
                    docs.insert(key.clone(), part_doc);
                    // Recurse: side files can in principle reference
                    // each other.
                    let recursed = docs.get(&key).unwrap().clone();
                    collect_referenced_parts(&recursed, container, docs, source)?;
                }
            }
        }
    }
    Ok(())
}

fn canonicalize(path_attr: &str) -> String {
    path_attr.trim_start_matches('/').to_ascii_lowercase()
}

fn flatten_build_item(
    item: &core_spec::BuildItem,
    docs: &HashMap<String, core_spec::ModelDoc>,
    meshes: &mut Vec<NewMesh>,
    mesh_idx_by_source: &mut HashMap<(String, u32), usize>,
    objects: &mut Vec<ProjectObject>,
    source: &Path,
) -> Result<(), LoadError> {
    let part_key = item.path.as_deref().map(canonicalize).unwrap_or_default();
    let item_xform = glam::Mat4::from_cols_array(&item.transform);

    expand(
        &part_key,
        item.objectid,
        item_xform,
        docs,
        meshes,
        mesh_idx_by_source,
        objects,
        source,
    )
}

#[allow(clippy::too_many_arguments)]
fn expand(
    part_key: &str,
    objectid: u32,
    accumulated: glam::Mat4,
    docs: &HashMap<String, core_spec::ModelDoc>,
    meshes: &mut Vec<NewMesh>,
    mesh_idx_by_source: &mut HashMap<(String, u32), usize>,
    objects: &mut Vec<ProjectObject>,
    source: &Path,
) -> Result<(), LoadError> {
    let doc = docs.get(part_key).ok_or_else(|| LoadError::Parse {
        path: source.into(),
        message: format!("unresolved component path: /{part_key}"),
    })?;
    let obj = doc.objects.get(&objectid).ok_or_else(|| LoadError::Parse {
        path: source.into(),
        message: format!("unknown objectid {objectid} in {part_key}"),
    })?;

    match &obj.body {
        core_spec::ObjectBody::Mesh {
            vertices,
            indices,
            paint_colors,
        } => {
            let key = (part_key.to_owned(), objectid);
            let mesh_idx = match mesh_idx_by_source.get(&key) {
                Some(&idx) => idx,
                None => {
                    if vertices.is_empty() || indices.is_empty() {
                        return Err(LoadError::Empty {
                            path: source.into(),
                        });
                    }
                    let normals = compute_vertex_normals(vertices, indices);
                    let bounding_box = compute_bounding_box(vertices);
                    let provenance = MeshProvenance::File(format!(
                        "{}#{}{}",
                        source.display(),
                        if part_key.is_empty() { "" } else { "/" },
                        if part_key.is_empty() {
                            format!("object{objectid}")
                        } else {
                            format!("{part_key}#{objectid}")
                        }
                    ));
                    let idx = meshes.len();
                    meshes.push(NewMesh {
                        vertices: vertices.clone(),
                        normals,
                        indices: indices.clone(),
                        paint_colors: (!paint_colors.is_empty()).then(|| paint_colors.clone()),
                        bounding_box,
                        provenance,
                    });
                    mesh_idx_by_source.insert(key, idx);
                    idx
                }
            };
            objects.push(ProjectObject {
                mesh_idx,
                transform: Transform::from_mat4(accumulated),
                // Names are filled in by apply_bbs_metadata when
                // model_settings is present; a stable fallback name
                // for OrcaCube-style 3MFs without that metadata
                // makes the scene legible.
                name: format!("object_{objectid}"),
                extruder_id: None,
                plate_id: 1,
                // Default to solo; apply_bbs_metadata assigns a
                // shared group when multiple leaves share one
                // outer model_settings object (= BBS multi-volume).
                group: None,
                // Filled by `apply_bbs_metadata` below from each object's
                // model_settings.config <metadata>; empty here until then.
                overrides: Default::default(),
            });
        }
        core_spec::ObjectBody::Components(comps) => {
            for c in comps {
                let child_xform = glam::Mat4::from_cols_array(&c.transform);
                let next = accumulated * child_xform;
                let next_key = c
                    .path
                    .as_deref()
                    .map(canonicalize)
                    .unwrap_or_else(|| part_key.to_owned());
                expand(
                    &next_key,
                    c.objectid,
                    next,
                    docs,
                    meshes,
                    mesh_idx_by_source,
                    objects,
                    source,
                )?;
            }
        }
    }

    Ok(())
}

fn apply_bbs_metadata(settings: &bbs_meta::ModelSettings, objects: &mut [ProjectObject]) {
    // One walk over the outer objects in document order. Each outer
    // `<object>` owns `parts.len().max(1)` consecutive flattened
    // ProjectObjects — `max(1)` because a *part-less* `<object>` (the
    // single-volume foreign shape, e.g. OrcaCube) still owns one. For each
    // owned ProjectObject we apply its identity (name/extruder) + per-object
    // config overrides + plate id + group id in the same pass, so the
    // metadata and the plate/group assignment can never desync (an earlier
    // version flattened only `<part>` entries for metadata, which silently
    // dropped a part-less object's object-level config and misaligned every
    // later object's metadata once any object lacked parts).
    //
    // Identity + config precedence: a part supplies its own name/extruder
    // and `ModelVolume::config`; a part-less object falls back to the outer
    // object's name/extruder and `ModelObject::config`. Override merge is
    // outer (shared by all parts) ∪ part, part winning — round-tripping our
    // own writer (solo overrides on `<object>`, group-member on `<part>`).
    //
    // Group identity: outer objects with >1 part are BBS multi-volume groups
    // (e.g. a cube split into upper + lower color regions); their leaf
    // ProjectObjects share a fresh group so the writer + slice path
    // collapse them back into one ModelObject with N ModelVolumes (otherwise
    // libslic3r treats each volume as freestanding and flags non-bed-touching
    // ones as "floating regions").
    let has_plate_info = !settings.plates.is_empty();
    let mut cursor = 0usize;
    for outer in &settings.objects {
        let part_count = outer.parts.len().max(1);
        let plate = if has_plate_info {
            settings
                .plates
                .iter()
                .find(|p| p.object_ids.contains(&outer.id))
                .map(|p| p.plater_id)
                .unwrap_or(1)
        } else {
            1
        };
        let group = if outer.parts.len() > 1 {
            Some(GroupId::fresh())
        } else {
            None
        };
        for offset in 0..part_count {
            let idx = cursor + offset;
            if idx >= objects.len() {
                break;
            }
            let obj = &mut objects[idx];
            // `None` for a part-less object → fall back to the outer object.
            let part = outer.parts.get(offset);

            let name = match part {
                Some(p) => p.name.as_ref(),
                None => outer.name.as_ref(),
            };
            if let Some(name) = name {
                obj.name = name.clone();
            }
            obj.extruder_id = match part {
                Some(p) => p.extruder,
                None => outer.default_extruder,
            };

            let mut merged = outer.config.clone();
            if let Some(p) = part {
                merged.extend(p.config.iter().map(|(k, v)| (k.clone(), v.clone())));
            }
            if !merged.is_empty() {
                obj.overrides = merged;
            }

            if has_plate_info {
                obj.plate_id = plate;
            }
            obj.group = group;
        }
        cursor += part_count;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn spike3_fixture() -> PathBuf {
        // Workspace-relative — cargo sets CWD to the crate dir during
        // tests, and the fixture lives under the workspace
        // `examples/spike3/` folder (one level up from src-tauri).
        let crate_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
        let mut p = PathBuf::from(crate_dir);
        p.pop();
        p.push("examples/spike3/fourcolor.3mf");
        p
    }

    fn proj_obj(mesh_idx: usize) -> ProjectObject {
        ProjectObject {
            mesh_idx,
            transform: Transform::IDENTITY,
            name: format!("placeholder_{mesh_idx}"),
            extruder_id: None,
            plate_id: 1,
            group: None,
            overrides: Default::default(),
        }
    }

    #[test]
    fn apply_bbs_metadata_keeps_partless_object_config_and_stays_aligned() {
        use bbs_meta::{ModelSettings, ObjectSettings, PartSettings};
        use std::collections::BTreeMap;

        // Object A is part-less but carries object-level config + name +
        // extruder — the single-volume foreign shape (e.g. OrcaCube, which
        // writes <object> with no <part>). Object B has one part with its own
        // config/name/extruder. The flatten yields two ProjectObjects in
        // document order. Before the unified walk, A (no part) contributed
        // nothing to the parts list, so its override was dropped AND B's part
        // metadata misaligned onto A.
        let settings = ModelSettings {
            objects: vec![
                ObjectSettings {
                    id: 1,
                    name: Some("A".into()),
                    default_extruder: Some(3),
                    config: BTreeMap::from([("layer_height".to_string(), "0.3".to_string())]),
                    parts: vec![],
                },
                ObjectSettings {
                    id: 2,
                    name: None,
                    default_extruder: None,
                    config: BTreeMap::new(),
                    parts: vec![PartSettings {
                        id: 1,
                        name: Some("Bpart".into()),
                        extruder: Some(2),
                        config: BTreeMap::from([("wall_loops".to_string(), "5".to_string())]),
                        source_object_id: None,
                    }],
                },
            ],
            plates: vec![],
        };
        let mut objects = vec![proj_obj(0), proj_obj(1)];
        apply_bbs_metadata(&settings, &mut objects);

        // A keeps its own object-level identity + override — not dropped.
        assert_eq!(objects[0].name, "A");
        assert_eq!(objects[0].extruder_id, Some(3));
        assert_eq!(
            objects[0].overrides.get("layer_height").map(String::as_str),
            Some("0.3"),
            "part-less object's object-level override must survive import",
        );
        // B's part metadata lands on objects[1], NOT misaligned onto A.
        assert_eq!(objects[1].name, "Bpart");
        assert_eq!(objects[1].extruder_id, Some(2));
        assert_eq!(
            objects[1].overrides.get("wall_loops").map(String::as_str),
            Some("5"),
        );
        assert!(
            !objects[0].overrides.contains_key("wall_loops"),
            "B's override must not bleed onto A",
        );
    }

    #[test]
    fn fourcolor_3mf_yields_eight_objects_with_extruder_pattern() {
        let path = spike3_fixture();
        if !path.exists() {
            eprintln!("skipping: fixture missing at {path:?}");
            return;
        }
        let project = load_3mf(&path).expect("load");
        assert_eq!(
            project.objects.len(),
            8,
            "4-color benchy ships 8 parts; got {}",
            project.objects.len()
        );
        // Extruders rotate 1,2,3,4,1,2,3,4.
        let extruders: Vec<Option<u8>> = project.objects.iter().map(|o| o.extruder_id).collect();
        assert_eq!(
            extruders,
            vec![
                Some(1),
                Some(2),
                Some(3),
                Some(4),
                Some(1),
                Some(2),
                Some(3),
                Some(4),
            ],
        );
        // Each part has its own mesh.
        assert_eq!(project.meshes.len(), 8);
        // All on plate 1.
        let plates: std::collections::HashSet<u32> =
            project.objects.iter().map(|o| o.plate_id).collect();
        assert_eq!(plates, std::collections::HashSet::from([1]));
        // Designer attribution survives.
        assert_eq!(
            project.file_metadata.get("Designer").map(|s| s.as_str()),
            Some("jansonne")
        );
    }

    fn two_cubes_fixture() -> PathBuf {
        // First-party fixture; generator script + binary live next
        // to each other under tests/fixtures/3mf/. See the dir's
        // README for shape + provenance.
        let crate_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
        PathBuf::from(crate_dir).join("tests/fixtures/3mf/two-cubes-2mat.3mf")
    }

    #[test]
    fn two_cubes_2mat_decodes_per_object_extruder_hints() {
        // The 2-cube fixture authored at tests/fixtures/3mf/ carries
        // a BBS-flavor `<metadata key="extruder">` on each <part>
        // (cube A = 1, cube B = 2). Pins the per-object extruder
        // hint path end-to-end through `apply_bbs_metadata` so a
        // regression there (e.g. zip-order vs document-order
        // confusion) surfaces immediately. Companion regression
        // tests for the auto-bind that consumes these values live
        // in `core::project::mutation::tests::auto_bind_*`.
        let project = load_3mf(&two_cubes_fixture()).expect("load 2-cube fixture");
        assert_eq!(project.objects.len(), 2, "two build items");
        let extruders: Vec<Option<u8>> = project.objects.iter().map(|o| o.extruder_id).collect();
        assert_eq!(extruders, vec![Some(1), Some(2)]);
        let names: Vec<&str> = project.objects.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, vec!["Cube A (T0)", "Cube B (T1)"]);
    }

    fn four_cubes_fixture() -> PathBuf {
        let crate_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
        PathBuf::from(crate_dir).join("tests/fixtures/3mf/four-cubes-4mat.3mf")
    }

    #[test]
    fn four_cubes_4mat_decodes_per_object_extruder_hints() {
        // Sibling to the 2-cube fixture but at the U1's full
        // toolchanger width. Specifically exercises the case where
        // every model material maps to a distinct physical
        // toolhead; the 2-cube version can't catch a future bug
        // that miscounts beyond 2.
        let project = load_3mf(&four_cubes_fixture()).expect("load 4-cube fixture");
        assert_eq!(project.objects.len(), 4, "four build items");
        let extruders: Vec<Option<u8>> = project.objects.iter().map(|o| o.extruder_id).collect();
        assert_eq!(extruders, vec![Some(1), Some(2), Some(3), Some(4)]);
        let names: Vec<&str> = project.objects.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["Cube A (T0)", "Cube B (T1)", "Cube C (T2)", "Cube D (T3)"],
        );
    }

    fn cube_halves_fixture() -> PathBuf {
        let crate_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
        PathBuf::from(crate_dir).join("tests/fixtures/3mf/cube-halves-2mat.3mf")
    }

    #[test]
    fn cube_halves_2mat_decodes_multi_volume_extruder_hints() {
        // Multi-volume single-object shape: one outer <object> with
        // two <part> children, geometry side uses <components> to
        // group two leaf meshes. Confirms the loader expands the
        // component chain and zips the per-part extruder hints
        // against the resulting two ProjectObjects in document order.
        let project = load_3mf(&cube_halves_fixture()).expect("load cube-halves fixture");
        assert_eq!(project.objects.len(), 2, "two volumes inside one group");
        let extruders: Vec<Option<u8>> = project.objects.iter().map(|o| o.extruder_id).collect();
        assert_eq!(extruders, vec![Some(1), Some(2)]);
        let names: Vec<&str> = project.objects.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, vec!["Lower half (M1)", "Upper half (M2)"]);
        // Both volumes land on plate 1.
        let plates: std::collections::HashSet<u32> =
            project.objects.iter().map(|o| o.plate_id).collect();
        assert_eq!(plates, std::collections::HashSet::from([1]));
        // And both volumes share a group — they're parts of one
        // logical ModelObject. Without grouping, the writer would
        // emit them as freestanding objects and libslic3r would
        // flag the upper half as a "floating region" needing
        // supports.
        let group_a = project.objects[0].group;
        let group_b = project.objects[1].group;
        assert!(group_a.is_some(), "lower half should have a group");
        assert_eq!(group_a, group_b, "both volumes belong to the same group");
    }

    #[test]
    fn two_cubes_2mat_leaves_solos_ungrouped() {
        // Sanity guard for the loader: when each outer model_settings
        // <object> has exactly one <part>, the leaves are solo —
        // group stays None and the writer emits them as
        // freestanding objects (today's flat shape).
        let project = load_3mf(&two_cubes_fixture()).expect("load 2-cube fixture");
        assert_eq!(project.objects.len(), 2);
        for obj in &project.objects {
            assert!(
                obj.group.is_none(),
                "{} should be solo, got group={:?}",
                obj.name,
                obj.group,
            );
        }
    }

    fn orcacube_fixture() -> PathBuf {
        let crate_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
        let mut p = PathBuf::from(crate_dir);
        p.pop();
        p.push("external/OrcaSlicer/resources/handy_models/OrcaCube_v2.3mf");
        p
    }

    #[test]
    fn orcacube_v2_yields_two_objects() {
        let path = orcacube_fixture();
        if !path.exists() {
            eprintln!("skipping: fixture missing at {path:?}");
            return;
        }
        let project = load_3mf(&path).expect("load");
        // OrcaCube_v2.3mf packages OrcaCube + OrcaPlug as separate
        // build items.
        assert_eq!(project.objects.len(), 2);
        // No BBS-flavor per-part extruder data; defaults to None.
        for obj in &project.objects {
            assert!(obj.extruder_id.is_none());
        }
        assert!(!project.meshes.is_empty());
    }

    #[test]
    fn truncated_zip_errors_with_path() {
        let mut tmp = tempfile::NamedTempFile::with_suffix(".3mf").expect("tempfile");
        std::io::Write::write_all(&mut tmp, b"not actually a zip").expect("write");
        let err = load_3mf(tmp.path()).expect_err("malformed zip");
        match err {
            LoadError::Parse { path, message } => {
                assert_eq!(path, tmp.path());
                assert!(
                    message.contains("zip"),
                    "expected zip-flavored parse error, got: {message}"
                );
            }
            other => panic!("expected Parse error, got {other:?}"),
        }
    }

    #[test]
    fn prusaslicer_flavor_3mf_rejected_with_guidance() {
        // Build a minimal zip with the Prusa metadata marker.
        let path = tempfile::NamedTempFile::with_suffix(".3mf")
            .expect("tempfile")
            .into_temp_path();
        {
            let f = std::fs::File::create(&path).expect("create");
            let mut zip = zip::ZipWriter::new(f);
            let opts: zip::write::FileOptions<()> = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            zip.start_file("Metadata/Slic3r_PE_model.config", opts)
                .unwrap();
            std::io::Write::write_all(&mut zip, b"<config/>").unwrap();
            zip.start_file("3D/3dmodel.model", opts).unwrap();
            std::io::Write::write_all(
                &mut zip,
                b"<model xmlns=\"http://schemas.microsoft.com/3dmanufacturing/core/2015/02\"/>",
            )
            .unwrap();
            zip.finish().unwrap();
        }
        let err = load_3mf(&path).expect_err("prusa flavor");
        match err {
            LoadError::Parse { message, .. } => {
                assert!(
                    message.contains("PrusaSlicer"),
                    "expected guidance message, got: {message}"
                );
            }
            other => panic!("expected Parse error, got {other:?}"),
        }
    }
}
