//! `.3mf` reader + sliced-`.gcode.3mf` writer.
//!
//! Two roles in one module:
//!
//! - **Project reader** (this file's `load_3mf`): loads a
//!   `.3mf` as a *project* — geometry, object placements, per-part
//!   extruder assignments, plate metadata — into a [`Project3mf`]
//!   that the `scene_load_3mf` Tauri command ingests into scene
//!   state.
//! - **Sliced `.gcode.3mf` writer** (`sliced::write_sliced_3mf`):
//!   the Bambu A1 mini send-format. Embeds G-code bodies + plate
//!   metadata + thumbnails with BambuStudio-namespace metadata.
//!   Consumed by the Phase 7a driver.
//!
//! Per PRD §8.2 this module lives at `core/threemf/` — project import,
//! the G-code preview's `.gcode.3mf` drag-drop, and the Bambu driver
//! all take stable deps on it.
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
//! PrusaSlicer / Slic3r PE 3MF geometry imports too: its `3D/3dmodel.model`
//! is standard 3MF core. Object display names are lifted from
//! `Metadata/Slic3r_PE_model.config` ([`bbs_meta::parse_prusa_object_names`]),
//! and a small geometry-intent subset of `Metadata/Slic3r_PE.config` (shell
//! counts, walls, infill — [`bbs_meta::parse_prusa_geometry_overrides`]) rides
//! in as per-object overrides so a shell-only model prints as designed. The
//! rest of the foreign print profile isn't adopted — this is a model import,
//! not a full project import (that path still wants a BBS/Orca
//! `project_settings.config`).

mod bbs_meta;
mod container;
mod core_spec;
pub mod paint;
mod slice_info;
mod sliced;

pub use paint::{decode_dominant_states, referenced_states};
pub use sliced::{
    fixture_input, md5_hex, read_sliced_3mf, write_sliced_3mf, AmsBinding, SlicedPlate,
    SlicedPlateMetadata, SlicedPlateRead, SlicedProjectInput, SlicedRead,
};

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

use crate::core::scene::loaders::{compute_bounding_box, LoadError};
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
    /// The outer `<build><item objectid>` this leaf flattened from —
    /// the same id BBS `model_settings.config` keys its `<object id>`
    /// on. `apply_bbs_metadata` correlates by this rather than by
    /// position, because BBS lists objects in a different order than
    /// `<build>` does (so a positional walk mis-assigns names).
    pub source_object_id: u32,
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
    } else if let Some(pe_bytes) = container.read_opt("Metadata/Slic3r_PE_model.config")? {
        // PrusaSlicer / Slic3r PE. Lift object display names from the per-object
        // metadata (applied by `source_object_id`, matching `<object id>`)...
        let names = bbs_meta::parse_prusa_object_names(&pe_bytes, path)?;
        // ...plus a small geometry-intent subset of the global print config
        // (shell counts, walls, infill) as per-object overrides, so a model
        // designed around e.g. 0 top/bottom shells prints as intended without
        // adopting the whole foreign profile. Prusa's config is print-global, so
        // the same values fan out onto every imported object.
        let geom_overrides = container
            .read_opt("Metadata/Slic3r_PE.config")?
            .map(|b| bbs_meta::parse_prusa_geometry_overrides(&b))
            .unwrap_or_default();
        for obj in objects.iter_mut() {
            if let Some(name) = names.get(&obj.source_object_id) {
                obj.name = name.clone();
            }
            for (k, v) in &geom_overrides {
                obj.overrides.insert(k.clone(), v.clone());
            }
        }
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
    // The outer build-item objectid, held constant through recursion so
    // every leaf carries the id BBS metadata keys on (see
    // `ProjectObject::source_object_id`). `objectid` itself changes as we
    // descend into components; this does not.
    outer_id: u32,
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
            support_paint,
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
                        indices: indices.clone(),
                        paint_colors: (!paint_colors.is_empty()).then(|| paint_colors.clone()),
                        support_paint: (!support_paint.is_empty()).then(|| support_paint.clone()),
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
                source_object_id: outer_id,
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
                    outer_id,
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
    // Correlate metadata to geometry by **object id**, not position: BBS
    // writes `model_settings.config` objects in a different order than
    // `<build>` lists items, so a positional walk mis-assigns names (e.g.
    // mechanical_dice's build order 2,4,6,8,… vs metadata order 2,6,8,4,…).
    // Each flattened leaf carries its outer build objectid in
    // `source_object_id`, matching the metadata's `<object id>`.
    //
    // An outer `<object>` owns the flattened leaves sharing its id, in
    // flatten order — `parts.len().max(1)` of them, `max(1)` because a
    // *part-less* `<object>` (the single-volume foreign shape, e.g.
    // OrcaCube) still owns one leaf and its object-level config. For each
    // owned leaf we apply identity (name/extruder) + per-object config +
    // plate id + group id together, so they can never desync.
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

    // Outer object id → its flattened leaf indices, in flatten order.
    let mut leaves_by_id: HashMap<u32, Vec<usize>> = HashMap::new();
    for (idx, obj) in objects.iter().enumerate() {
        leaves_by_id
            .entry(obj.source_object_id)
            .or_default()
            .push(idx);
    }

    for outer in &settings.objects {
        let Some(leaves) = leaves_by_id.get(&outer.id) else {
            continue;
        };
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
        for (offset, &idx) in leaves.iter().enumerate() {
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

    fn proj_obj(mesh_idx: usize, source_object_id: u32) -> ProjectObject {
        ProjectObject {
            mesh_idx,
            transform: Transform::IDENTITY,
            source_object_id,
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
        // Leaves carry their outer build objectid (A=1, B=2).
        let mut objects = vec![proj_obj(0, 1), proj_obj(1, 2)];
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
    fn names_track_object_id_when_metadata_order_differs_from_build_order() {
        use bbs_meta::{ModelSettings, ObjectSettings};
        use std::collections::BTreeMap;

        // mechanical_dice.3mf: <build> lists objectids 2,4,6 but
        // model_settings.config lists them 2,6,4. A positional walk would
        // give the geometry leaf for id 4 the name authored for id 6.
        let obj = |id: u32, name: &str| ObjectSettings {
            id,
            name: Some(name.into()),
            default_extruder: None,
            config: BTreeMap::new(),
            parts: vec![],
        };
        let settings = ModelSettings {
            objects: vec![
                obj(2, "top_frame"),
                obj(6, "wheel"),
                obj(4, "bottom_frame"),
            ],
            plates: vec![],
        };
        // Geometry in build order: 2, 4, 6.
        let mut objects = vec![proj_obj(0, 2), proj_obj(1, 4), proj_obj(2, 6)];
        apply_bbs_metadata(&settings, &mut objects);

        assert_eq!(objects[0].name, "top_frame");
        assert_eq!(objects[1].name, "bottom_frame");
        assert_eq!(objects[2].name, "wheel");
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
        // in `core::project::mutation::materials::tests::auto_bind_*`.
        let project = load_3mf(&two_cubes_fixture()).expect("load 2-cube fixture");
        assert_eq!(project.objects.len(), 2, "two build items");
        let extruders: Vec<Option<u8>> = project.objects.iter().map(|o| o.extruder_id).collect();
        assert_eq!(extruders, vec![Some(1), Some(2)]);
        let names: Vec<&str> = project.objects.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, vec!["Cube A (T0)", "Cube B (T1)"]);
    }

    fn prusa_cube_fixture() -> PathBuf {
        let crate_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
        PathBuf::from(crate_dir).join("tests/fixtures/3mf/prusa-cube.3mf")
    }

    #[test]
    fn prusa_pe_3mf_imports_geometry_and_object_name() {
        // A Slic3r PE / PrusaSlicer 3MF: standard-core geometry + a
        // `Slic3r_PE_model.config` (no BBS `model_settings.config`, no
        // `project_settings.config`). We used to reject these outright; now the
        // geometry imports and the object display name is lifted from the Prusa
        // metadata — NOT the sibling `<volume>` name ("bookend.stl").
        let project = load_3mf(&prusa_cube_fixture()).expect("load Prusa PE 3MF");
        assert_eq!(project.objects.len(), 1, "single build item");
        assert_eq!(project.objects[0].name, "Heart-shaped Bookend");
        assert!(
            project.embedded_settings.is_none(),
            "a Prusa model 3MF carries no BBS project_settings.config",
        );
        // Geometry actually parsed (a 20mm cube → 24 verts / 12 tris).
        let mesh = &project.meshes[project.objects[0].mesh_idx];
        assert_eq!(mesh.vertices.len(), 24 * 3, "cube vertices");
        assert_eq!(mesh.indices.len(), 12 * 3, "cube triangles");

        // The geometry-intent subset of Slic3r_PE.config lands as per-object
        // overrides (Prusa key → OrcaSlicer key); printer/filament keys and the
        // print-global skirt/spiral keys are NOT adopted.
        let ov = &project.objects[0].overrides;
        let get = |k: &str| ov.get(k).map(String::as_str);
        // Shells / walls / infill.
        assert_eq!(get("top_shell_layers"), Some("0"));
        assert_eq!(get("bottom_shell_layers"), Some("0"));
        assert_eq!(get("wall_loops"), Some("6"));
        assert_eq!(get("sparse_infill_density"), Some("15%"));
        assert_eq!(get("sparse_infill_pattern"), Some("honeycomb"));
        assert_eq!(get("top_surface_pattern"), Some("rectilinear"));
        assert_eq!(get("bottom_surface_pattern"), Some("rectilinear"));
        assert_eq!(get("infill_direction"), Some("45"));
        assert_eq!(get("infill_wall_overlap"), Some("25%"));
        // Raft / brim (object-scoped) + seam. Prusa's brim_width=0 → OrcaSlicer
        // brim_type=no_brim, since Orca's default auto_brim ignores brim_width.
        assert_eq!(get("raft_layers"), Some("0"));
        assert_eq!(get("brim_width"), Some("0"));
        assert_eq!(get("brim_type"), Some("no_brim"));
        assert_eq!(get("seam_position"), Some("nearest"));
        // NOT adopted: printer/filament, and print-global skirt/spiral (which
        // can't be per-object overrides).
        for absent in [
            "bed_shape",
            "nozzle_temperature",
            "skirt_loops",
            "skirts",
            "spiral_mode",
            "spiral_vase",
        ] {
            assert!(!ov.contains_key(absent), "{absent} must not be adopted");
        }
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




}
