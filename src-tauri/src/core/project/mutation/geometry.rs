//! Mesh + scene-object mechanics for [`Project`]: ID allocation, mesh
//! registration, object lifecycle (add/clone/delete/move-between-plates),
//! transforms (orient, lay-flat, align-face, set-transform), selection,
//! grouping, and the bounds/seat/clamp helpers.

use std::collections::{BTreeSet, HashMap, HashSet};

use glam::{Quat, Vec3};

use crate::core::project::model::{PlateId, Project};
use crate::core::printer::profile::BoundingBox;
use crate::core::scene::bed;
use crate::core::scene::events::{SceneEvent, SceneOpError, SelectMode};
use crate::core::scene::primitives::{self, PrimitiveKind, PrimitiveParams};
use crate::core::scene::state::{
    mesh_bb_corners, Group, GroupId, HoleMarker, Mesh, MeshId, MeshProvenance, Modifier,
    ModifierKind, NewMesh, NewSceneObject, ObjectId, OrderedIds, SceneObject,
};
use crate::core::scene::transform::Transform;

/// Which side of the cut plane a half came from (the side the plane normal
/// points toward = positive).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CutSide {
    Pos,
    Neg,
}

impl CutSide {
    /// Object/group name suffix for a half from this side when both sides are
    /// kept.
    fn suffix(self) -> &'static str {
        match self {
            CutSide::Pos => " (A)",
            CutSide::Neg => " (B)",
        }
    }
}

/// One source object's data, read off the scene lock so the FFI cut can run
/// unlocked. See [`Project::cut_targets`].
pub struct CutTarget {
    pub object_id: ObjectId,
    pub vertices: std::sync::Arc<Vec<f32>>,
    pub indices: std::sync::Arc<Vec<u32>>,
    /// Per-triangle MMU paint, passed through the cut so painted faces survive.
    pub paint: Option<std::sync::Arc<Vec<String>>>,
    pub transform: Transform,
    pub name: String,
    pub extruder_id: Option<u8>,
    pub group: Option<GroupId>,
    /// The source object's existing connector volumes (local frame): peg/hole
    /// geometry + kind. Cutting a previously-cut object carries these onto the
    /// halves (routed by side) so a re-cut adds to the connectors instead of
    /// dropping them with the deleted source.
    pub modifiers: Vec<(Vec<f32>, Vec<u32>, ModifierKind)>,
    /// The source object's existing hole markers (local frame), carried likewise.
    pub hole_markers: Vec<HoleMarker>,
}

/// One kept half of a cut (the FFI output for a side the user is keeping).
pub struct CutHalfOut {
    pub side: CutSide,
    pub vertices: Vec<f32>,
    pub indices: Vec<u32>,
    /// Per-triangle MMU paint re-projected onto this half, or `None` when the
    /// source was unpainted.
    pub paint: Option<Vec<String>>,
    /// Connector volumes for this half (local frame): peg/hole mesh + kind.
    /// Registered as the half object's [`SceneObject`] modifiers, resolved at
    /// slice time rather than baked in.
    pub modifiers: Vec<(Vec<f32>, Vec<u32>, ModifierKind)>,
    /// Hole-opening markers for this half (local frame). Display-only decals the
    /// viewport shades onto the cut cap; stored, never sliced.
    pub hole_markers: Vec<HoleMarker>,
}

/// The kept halves of one cut source, ready to register. See
/// [`Project::apply_cut`].
pub struct CutResult {
    pub source_id: ObjectId,
    pub transform: Transform,
    pub base_name: String,
    pub extruder_id: Option<u8>,
    pub source_group: Option<GroupId>,
    pub halves: Vec<CutHalfOut>,
    /// Free dowel-pin meshes (local frame), one per Dowel connector — each
    /// registered as a separate printable object.
    pub dowels: Vec<(Vec<f32>, Vec<u32>)>,
}

impl Project {
    // ---- ID allocators (scene-wide) -------------------------------

    /// Allocate the next monotonic `MeshId`. IDs start at 1.
    pub fn next_mesh_id(&mut self) -> MeshId {
        self.next_mesh_id = self.next_mesh_id.wrapping_add(1);
        MeshId(self.next_mesh_id)
    }

    /// Allocate the next monotonic `ObjectId`. IDs start at 1.
    pub fn next_object_id(&mut self) -> ObjectId {
        self.next_object_id = self.next_object_id.wrapping_add(1);
        ObjectId(self.next_object_id)
    }

    /// Register a mesh in the scene-wide pool. Always allocates a
    /// fresh `MeshId`; the caller hands in a `NewMesh` (no id field)
    /// so there's no possibility of an ID collision or sentinel
    /// ambiguity.
    pub fn register_mesh(&mut self, new_mesh: NewMesh) -> MeshId {
        let id = self.next_mesh_id();
        self.meshes.insert(
            id,
            Mesh {
                id,
                vertices: std::sync::Arc::new(new_mesh.vertices),
                indices: std::sync::Arc::new(new_mesh.indices),
                paint_colors: new_mesh.paint_colors.map(std::sync::Arc::new),
                support_paint: new_mesh.support_paint.map(std::sync::Arc::new),
                bounding_box: new_mesh.bounding_box,
                provenance: new_mesh.provenance,
            },
        );
        id
    }

    /// Register a scene object on the active plate. Always
    /// allocates a fresh `ObjectId` (scene-wide unique).
    ///
    /// **Auto-bind side effect:** if the object's model material
    /// (its `extruder_id`, defaulted to `1` when `None`) isn't
    /// already in the active plate's `material_to_slot` map, a
    /// default mapping lands automatically — walks the bound
    /// PrinterInstance's extruder/slot grid in `(extruder, slot)`
    /// order, picks the slot at flat-index `(material - 1) MOD
    /// total_slots`. Idempotent on re-call; user edits via the
    /// panel always win.
    pub fn register_object(&mut self, new_obj: NewSceneObject) -> ObjectId {
        let id = self.next_object_id();
        let active = self.active_plate;
        let extruder_id = new_obj.extruder_id;
        self.plates[active].scene.objects.push(SceneObject {
            id,
            mesh: new_obj.mesh,
            transform: new_obj.transform,
            name: new_obj.name,
            visible: new_obj.visible,
            extruder_id,
            group: new_obj.group,
        });
        self.ensure_default_material_slot_on_active(extruder_id.unwrap_or(1));
        // MMU face-painted materials live on the mesh, not the object's
        // extruder_id, so bind them too — else adding a single painted
        // multi-material object surfaces only its base material in the list
        // (the preview already shows both). Mirrors the Orca importer, which
        // binds painted filaments via ensure_material_bound_on_active.
        for material in self.mesh_painted_materials(new_obj.mesh) {
            self.ensure_default_material_slot_on_active(material);
        }
        id
    }

    /// Register a mesh and place one default `SceneObject` at
    /// origin on the active plate. Returns (mesh_id, object_id,
    /// events).
    pub fn load_mesh(&mut self, new_mesh: NewMesh) -> (MeshId, ObjectId, Vec<SceneEvent>) {
        let obj_name = match &new_mesh.provenance {
            MeshProvenance::File(p) => p
                .rsplit_once('/')
                .map(|(_, leaf)| leaf.to_string())
                .unwrap_or_else(|| p.clone()),
            MeshProvenance::Primitive(name) => name.clone(),
        };
        let mesh_id = self.register_mesh(new_mesh);
        // Lift to z=0 + center on the bed. Without this, an STL/OBJ
        // whose native vertices live below Z=0 (common for symmetric
        // models centered on origin) lands underneath the build
        // plate; libslic3r clips it and the slice produces zero
        // layers. Same treatment add_from_primitive applies to
        // procedural primitives.
        let transform = self.lift_and_center_transform(mesh_id);
        let obj_id = self.register_object(NewSceneObject {
            mesh: mesh_id,
            transform,
            name: obj_name,
            visible: true,
            extruder_id: None,
            group: None,
        });

        let plate_id = self.active_plate().id;
        let mesh_header = self.meshes.get(&mesh_id).unwrap().header();
        let obj_clone = self
            .active_plate()
            .scene
            .objects
            .get(&obj_id)
            .unwrap()
            .clone();
        let events = vec![
            SceneEvent::MeshLoaded { mesh: mesh_header },
            SceneEvent::ObjectAdded {
                plate_id,
                object: obj_clone,
            },
        ];
        (mesh_id, obj_id, events)
    }

    /// Build the transform that drops the mesh's lowest-Z vertex
    /// onto the build plate (Z=0) and centers its XY bbox on the
    /// active plate's bed. Falls back to a Z-only lift when no bed
    /// is bound, and to identity when neither shift is needed.
    ///
    /// Used by both [`Self::add_from_primitive`] and
    /// [`Self::load_mesh`] so file-imported meshes get the same
    /// "land on the bed, centered" treatment as procedural ones.
    fn lift_and_center_transform(&self, mesh_id: MeshId) -> Transform {
        let mesh_bb = self.meshes.get(&mesh_id).unwrap().bounding_box;
        let lift_z = -mesh_bb.min[2] as f32;
        let (shift_x, shift_y) = match &self.active_plate().scene.bed {
            Some(bed) => {
                let bed_cx = ((bed.extents.min[0] + bed.extents.max[0]) * 0.5) as f32;
                let bed_cy = ((bed.extents.min[1] + bed.extents.max[1]) * 0.5) as f32;
                let mesh_cx = ((mesh_bb.min[0] + mesh_bb.max[0]) * 0.5) as f32;
                let mesh_cy = ((mesh_bb.min[1] + mesh_bb.max[1]) * 0.5) as f32;
                (bed_cx - mesh_cx, bed_cy - mesh_cy)
            }
            None => (0.0, 0.0),
        };
        if shift_x.abs() + shift_y.abs() + lift_z.abs() > 1e-5 {
            Transform::translation(Vec3::new(shift_x, shift_y, lift_z))
        } else {
            Transform::IDENTITY
        }
    }

    /// Add (or re-instance) a procedural primitive on the active
    /// plate. Looks up the `(kind, params)` tuple in the scene-wide
    /// cache; if missing, generates the mesh once and stashes the
    /// `MeshId`. Always creates a fresh SceneObject placed at plate
    /// origin so re-clicking "Add cube" in the library palette
    /// piles new objects on top rather than replacing the previous
    /// one.
    pub fn add_from_primitive(
        &mut self,
        kind: PrimitiveKind,
        params: PrimitiveParams,
    ) -> (MeshId, ObjectId, Vec<SceneEvent>) {
        let mut events = Vec::new();
        let mesh_id = match self
            .primitive_cache
            .iter()
            .find(|(k, p, _)| *k == kind && *p == params)
            .map(|(_, _, id)| *id)
        {
            Some(id) => id,
            None => {
                let new_mesh = primitives::generate(kind, params);
                let id = self.register_mesh(new_mesh);
                self.primitive_cache.push((kind, params, id));
                let header = self.meshes.get(&id).unwrap().header();
                events.push(SceneEvent::MeshLoaded { mesh: header });
                id
            }
        };

        let name = match kind {
            PrimitiveKind::Cube => "Cube",
            PrimitiveKind::Cylinder => "Cylinder",
            PrimitiveKind::Sphere => "Sphere",
            PrimitiveKind::Cone => "Cone",
            PrimitiveKind::Torus => "Torus",
        };
        // Lift to z=0 + center on the bed via the shared helper —
        // primitives like cube/sphere/torus are origin-centered
        // geometrically, so a bare-origin placement would sink half
        // the primitive below the plate AND straddle the back-left
        // corner.
        let transform = self.lift_and_center_transform(mesh_id);
        let obj_id = self.register_object(NewSceneObject {
            mesh: mesh_id,
            transform,
            name: name.to_string(),
            visible: true,
            extruder_id: None,
            group: None,
        });
        let plate_id = self.active_plate().id;
        let obj_clone = self
            .active_plate()
            .scene
            .objects
            .get(&obj_id)
            .unwrap()
            .clone();
        events.push(SceneEvent::ObjectAdded {
            plate_id,
            object: obj_clone,
        });
        events.extend(self.out_of_bounds_event(obj_id));
        (mesh_id, obj_id, events)
    }

    /// Clone `ids` (objects on the active plate) `times` times back onto the
    /// active plate. Geometry is shared (same `MeshId`); each copy is placed at
    /// the source's transform — the caller auto-arranges to spread the copies
    /// out instead of stacking them on the originals.
    ///
    /// Everything that makes a copy a faithful duplicate travels with it:
    /// per-object setting overrides are copied, and grouping is preserved *per
    /// copy* — objects that shared a group get a fresh shared `GroupId` (plus
    /// the group's display name) in each copy, so a multi-volume object stays
    /// rigid yet arranges as its own unit, independent of the original.
    ///
    /// Returns the new object ids (copy-major: copy 0's objects, then copy 1's,
    /// …) and the `ObjectAdded` events to emit.
    pub fn clone_objects(
        &mut self,
        ids: &OrderedIds,
        times: u32,
    ) -> (Vec<ObjectId>, Vec<SceneEvent>) {
        let active = self.active_plate;
        // Snapshot the sources up front so every copy clones the *original*,
        // never a copy. `ids` is an OrderedIds (authored order, guaranteed by the
        // type), so the clones land in the same order as their originals — there
        // is no way to pass an unordered selection here. Carry each source's
        // overrides alongside it.
        let plate = &self.plates[active].scene;
        type Source = (
            SceneObject,
            Option<HashMap<String, String>>,
            Option<Vec<Modifier>>,
            Option<Vec<HoleMarker>>,
        );
        let sources: Vec<Source> = ids
            .iter()
            .filter_map(|id| {
                let obj = plate.objects.get(id)?.clone();
                let overrides = plate.object_overrides.get(id).cloned();
                // Reuse the source's connector MeshIds — meshes are immutable and
                // prune counts every plate's modifier refs, so a cloned half keeps
                // its pegs/holes without re-registering geometry.
                let modifiers = plate.object_modifiers.get(id).cloned();
                let hole_markers = plate.object_hole_markers.get(id).cloned();
                Some((obj, overrides, modifiers, hole_markers))
            })
            .collect();

        let mut new_ids = Vec::new();
        let mut events = Vec::new();
        if sources.is_empty() {
            return (new_ids, events);
        }

        for _ in 0..times {
            // One fresh group id per source group, shared within this copy.
            let mut group_remap: HashMap<GroupId, GroupId> = HashMap::new();
            for (src, overrides, modifiers, hole_markers) in &sources {
                let group = src
                    .group
                    .map(|g| *group_remap.entry(g).or_insert_with(GroupId::fresh));
                let obj_id = self.register_object(NewSceneObject {
                    mesh: src.mesh,
                    transform: src.transform,
                    name: src.name.clone(),
                    visible: src.visible,
                    extruder_id: src.extruder_id,
                    group,
                });
                if let Some(ov) = overrides {
                    self.plates[active]
                        .scene
                        .object_overrides
                        .insert(obj_id, ov.clone());
                }
                if let Some(mods) = modifiers {
                    self.plates[active]
                        .scene
                        .object_modifiers
                        .insert(obj_id, mods.clone());
                }
                if let Some(markers) = hole_markers {
                    self.plates[active]
                        .scene
                        .object_hole_markers
                        .insert(obj_id, markers.clone());
                }
                let plate_id = self.plates[active].id;
                let object = self.plates[active]
                    .scene
                    .objects
                    .get(&obj_id)
                    .expect("just registered")
                    .clone();
                events.push(SceneEvent::ObjectAdded { plate_id, object });
                new_ids.push(obj_id);
            }
            // Carry the group display names onto the fresh group ids.
            for (src_g, new_g) in &group_remap {
                if let Some(g) = self.plates[active].scene.groups.get(src_g).cloned() {
                    self.plates[active].scene.groups.insert(*new_g, g);
                }
            }
        }
        (new_ids, events)
    }

    // ---- Selection -----------------------------------------------

    /// Apply a selection change on the active plate. Returns one
    /// `SelectionChanged` event (sorted for deterministic output)
    /// or empty if the selection didn't actually change.
    /// Expand `ids` to include every object sharing a group with any of them,
    /// returned in authored order as an [`OrderedIds`]. Ungrouped ids pass
    /// through (still ordered); the result is deduped.
    fn expand_to_groups(
        plate: &crate::core::scene::state::PlateSceneState,
        ids: &[ObjectId],
    ) -> OrderedIds {
        let groups: HashSet<GroupId> = ids
            .iter()
            .filter_map(|id| plate.objects.get(id).and_then(|o| o.group))
            .collect();
        if groups.is_empty() {
            return plate.objects.in_order(ids);
        }
        let mut want: HashSet<ObjectId> = ids.iter().copied().collect();
        for o in plate.objects.values() {
            if o.group.is_some_and(|g| groups.contains(&g)) {
                want.insert(o.id);
            }
        }
        let want: Vec<ObjectId> = want.into_iter().collect();
        plate.objects.in_order(&want)
    }

    /// Expand `ids` to whole-group membership on the active plate (in authored
    /// order). The canvas uses this so clicking one part selects the whole
    /// group; the object list passes ids straight to `select` to keep parts
    /// individually selectable.
    pub fn group_expanded_ids(&self, ids: &[ObjectId]) -> OrderedIds {
        Self::expand_to_groups(&self.plates[self.active_plate].scene, ids)
    }

    pub fn select(&mut self, ids: &[ObjectId], mode: SelectMode) -> Vec<SceneEvent> {
        let active = self.active_plate;
        let plate_id = self.plates[active].id;
        let plate = &mut self.plates[active].scene;
        let before: HashSet<ObjectId> = plate.selection.iter().copied().collect();
        // `ids` is taken as-is — the object list selects individuals.
        // The canvas's select-the-whole-group behaviour is opt-in via
        // `group_expanded_ids` at the command layer.
        let present: Vec<ObjectId> = ids
            .iter()
            .copied()
            .filter(|id| plate.objects.contains_key(id))
            .collect();
        match mode {
            SelectMode::Replace => {
                plate.selection = present.into_iter().collect();
            }
            SelectMode::Add => {
                for id in present {
                    plate.selection.insert(id);
                }
            }
            SelectMode::Toggle => {
                // Toggle the (possibly group-expanded) set as a unit:
                // remove it if already fully selected, else add it all.
                let all_selected =
                    !present.is_empty() && present.iter().all(|id| plate.selection.contains(id));
                for id in present {
                    if all_selected {
                        plate.selection.remove(&id);
                    } else {
                        plate.selection.insert(id);
                    }
                }
            }
        }
        if plate.selection == before {
            return Vec::new();
        }
        let mut sorted: Vec<ObjectId> = plate.selection.iter().copied().collect();
        sorted.sort();
        vec![SceneEvent::SelectionChanged {
            plate_id,
            selected: sorted,
        }]
    }

    /// Clear the active plate's selection.
    pub fn deselect_all(&mut self) -> Vec<SceneEvent> {
        let active = self.active_plate;
        let plate_id = self.plates[active].id;
        let plate = &mut self.plates[active].scene;
        if plate.selection.is_empty() {
            return Vec::new();
        }
        plate.selection.clear();
        vec![SceneEvent::SelectionChanged {
            plate_id,
            selected: Vec::new(),
        }]
    }

    // ---- Object grouping (active plate) ---------------------------
    //
    // A "group" is a set of objects sharing a `group` — the same
    // mechanism the 3MF loader uses for multi-volume objects, now also
    // driven by user grouping. The writer/slice path already emits a
    // shared `group` as one ModelObject with multiple volumes, so
    // grouping == assembling into one logical print object. Only the
    // display name (`scene.groups`) is new state.

    /// Group `ids` on the active plate under one new, globally-unique
    /// [`GroupId`] named `name`. Objects already in other groups move
    /// into this one; any group thereby left with a single member is
    /// dissolved. No-op for fewer than two ids.
    pub fn group_objects(
        &mut self,
        ids: &[ObjectId],
        name: String,
    ) -> Result<Vec<SceneEvent>, SceneOpError> {
        if ids.len() < 2 {
            return Ok(Vec::new());
        }
        let group = GroupId::fresh();
        let active = self.active_plate;
        let plate_id = self.plates[active].id;
        let mut events = Vec::new();
        for &id in ids {
            let obj = self.plates[active]
                .scene
                .objects
                .get_mut(&id)
                .ok_or(SceneOpError::UnknownObject(id))?;
            obj.group = Some(group);
            events.push(SceneEvent::ObjectUpdated {
                plate_id,
                object: obj.clone(),
            });
        }
        self.plates[active]
            .scene
            .groups
            .insert(group, Group { name });
        events.extend(self.dissolve_orphan_groups_on_active());
        events.push(SceneEvent::PlateChanged { plate_id });
        Ok(events)
    }

    /// Ungroup: clear `group` from every member of `group` on the active
    /// plate and drop its name.
    pub fn ungroup_objects(&mut self, group: GroupId) -> Vec<SceneEvent> {
        let active = self.active_plate;
        let plate_id = self.plates[active].id;
        let member_ids: Vec<ObjectId> = self.plates[active]
            .scene
            .objects
            .values()
            .filter(|o| o.group == Some(group))
            .map(|o| o.id)
            .collect();
        let mut events = Vec::new();
        for id in member_ids {
            if let Some(obj) = self.plates[active].scene.objects.get_mut(&id) {
                obj.group = None;
                events.push(SceneEvent::ObjectUpdated {
                    plate_id,
                    object: obj.clone(),
                });
            }
        }
        self.plates[active].scene.groups.remove(&group);
        events.push(SceneEvent::PlateChanged { plate_id });
        events
    }

    /// Rename a group on the active plate.
    pub fn rename_group(&mut self, group: GroupId, name: String) -> Vec<SceneEvent> {
        let active = self.active_plate;
        let plate_id = self.plates[active].id;
        self.plates[active]
            .scene
            .groups
            .insert(group, Group { name });
        vec![SceneEvent::PlateChanged { plate_id }]
    }

    /// Dissolve any group on the active plate left with fewer than two
    /// members — a group of one isn't a group. Clears those objects'
    /// `group` and drops the names. Returns the per-object events.
    fn dissolve_orphan_groups_on_active(&mut self) -> Vec<SceneEvent> {
        use std::collections::{HashMap, HashSet};
        let active = self.active_plate;
        let plate_id = self.plates[active].id;
        let mut counts: HashMap<GroupId, usize> = HashMap::new();
        for o in self.plates[active].scene.objects.values() {
            if let Some(g) = o.group {
                *counts.entry(g).or_insert(0) += 1;
            }
        }
        let orphans: HashSet<GroupId> = counts
            .into_iter()
            .filter(|&(_, c)| c < 2)
            .map(|(g, _)| g)
            .collect();
        if orphans.is_empty() {
            return Vec::new();
        }
        let ids: Vec<ObjectId> = self.plates[active]
            .scene
            .objects
            .values()
            .filter(|o| o.group.is_some_and(|g| orphans.contains(&g)))
            .map(|o| o.id)
            .collect();
        let mut events = Vec::new();
        for id in ids {
            if let Some(obj) = self.plates[active].scene.objects.get_mut(&id) {
                obj.group = None;
                events.push(SceneEvent::ObjectUpdated {
                    plate_id,
                    object: obj.clone(),
                });
            }
        }
        for g in orphans {
            self.plates[active].scene.groups.remove(&g);
        }
        events
    }

    /// Build one combined **world-space** mesh from a set of objects — each
    /// object's local mesh baked through its transform — as flattened vertices
    /// + triangle indices. Used to run the FFI auto-orient on a whole selection
    /// (single object, a group, or a multi-select) off the scene lock, so the
    /// optimizer sees the assembly as it currently sits.
    pub fn objects_world_mesh(
        &self,
        ids: &[ObjectId],
    ) -> Result<(Vec<f32>, Vec<u32>), SceneOpError> {
        let active = self.active_plate;
        let mut vertices: Vec<f32> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        for &id in ids {
            let obj = self.plates[active]
                .scene
                .objects
                .get(&id)
                .ok_or(SceneOpError::UnknownObject(id))?;
            let mesh = self
                .meshes
                .get(&obj.mesh)
                .ok_or(SceneOpError::UnknownMesh(obj.mesh))?;
            // Main mesh + solid pegs (each keeps its own index base offset).
            for mesh in std::iter::once(mesh).chain(self.peg_meshes(id)) {
                let base = (vertices.len() / 3) as u32;
                for v in mesh.vertices.chunks_exact(3) {
                    let w = obj.transform.apply_point(Vec3::new(v[0], v[1], v[2]));
                    vertices.extend_from_slice(&[w.x, w.y, w.z]);
                }
                indices.extend(mesh.indices.iter().map(|&i| i + base));
            }
        }
        Ok((vertices, indices))
    }

    /// Read out the per-object data the split tool needs to cut each selected
    /// object off the scene lock: its local mesh (Arc-cloned — no copy), world
    /// transform, name, material, and group. Skips invisible objects. Errors on
    /// an unknown object/mesh. The plane is applied per object in its own local
    /// frame by the caller, so the halves can reuse the same `transform`.
    pub fn cut_targets(&self, ids: &[ObjectId]) -> Result<Vec<CutTarget>, SceneOpError> {
        let active = self.active_plate;
        let mut out = Vec::new();
        for &id in ids {
            let obj = self.plates[active]
                .scene
                .objects
                .get(&id)
                .ok_or(SceneOpError::UnknownObject(id))?;
            if !obj.visible {
                continue;
            }
            let mesh = self
                .meshes
                .get(&obj.mesh)
                .ok_or(SceneOpError::UnknownMesh(obj.mesh))?;
            // Existing connector volumes → raw geometry (re-registered per half in
            // apply_cut), so a re-cut carries them onto the resulting halves.
            let modifiers = self.plates[active]
                .scene
                .object_modifiers
                .get(&id)
                .into_iter()
                .flatten()
                .filter_map(|m| {
                    let mm = self.meshes.get(&m.mesh)?;
                    Some((mm.vertices.as_ref().clone(), mm.indices.as_ref().clone(), m.kind))
                })
                .collect();
            let hole_markers = self.plates[active]
                .scene
                .object_hole_markers
                .get(&id)
                .cloned()
                .unwrap_or_default();
            out.push(CutTarget {
                object_id: id,
                vertices: mesh.vertices.clone(),
                indices: mesh.indices.clone(),
                paint: mesh.paint_colors.clone(),
                transform: obj.transform,
                name: obj.name.clone(),
                extruder_id: obj.extruder_id,
                group: obj.group,
                modifiers,
                hole_markers,
            });
        }
        Ok(out)
    }

    /// Register each kept cut half as a new mesh + object on the active plate,
    /// remove the source objects, and select the results. Grouping is preserved
    /// per side: halves descending from one source group are re-grouped (one
    /// fresh group per (source group, side)), so a group split in two yields two
    /// coherent groups; halves of an ungrouped source stay ungrouped. Per-side
    /// groups left with a single member dissolve. MMU paint rides along per half
    /// (`CutHalfOut::paint`, re-projected by the FFI); dowel pins are fresh
    /// geometry and stay unpainted.
    pub fn apply_cut(&mut self, results: Vec<CutResult>) -> (Vec<ObjectId>, Vec<SceneEvent>) {
        let active = self.active_plate;
        let plate_id = self.plates[active].id;
        let mut events = Vec::new();
        let mut new_ids = Vec::new();
        // (source group, side) → fresh group for that side's halves.
        let mut group_remap: HashMap<(GroupId, CutSide), GroupId> = HashMap::new();

        let source_ids: Vec<ObjectId> = results.iter().map(|r| r.source_id).collect();
        for res in results {
            let both = res.halves.len() > 1; // both sides kept → " (A)"/" (B)"
            for half in res.halves {
                let group = res
                    .source_group
                    .map(|g| *group_remap.entry((g, half.side)).or_insert_with(GroupId::fresh));
                let suffix = if both { half.side.suffix() } else { " (cut)" };
                let bbox = crate::core::scene::loaders::compute_bounding_box(&half.vertices);
                let mesh = self.register_mesh(NewMesh {
                    vertices: half.vertices,
                    indices: half.indices,
                    paint_colors: half.paint,
                    // ponytail: cut drops support paint in v1 (the deferred-cut
                    // FFI only carries MMU paint); repaint after cutting.
                    support_paint: None,
                    bounding_box: bbox,
                    provenance: MeshProvenance::Primitive(format!("{} (cut)", res.base_name)),
                });
                let header = self.meshes.get(&mesh).expect("just registered").header();
                events.push(SceneEvent::MeshLoaded { mesh: header });
                let obj_id = self.register_object(NewSceneObject {
                    mesh,
                    transform: res.transform,
                    name: format!("{}{}", res.base_name, suffix),
                    visible: true,
                    extruder_id: res.extruder_id,
                    group,
                });
                let object = self.plates[active]
                    .scene
                    .objects
                    .get(&obj_id)
                    .expect("just registered")
                    .clone();
                events.push(SceneEvent::ObjectAdded { plate_id, object });
                // Register this half's connector volumes as object modifiers
                // (mesh pool + the per-object sidecar), resolved at slice time.
                if !half.modifiers.is_empty() {
                    let mut mods = Vec::with_capacity(half.modifiers.len());
                    for (verts, idx, kind) in half.modifiers {
                        let bbox = crate::core::scene::loaders::compute_bounding_box(&verts);
                        let mmesh = self.register_mesh(NewMesh {
                            vertices: verts,
                            indices: idx,
                            paint_colors: None,
                            support_paint: None,
                            bounding_box: bbox,
                            provenance: MeshProvenance::Primitive(format!(
                                "{} connector",
                                res.base_name
                            )),
                        });
                        let header = self.meshes.get(&mmesh).expect("just registered").header();
                        events.push(SceneEvent::MeshLoaded { mesh: header });
                        mods.push(Modifier { mesh: mmesh, kind });
                    }
                    self.plates[active].scene.object_modifiers.insert(obj_id, mods);
                }
                if !half.hole_markers.is_empty() {
                    self.plates[active]
                        .scene
                        .object_hole_markers
                        .insert(obj_id, half.hole_markers);
                }
                new_ids.push(obj_id);
            }
            // Free dowel pins → standalone ungrouped objects (same transform as
            // the source; the user arranges them).
            for (di, (verts, idx)) in res.dowels.into_iter().enumerate() {
                if verts.is_empty() || idx.is_empty() {
                    continue;
                }
                let bbox = crate::core::scene::loaders::compute_bounding_box(&verts);
                let mesh = self.register_mesh(NewMesh {
                    vertices: verts,
                    indices: idx,
                    paint_colors: None,
                    support_paint: None,
                    bounding_box: bbox,
                    provenance: MeshProvenance::Primitive(format!("{} pin", res.base_name)),
                });
                let header = self.meshes.get(&mesh).expect("just registered").header();
                events.push(SceneEvent::MeshLoaded { mesh: header });
                let obj_id = self.register_object(NewSceneObject {
                    mesh,
                    transform: res.transform,
                    name: format!("{} pin {}", res.base_name, di + 1),
                    visible: true,
                    extruder_id: res.extruder_id,
                    group: None,
                });
                let object = self.plates[active]
                    .scene
                    .objects
                    .get(&obj_id)
                    .expect("just registered")
                    .clone();
                events.push(SceneEvent::ObjectAdded { plate_id, object });
                new_ids.push(obj_id);
            }
        }
        // Name each per-side group after its source group.
        for ((src_g, side), new_g) in &group_remap {
            let base = self.plates[active]
                .scene
                .groups
                .get(src_g)
                .map(|g| g.name.clone())
                .unwrap_or_default();
            self.plates[active]
                .scene
                .groups
                .insert(*new_g, Group { name: format!("{base}{}", side.suffix()) });
        }
        // Remove the originals (prunes orphan meshes/material bindings + emits
        // ObjectRemoved/SelectionChanged).
        events.extend(self.delete_objects(&source_ids));
        // A per-side group that ended with a single member isn't a group.
        events.extend(self.dissolve_orphan_groups_on_active());
        // Drop now-memberless group entries (the fully-consumed source groups).
        let live: HashSet<GroupId> = self.plates[active]
            .scene
            .objects
            .values()
            .filter_map(|o| o.group)
            .collect();
        self.plates[active].scene.groups.retain(|g, _| live.contains(g));
        // Select the freshly-created halves.
        events.extend(self.select(&new_ids, SelectMode::Replace));
        (new_ids, events)
    }

    /// Combined world-space AABB of a selection from each object's local
    /// bounding-box corners — the conservative bbox used for bounds + clamping
    /// (matches `bed::object_out_of_bounds`).
    /// Solid peg modifier meshes attached to `id` (local frame, so they ride the
    /// object's transform like its main mesh). Placement ops fold these in so a
    /// cut half's protruding peg is seated/bounded with the object. Holes are
    /// cavities and hole-markers flat decals — neither extends the outer hull, so
    /// both are excluded.
    fn peg_meshes(&self, id: ObjectId) -> impl Iterator<Item = &Mesh> {
        self.plates[self.active_plate]
            .scene
            .object_modifiers
            .get(&id)
            .into_iter()
            .flatten()
            .filter(|m| m.kind == ModifierKind::Peg)
            .filter_map(move |m| self.meshes.get(&m.mesh))
    }

    fn combined_bbox_aabb(&self, ids: &[ObjectId]) -> Result<(Vec3, Vec3), SceneOpError> {
        let active = self.active_plate;
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for &id in ids {
            let obj = self.plates[active]
                .scene
                .objects
                .get(&id)
                .ok_or(SceneOpError::UnknownObject(id))?;
            let bb = self
                .meshes
                .get(&obj.mesh)
                .ok_or(SceneOpError::UnknownMesh(obj.mesh))?
                .bounding_box;
            let peg_bbs = self.peg_meshes(id).map(|m| m.bounding_box);
            for bb in std::iter::once(bb).chain(peg_bbs) {
                for corner in mesh_bb_corners(&bb) {
                    let w = obj.transform.apply_point(corner);
                    lo = lo.min(w);
                    hi = hi.max(w);
                }
            }
        }
        Ok((lo, hi))
    }

    /// X/Y shift (z = 0) that pulls a footprint `[lo, hi]` back onto the active
    /// plate's bed if it overhangs an edge. Zero when in-bounds or no bed bound.
    fn bed_xy_clamp_shift(&self, lo: Vec3, hi: Vec3) -> Vec3 {
        let Some(bed) = self.plates[self.active_plate].scene.bed.as_ref() else {
            return Vec3::ZERO;
        };
        let clamp = |l: f32, h: f32, bmin: f32, bmax: f32| -> f32 {
            if l < bmin {
                bmin - l
            } else if h > bmax {
                bmax - h
            } else {
                0.0
            }
        };
        Vec3::new(
            clamp(
                lo.x,
                hi.x,
                bed.extents.min[0] as f32,
                bed.extents.max[0] as f32,
            ),
            clamp(
                lo.y,
                hi.y,
                bed.extents.min[1] as f32,
                bed.extents.max[1] as f32,
            ),
            0.0,
        )
    }

    /// Exact lowest world-space Z of a selection, walking its true mesh
    /// vertices (not the bbox). Seating a freshly-rotated object on the plate
    /// needs the real contact point: the bbox over-approximates, so settling
    /// its min would leave the geometry floating. O(V) per call (~ms even for
    /// dense meshes) — only the seat step pays it; bounds stay cheap bbox.
    fn combined_world_min_z(&self, ids: &[ObjectId]) -> Result<f32, SceneOpError> {
        let active = self.active_plate;
        let mut min_z = f32::INFINITY;
        for &id in ids {
            let obj = self.plates[active]
                .scene
                .objects
                .get(&id)
                .ok_or(SceneOpError::UnknownObject(id))?;
            let mesh = self
                .meshes
                .get(&obj.mesh)
                .ok_or(SceneOpError::UnknownMesh(obj.mesh))?;
            for mesh in std::iter::once(mesh).chain(self.peg_meshes(id)) {
                for v in mesh.vertices.chunks_exact(3) {
                    let w = obj.transform.apply_point(Vec3::new(v[0], v[1], v[2]));
                    min_z = min_z.min(w.z);
                }
            }
        }
        Ok(min_z)
    }

    /// Apply a world-frame `rotation` to a selection **as one rigid unit**,
    /// then seat it flush on the build plate. Every object rotates about a
    /// shared `pivot`, keeping groups/assemblies intact:
    /// - `None` pivots about the selection's combined-AABB center — auto-orient,
    ///   where the FFI optimizer supplies the rotation and there's no anchor
    ///   point to preserve.
    /// - `Some(contact)` pivots about an explicit world point — "lay flat on…",
    ///   where `contact` is the clicked face's ray hit, so the spot the user
    ///   clicked stays put rather than swinging away.
    ///
    /// After rotating, the selection's **true lowest vertex** is translated to
    /// z=0 — the geometry sits exactly on the plate, no bounding-box gap, no
    /// float — and the footprint is pulled back onto the bed in X/Y if a
    /// reoriented edge overhangs. Because we just seated the object, its min Z
    /// is authoritative truth, so the conservative below-plate bounds check is
    /// suppressed for this op's output (it would false-positive on sub-micron FP
    /// noise); X/Y-out-of-volume and too-tall still report.
    pub fn orient_objects(
        &mut self,
        ids: &[ObjectId],
        rotation: Quat,
        pivot: Option<Vec3>,
    ) -> Result<Vec<SceneEvent>, SceneOpError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let active = self.active_plate;
        let plate_id = self.plates[active].id;

        // Resolve every target up front so the op is transactional: a missing
        // id (e.g. one deleted during auto-orient's unlocked optimizer run)
        // aborts here, before any transform is mutated, rather than leaving a
        // half-rotated, un-settled selection that emitted no events.
        for &id in ids {
            if !self.plates[active].scene.objects.contains_key(&id) {
                return Err(SceneOpError::UnknownObject(id));
            }
        }

        let pivot = match pivot {
            Some(p) => p,
            None => {
                let (lo, hi) = self.combined_bbox_aabb(ids)?;
                (lo + hi) * 0.5
            }
        };
        let rot_about_pivot = Transform::translation(pivot)
            .compose(Transform::rotation(rotation))
            .compose(Transform::translation(-pivot));
        for &id in ids {
            let obj = self.plates[active]
                .scene
                .objects
                .get_mut(&id)
                .ok_or(SceneOpError::UnknownObject(id))?;
            obj.transform = rot_about_pivot.compose(obj.transform);
        }

        // Seat the rotated selection flush on the plate (true lowest vertex to
        // z=0, exact contact) + clamp its footprint back onto the bed if a
        // reoriented edge overhangs.
        self.seat_clamp_emit(active, plate_id, ids)
    }

    /// Seat a freshly-transformed selection flush on the plate (its true
    /// lowest vertex to `z=0`, exact contact with no bbox gap) and clamp the
    /// footprint back onto the bed in X/Y, then emit one `ObjectUpdated` per
    /// object plus any post-seat out-of-bounds warnings. The shared tail of
    /// every placement op (orient, align-face) so the seat/clamp/bounds policy
    /// lives in one place.
    fn seat_clamp_emit(
        &mut self,
        active: usize,
        plate_id: PlateId,
        ids: &[ObjectId],
    ) -> Result<Vec<SceneEvent>, SceneOpError> {
        let min_z = self.combined_world_min_z(ids)?;
        let (lo, hi) = self.combined_bbox_aabb(ids)?;
        let mut shift = self.bed_xy_clamp_shift(lo, hi);
        // `min_z` is non-finite only when the whole selection has no vertices
        // (degenerate/empty meshes) — there's no geometry to seat, so leave Z
        // untouched rather than translating everything to -∞.
        shift.z = if min_z.is_finite() { -min_z } else { 0.0 };
        let settle = Transform::translation(shift);
        let mut events = Vec::with_capacity(ids.len());
        for &id in ids {
            let obj = self.plates[active].scene.objects.get_mut(&id).unwrap();
            obj.transform = settle.compose(obj.transform);
            events.push(SceneEvent::ObjectUpdated {
                plate_id,
                object: obj.clone(),
            });
        }
        for &id in ids {
            events.extend(self.out_of_bounds_event_seated(id));
        }
        Ok(events)
    }

    /// Align a face by yaw + coplanar slide. Yaw the selection in place (about
    /// its AABB center) by `yaw`, then slide it **along `slide_dir`** so the
    /// tracked world point reaches `target_coord` (its projection onto
    /// `slide_dir`) — making the clicked face coplanar with a reference face —
    /// then seat + clamp like `orient_objects`. `track_point` is the clicked
    /// world point on the selection *before* the yaw; it's followed through the
    /// rotation so the slide lands it exactly on the reference plane. `slide_dir`
    /// is a horizontal unit vector (the reference face's in-plane normal), so
    /// the slide never disturbs the z-seat.
    pub fn align_face_coplanar(
        &mut self,
        ids: &[ObjectId],
        yaw: Quat,
        slide_dir: Vec3,
        target_coord: f32,
        track_point: Vec3,
    ) -> Result<Vec<SceneEvent>, SceneOpError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let active = self.active_plate;
        let plate_id = self.plates[active].id;
        // Resolve every target up front (transactional, like orient_objects).
        for &id in ids {
            if !self.plates[active].scene.objects.contains_key(&id) {
                return Err(SceneOpError::UnknownObject(id));
            }
        }

        // In-place yaw about the selection's AABB center.
        let (lo, hi) = self.combined_bbox_aabb(ids)?;
        let pivot = (lo + hi) * 0.5;
        let rot = Transform::translation(pivot)
            .compose(Transform::rotation(yaw))
            .compose(Transform::translation(-pivot));
        for &id in ids {
            let obj = self.plates[active].scene.objects.get_mut(&id).unwrap();
            obj.transform = rot.compose(obj.transform);
        }

        // Slide along `slide_dir` so the (rotated) tracked point reaches the
        // reference plane: the clicked faces become coplanar.
        let track_after = rot.apply_point(track_point);
        let delta = target_coord - track_after.dot(slide_dir);
        let slide = Transform::translation(slide_dir * delta);
        for &id in ids {
            let obj = self.plates[active].scene.objects.get_mut(&id).unwrap();
            obj.transform = slide.compose(obj.transform);
        }

        // Seat on the plate + clamp the footprint (same tail as orient_objects).
        self.seat_clamp_emit(active, plate_id, ids)
    }

    /// Replace an object's transform wholesale. Used by
    /// auto-arrange and the gizmo's drag-finalization
    /// step.
    pub fn set_object_transform(
        &mut self,
        id: ObjectId,
        transform: Transform,
    ) -> Result<Vec<SceneEvent>, SceneOpError> {
        let active = self.active_plate;
        let plate_id = self.plates[active].id;
        let obj = self.plates[active]
            .scene
            .objects
            .get_mut(&id)
            .ok_or(SceneOpError::UnknownObject(id))?;
        obj.transform = transform;
        let clone = obj.clone();
        let mut events = vec![SceneEvent::ObjectUpdated {
            plate_id,
            object: clone,
        }];
        events.extend(self.out_of_bounds_event(id));
        Ok(events)
    }

    /// Delete one or more objects on the active plate. Removes
    /// from selection if present. Returns one `ObjectRemoved` event
    /// per id plus (if the selection changed) a `SelectionChanged`
    /// event, plus (if a material's last user was removed) a
    /// `MaterialSlotChanged` event.
    pub fn delete_objects(&mut self, ids: &[ObjectId]) -> Vec<SceneEvent> {
        let active = self.active_plate;
        let plate_id = self.plates[active].id;
        let mut events = Vec::new();
        let mut selection_changed = false;
        // Collect the materials the soon-to-be-removed objects use,
        // *before* removal — otherwise we can't tell which bindings
        // might now be orphaned.
        let mut removed_materials: BTreeSet<u8> = BTreeSet::new();
        let mut removed_meshes: BTreeSet<MeshId> = BTreeSet::new();
        {
            let plate = &mut self.plates[active].scene;
            for id in ids {
                if let Some(obj) = plate.objects.remove(id) {
                    removed_materials.insert(obj.extruder_id.unwrap_or(1));
                    removed_meshes.insert(obj.mesh);
                    // Drop the object's connector volumes too (sidecar + meshes).
                    if let Some(mods) = plate.object_modifiers.remove(id) {
                        removed_meshes.extend(mods.iter().map(|m| m.mesh));
                    }
                    // Hole markers are display-only (no mesh) but leave a stale
                    // sidecar that bloats the saved project if not removed.
                    plate.object_hole_markers.remove(id);
                    events.push(SceneEvent::ObjectRemoved {
                        plate_id,
                        object_id: *id,
                    });
                    if plate.selection.remove(id) {
                        selection_changed = true;
                    }
                }
            }
            if selection_changed {
                let mut sorted: Vec<ObjectId> = plate.selection.iter().copied().collect();
                sorted.sort();
                events.push(SceneEvent::SelectionChanged {
                    plate_id,
                    selected: sorted,
                });
            }
        }
        // Painted (MMU) materials are named by the mesh's paint, not any
        // object's extruder_id, so add the removed meshes' paint states to the
        // orphan candidates — else a face-painted material's slot binding
        // lingers after its object is gone. The prune itself re-checks
        // `materials_on_plate`, so a state still painted elsewhere is kept.
        for mid in &removed_meshes {
            removed_materials.extend(self.mesh_painted_materials(*mid));
        }
        if self.prune_orphan_material_bindings(active, &removed_materials) {
            events.push(SceneEvent::MaterialSlotChanged { plate_id });
        }
        self.prune_orphan_meshes(&removed_meshes);
        events
    }

    /// Drop meshes (and their primitive-cache entries) in `candidates` that no
    /// object on *any* plate references any more. Meshes are global and can be
    /// shared (primitive dedup, group volumes), so a mesh only goes once its last
    /// referencing object is gone — otherwise a deleted import's geometry lingers
    /// in `meshes` and bloats the next `.n3o` save (it's serialized per mesh).
    fn prune_orphan_meshes(&mut self, candidates: &BTreeSet<MeshId>) {
        if candidates.is_empty() {
            return;
        }
        let mut referenced: HashSet<MeshId> = self
            .plates
            .iter()
            .flat_map(|p| p.scene.objects.values())
            .map(|o| o.mesh)
            .collect();
        // Connector volumes are referenced only via the modifier sidecar, not
        // any object's `mesh` — count them or they'd be pruned right after a cut.
        for p in &self.plates {
            for mods in p.scene.object_modifiers.values() {
                referenced.extend(mods.iter().map(|m| m.mesh));
            }
        }
        for id in candidates {
            if !referenced.contains(id) {
                self.meshes.remove(id);
                self.primitive_cache.retain(|(_, _, mid)| mid != id);
            }
        }
    }

    /// Move a *set* of objects from one plate to another — the shared backend
    /// for auto-arrange spill (#3 phase 2) and a manual "send to plate" (#4).
    ///
    /// The ids are expanded to whole-group membership so a group is never split
    /// across plates, and each object's transform is **preserved**, so a moved
    /// group keeps its arrangement (there's no per-object recentering — that
    /// would scatter a group; anything
    /// off the target bed surfaces through the normal bounds check when that
    /// plate is viewed). Per-object overrides and each moved group's name travel
    /// with the objects; the source plate's selection and now-orphaned material
    /// bindings are pruned.
    pub fn move_objects_to_plate(
        &mut self,
        from_plate: PlateId,
        to_plate: PlateId,
        ids: &[ObjectId],
    ) -> Result<Vec<SceneEvent>, SceneOpError> {
        if from_plate == to_plate {
            return Err(SceneOpError::SamePlate(from_plate));
        }
        let from_idx = self
            .plate_index(from_plate)
            .ok_or(SceneOpError::UnknownPlate(from_plate))?;
        let to_idx = self
            .plate_index(to_plate)
            .ok_or(SceneOpError::UnknownPlate(to_plate))?;

        // Whole groups move together (no split across plates). expand_to_groups
        // returns an OrderedIds (authored order), so the objects keep their
        // relative order on the target plate.
        let ids = Self::expand_to_groups(&self.plates[from_idx].scene, ids);
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        for &id in ids.iter() {
            if !self.plates[from_idx].scene.objects.contains_key(&id) {
                return Err(SceneOpError::UnknownObject(id));
            }
        }

        let mut events = Vec::with_capacity(ids.len() * 2 + 2);
        let mut moved_materials: BTreeSet<u8> = BTreeSet::new();
        let mut moving_groups: HashSet<GroupId> = HashSet::new();
        let mut any_was_selected = false;
        for &id in ids.iter() {
            let obj = self.plates[from_idx].scene.objects.remove(&id).unwrap();
            if let Some(g) = obj.group {
                moving_groups.insert(g);
            }
            moved_materials.insert(obj.extruder_id.unwrap_or(1));
            let overrides = self.plates[from_idx].scene.object_overrides.remove(&id);
            // Cut-connector sidecars move with the object (same ObjectId), or the
            // target plate slices without pegs/holes and the source keeps stale
            // entries — a leaked `object_modifiers` pins its connector meshes
            // forever, a leaked `object_hole_markers` (meshless) bloats the save.
            let modifiers = self.plates[from_idx].scene.object_modifiers.remove(&id);
            let hole_markers = self.plates[from_idx].scene.object_hole_markers.remove(&id);
            if self.plates[from_idx].scene.selection.remove(&id) {
                any_was_selected = true;
            }
            self.plates[to_idx].scene.objects.push(obj.clone());
            if let Some(map) = overrides {
                self.plates[to_idx].scene.object_overrides.insert(id, map);
            }
            if let Some(mods) = modifiers {
                self.plates[to_idx].scene.object_modifiers.insert(id, mods);
            }
            if let Some(markers) = hole_markers {
                self.plates[to_idx].scene.object_hole_markers.insert(id, markers);
            }
            events.push(SceneEvent::ObjectRemoved {
                plate_id: from_plate,
                object_id: id,
            });
            events.push(SceneEvent::ObjectAdded {
                plate_id: to_plate,
                object: obj,
            });
        }

        // Carry each fully-moved group's metadata (its name) to the target.
        for g in moving_groups {
            if let Some(group) = self.plates[from_idx].scene.groups.remove(&g) {
                self.plates[to_idx].scene.groups.insert(g, group);
            }
        }

        // Carry the moved materials' slot bindings to the target so the objects
        // keep their material→slot assignment there (same-printer spill keeps it
        // exact). Don't clobber a binding the target already has; if the source
        // had none, let the target auto-bind. Done before the source prune below
        // so the source bindings are still present to copy.
        let mut target_bindings_changed = false;
        for &mat in &moved_materials {
            if self.plates[to_idx].material_to_slot.contains_key(&mat) {
                continue;
            }
            if let Some(&slot) = self.plates[from_idx].material_to_slot.get(&mat) {
                self.plates[to_idx].material_to_slot.insert(mat, slot);
                target_bindings_changed = true;
            } else {
                let before = self.plates[to_idx].material_to_slot.len();
                self.ensure_material_slot_on_plate(to_idx, mat);
                target_bindings_changed |= self.plates[to_idx].material_to_slot.len() != before;
            }
        }
        if target_bindings_changed {
            events.push(SceneEvent::MaterialSlotChanged {
                plate_id: to_plate,
            });
        }

        if any_was_selected {
            let mut sel: Vec<ObjectId> =
                self.plates[from_idx].scene.selection.iter().copied().collect();
            sel.sort();
            events.push(SceneEvent::SelectionChanged {
                plate_id: from_plate,
                selected: sel,
            });
        }
        if self.prune_orphan_material_bindings(from_idx, &moved_materials) {
            events.push(SceneEvent::MaterialSlotChanged {
                plate_id: from_plate,
            });
        }
        Ok(events)
    }

    // ---- Bounds / helpers ----------------------------------------

    /// Compute the world-space bounding box of all visible objects
    /// on the active plate. Used by `Frame All` in the renderer.
    pub fn visible_bounds(&self) -> Option<BoundingBox> {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        let mut any = false;
        for obj in self.active_plate().scene.objects.values() {
            if !obj.visible {
                continue;
            }
            let mesh = match self.meshes.get(&obj.mesh) {
                Some(m) => m,
                None => continue,
            };
            let bb = &mesh.bounding_box;
            for &x in &[bb.min[0] as f32, bb.max[0] as f32] {
                for &y in &[bb.min[1] as f32, bb.max[1] as f32] {
                    for &z in &[bb.min[2] as f32, bb.max[2] as f32] {
                        let p = obj.transform.apply_point(Vec3::new(x, y, z));
                        min = min.min(p);
                        max = max.max(p);
                        any = true;
                    }
                }
            }
        }
        if any {
            Some(BoundingBox {
                min: [min.x as f64, min.y as f64, min.z as f64],
                max: [max.x as f64, max.y as f64, max.z as f64],
            })
        } else {
            None
        }
    }

    /// Check `object_id` on the active plate against its bed and
    /// emit warnings for every reason it's out of bounds. No bed =
    /// no check (silently). Designed to be called by every
    /// transform op so the UI can flash a non-blocking warning the
    /// instant the user nudges an object off the plate.
    fn out_of_bounds_event(&self, object_id: ObjectId) -> Option<SceneEvent> {
        let plate_id = self.active_plate().id;
        let plate = &self.active_plate().scene;
        let bed = plate.bed.as_ref()?;
        let obj = plate.objects.get(&object_id)?;
        let mesh = self.meshes.get(&obj.mesh)?;
        let reasons = bed::object_out_of_bounds(obj, mesh, bed);
        if reasons.is_empty() {
            None
        } else {
            Some(SceneEvent::ObjectOutOfBounds {
                plate_id,
                object_id,
                reasons,
            })
        }
    }

    /// Like `out_of_bounds_event`, but for an object a placement op just seated
    /// flush on the plate. That op holds the absolute truth that the object's
    /// lowest point sits at z=0, so the conservative bbox below-plate check is
    /// dropped — it would false-positive on sub-micron FP noise in the settled
    /// min. X/Y-out-of-volume, too-tall, and exclusion-zone reasons still report.
    fn out_of_bounds_event_seated(&self, object_id: ObjectId) -> Option<SceneEvent> {
        let plate_id = self.active_plate().id;
        let plate = &self.active_plate().scene;
        let bed = plate.bed.as_ref()?;
        let obj = plate.objects.get(&object_id)?;
        let mesh = self.meshes.get(&obj.mesh)?;
        let reasons: Vec<_> = bed::object_out_of_bounds(obj, mesh, bed)
            .into_iter()
            .filter(|r| !matches!(r, bed::OutOfBoundsReason::BelowBuildPlate))
            .collect();
        if reasons.is_empty() {
            None
        } else {
            Some(SceneEvent::ObjectOutOfBounds {
                plate_id,
                object_id,
                reasons,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::project::mutation::test_support::*;

    #[test]
    fn group_objects_shares_id_and_name_then_ungroups() {
        let mut p = Project::default();
        let (_, a) = add_cube(&mut p);
        let (_, b) = add_cube(&mut p);

        let events = p.group_objects(&[a, b], "Bracket".into()).unwrap();
        assert!(!events.is_empty());
        let plate = p.active_plate();
        let ga = plate.scene.objects[&a].group.expect("a grouped");
        assert_eq!(plate.scene.objects[&b].group, Some(ga), "shared group id");
        assert_eq!(
            plate.scene.groups.get(&ga).map(|g| g.name.as_str()),
            Some("Bracket"),
        );

        p.ungroup_objects(ga);
        let plate = p.active_plate();
        assert_eq!(plate.scene.objects[&a].group, None);
        assert_eq!(plate.scene.objects[&b].group, None);
        assert!(plate.scene.groups.is_empty(), "name dropped on ungroup");
    }

    #[test]
    fn grouping_fewer_than_two_objects_is_a_noop() {
        let mut p = Project::default();
        let (_, a) = add_cube(&mut p);
        assert!(p.group_objects(&[a], "G".into()).unwrap().is_empty());
        assert_eq!(p.active_plate().scene.objects[&a].group, None);
    }

    #[test]
    fn regrouping_dissolves_a_group_left_with_one_member() {
        let mut p = Project::default();
        let (_, a) = add_cube(&mut p);
        let (_, b) = add_cube(&mut p);
        let (_, c) = add_cube(&mut p);
        p.group_objects(&[a, b], "G1".into()).unwrap();
        let g1 = p.active_plate().scene.objects[&a].group.unwrap();
        // Move b into a new group with c — G1 is left with only a, so it
        // dissolves (a group of one isn't a group).
        p.group_objects(&[b, c], "G2".into()).unwrap();
        let plate = p.active_plate();
        assert_eq!(plate.scene.objects[&a].group, None, "a's group dissolved");
        assert!(!plate.scene.groups.contains_key(&g1));
        let gbc = plate.scene.objects[&b].group.expect("b grouped");
        assert_eq!(plate.scene.objects[&c].group, Some(gbc));
    }

    /// Synthetic kept half (no FFI) — geometry is irrelevant to the
    /// register/remove/group bookkeeping under test.
    fn half(side: CutSide) -> CutHalfOut {
        CutHalfOut { side, vertices: vec![0.0; 9], indices: vec![0, 1, 2], paint: None, modifiers: vec![], hole_markers: vec![] }
    }

    #[test]
    fn apply_cut_single_object_both_sides_makes_two_ungrouped() {
        let mut p = Project::default();
        let (mesh, a) = add_cube(&mut p);
        let tf = p.active_plate().scene.objects[&a].transform;
        let res = CutResult {
            source_id: a,
            transform: tf,
            base_name: "cube".into(),
            extruder_id: Some(1),
            source_group: None,
            halves: vec![half(CutSide::Pos), half(CutSide::Neg)],
            dowels: vec![],
        };
        let (new_ids, _events) = p.apply_cut(vec![res]);
        assert_eq!(new_ids.len(), 2, "both sides → two objects");
        assert!(p.active_plate().scene.objects.get(&a).is_none(), "source removed");
        assert!(p.meshes.get(&mesh).is_none(), "orphan source mesh pruned");
        let plate = p.active_plate();
        let names: Vec<&str> =
            new_ids.iter().map(|id| plate.scene.objects[id].name.as_str()).collect();
        assert!(names.contains(&"cube (A)") && names.contains(&"cube (B)"));
        for id in &new_ids {
            let o = &plate.scene.objects[id];
            assert_eq!(o.group, None, "ungroduped source → ungrouped halves");
            assert!(p.meshes[&o.mesh].paint_colors.is_none(), "unpainted source → unpainted halves");
        }
        assert_eq!(
            plate.scene.selection.iter().copied().collect::<HashSet<_>>(),
            new_ids.iter().copied().collect::<HashSet<_>>(),
            "halves selected",
        );
    }

    #[test]
    fn apply_cut_single_kept_side_names_it_cut() {
        let mut p = Project::default();
        let (_, a) = add_cube(&mut p);
        let tf = p.active_plate().scene.objects[&a].transform;
        let res = CutResult {
            source_id: a,
            transform: tf,
            base_name: "cube".into(),
            extruder_id: None,
            source_group: None,
            halves: vec![half(CutSide::Pos)],
            dowels: vec![],
        };
        let (new_ids, _) = p.apply_cut(vec![res]);
        assert_eq!(new_ids.len(), 1);
        assert_eq!(p.active_plate().scene.objects[&new_ids[0]].name, "cube (cut)");
    }

    #[test]
    fn apply_cut_carries_paint_onto_the_half_mesh() {
        let mut p = Project::default();
        let (_, a) = add_cube(&mut p);
        let tf = p.active_plate().scene.objects[&a].transform;
        let painted = CutHalfOut {
            side: CutSide::Pos,
            vertices: vec![0.0; 9],
            indices: vec![0, 1, 2],
            paint: Some(vec!["4".into()]),
            modifiers: vec![],
            hole_markers: vec![],
        };
        let res = CutResult {
            source_id: a,
            transform: tf,
            base_name: "cube".into(),
            extruder_id: None,
            source_group: None,
            halves: vec![painted],
            dowels: vec![],
        };
        let (new_ids, _) = p.apply_cut(vec![res]);
        let mesh = p.active_plate().scene.objects[&new_ids[0]].mesh;
        assert_eq!(
            p.meshes[&mesh].paint_colors.as_deref(),
            Some(&vec!["4".to_string()]),
            "the half's paint reaches the registered mesh",
        );
    }

    #[test]
    fn cut_targets_carry_existing_connectors_for_recut() {
        // Re-cutting a previously-cut object must carry its pegs/holes/markers so
        // they reach the new halves rather than being dropped with the source.
        let mut p = Project::default();
        let (_, a) = add_cube(&mut p);
        let active = p.active_plate;
        let peg = p.register_mesh(NewMesh {
            vertices: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            indices: vec![0, 1, 2],
            paint_colors: None,
            support_paint: None,
            bounding_box: BoundingBox { min: [0.0; 3], max: [1.0, 1.0, 0.0] },
            provenance: MeshProvenance::Primitive("peg".into()),
        });
        p.plates[active]
            .scene
            .object_modifiers
            .insert(a, vec![Modifier { mesh: peg, kind: ModifierKind::Peg }]);
        p.plates[active].scene.object_hole_markers.insert(
            a,
            vec![crate::core::scene::state::HoleMarker {
                shape: crate::core::scene::state::HoleMarkerShape::Circle,
                radius: 1.0,
                center: [0.0, 0.0, 0.0],
                normal: [0.0, 0.0, 1.0],
                u_axis: [1.0, 0.0, 0.0],
            }],
        );
        let targets = p.cut_targets(&[a]).unwrap();
        assert_eq!(targets[0].modifiers.len(), 1, "peg modifier carried into the cut target");
        assert_eq!(targets[0].hole_markers.len(), 1, "hole marker carried into the cut target");
    }

    #[test]
    fn apply_cut_registers_dowel_pins_as_objects() {
        let mut p = Project::default();
        let (_, a) = add_cube(&mut p);
        let tf = p.active_plate().scene.objects[&a].transform;
        let res = CutResult {
            source_id: a,
            transform: tf,
            base_name: "cube".into(),
            extruder_id: None,
            source_group: None,
            halves: vec![half(CutSide::Pos), half(CutSide::Neg)],
            dowels: vec![(vec![0.0; 9], vec![0, 1, 2])],
        };
        let (new_ids, _) = p.apply_cut(vec![res]);
        assert_eq!(new_ids.len(), 3, "two halves + one pin");
        let plate = p.active_plate();
        assert!(
            new_ids.iter().any(|id| plate.scene.objects[id].name == "cube pin 1"),
            "the dowel pin is registered as its own object",
        );
    }

    #[test]
    fn apply_cut_group_both_sides_makes_two_per_side_groups() {
        let mut p = Project::default();
        let (_, a) = add_cube(&mut p);
        let (_, b) = add_cube(&mut p);
        p.group_objects(&[a, b], "Bracket".into()).unwrap();
        let g = p.active_plate().scene.objects[&a].group.unwrap();
        let (ta, tb) = {
            let plate = p.active_plate();
            (plate.scene.objects[&a].transform, plate.scene.objects[&b].transform)
        };
        let results = vec![
            CutResult {
                source_id: a,
                transform: ta,
                base_name: "a".into(),
                extruder_id: None,
                source_group: Some(g),
                halves: vec![half(CutSide::Pos), half(CutSide::Neg)],
                dowels: vec![],
            },
            CutResult {
                source_id: b,
                transform: tb,
                base_name: "b".into(),
                extruder_id: None,
                source_group: Some(g),
                halves: vec![half(CutSide::Pos), half(CutSide::Neg)],
                dowels: vec![],
            },
        ];
        let (new_ids, _) = p.apply_cut(results);
        assert_eq!(new_ids.len(), 4);
        let plate = p.active_plate();
        let mut by_group: HashMap<GroupId, usize> = HashMap::new();
        for id in &new_ids {
            let g = plate.scene.objects[id].group.expect("half is grouped");
            *by_group.entry(g).or_insert(0) += 1;
        }
        assert_eq!(by_group.len(), 2, "one fresh group per side");
        assert!(by_group.values().all(|&c| c == 2), "each side keeps both members");
        assert!(!plate.scene.groups.contains_key(&g), "consumed source group dropped");
        // Per-side groups named after the source.
        let names: HashSet<&str> = by_group
            .keys()
            .map(|g| plate.scene.groups[g].name.as_str())
            .collect();
        assert!(names.contains("Bracket (A)") && names.contains("Bracket (B)"));
    }

    #[test]
    fn group_expansion_is_opt_in_select_stays_individual() {
        let mut p = Project::default();
        let (_, a) = add_cube(&mut p);
        let (_, b) = add_cube(&mut p);
        let (_, c) = add_cube(&mut p);
        p.group_objects(&[a, b], "G".into()).unwrap();

        // The canvas's expansion helper pulls in the whole group, in authored
        // order (a, b registered before c).
        assert_eq!(
            p.group_expanded_ids(&[a]).to_vec(),
            vec![a, b],
            "grouped object expands to its group",
        );
        assert_eq!(
            p.group_expanded_ids(&[c]).to_vec(),
            vec![c],
            "ungrouped passes through"
        );

        // ...but `select` itself (the object-list path) stays individual.
        p.select(&[a], SelectMode::Replace);
        let sel = &p.active_plate().scene.selection;
        assert_eq!(sel.len(), 1, "select does not auto-expand");
        assert!(sel.contains(&a));

        // Toggle a multi-id set as a unit (the canvas group-toggle path):
        // a is selected, b is not → not all selected → add both.
        p.select(&[a, b], SelectMode::Toggle);
        assert!(p.active_plate().scene.selection.contains(&b));
        // Both selected now → toggle removes both.
        p.select(&[a, b], SelectMode::Toggle);
        assert!(p.active_plate().scene.selection.is_empty());
    }

    #[test]
    fn empty_project_has_no_visible_bounds() {
        let p = Project::default();
        assert!(p.visible_bounds().is_none());
    }

    #[test]
    fn monotonic_ids_dont_reuse() {
        let mut p = Project::default();
        assert_eq!(p.next_mesh_id(), MeshId(1));
        assert_eq!(p.next_mesh_id(), MeshId(2));
        assert_eq!(p.next_object_id(), ObjectId(1));
        assert_eq!(p.next_object_id(), ObjectId(2));
    }

    #[test]
    fn register_mesh_allocates_monotonically() {
        let mut p = Project::default();
        let id = p.register_mesh(unit_cube_mesh());
        assert_eq!(id, MeshId(1));
        let id2 = p.register_mesh(unit_cube_mesh());
        assert_eq!(id2, MeshId(2));
    }

    #[test]
    fn visible_bounds_unions_objects() {
        let mut p = Project::default();
        let mesh_id = p.register_mesh(unit_cube_mesh());
        let _ = p.register_object(NewSceneObject::at_origin(mesh_id, "cube0"));
        let _ = p.register_object(NewSceneObject {
            transform: Transform::translation(Vec3::new(10.0, 0.0, 0.0)),
            name: "cube1".into(),
            ..NewSceneObject::at_origin(mesh_id, "cube1")
        });

        let bb = p.visible_bounds().expect("two visible objects");
        assert_eq!(bb.min, [0.0, 0.0, 0.0]);
        assert!((bb.max[0] - 11.0).abs() < 1e-3, "got {bb:?}");
    }

    #[test]
    fn invisible_objects_skipped_in_bounds() {
        let mut p = Project::default();
        let mesh_id = p.register_mesh(unit_cube_mesh());
        let _ = p.register_object(NewSceneObject {
            visible: false,
            transform: Transform::translation(Vec3::new(100.0, 0.0, 0.0)),
            ..NewSceneObject::at_origin(mesh_id, "hidden")
        });
        assert!(p.visible_bounds().is_none());
    }

    #[test]
    fn select_no_op_emits_no_event() {
        let mut p = Project::default();
        let (_mesh, obj) = add_cube(&mut p);
        let _ = p.select(&[obj], SelectMode::Replace);
        let events = p.select(&[obj], SelectMode::Replace);
        assert!(events.is_empty(), "re-selecting same set is a no-op");
    }

    #[test]
    fn select_unknown_object_is_skipped() {
        let mut p = Project::default();
        let (_mesh, obj) = add_cube(&mut p);
        let events = p.select(&[obj, ObjectId(9999)], SelectMode::Replace);
        match &events[0] {
            SceneEvent::SelectionChanged { selected, .. } => {
                assert_eq!(selected, &vec![obj], "unknown id filtered out");
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn deselect_all_emits_event_when_selection_was_nonempty() {
        let mut p = Project::default();
        let (_mesh, obj) = add_cube(&mut p);
        let _ = p.select(&[obj], SelectMode::Replace);
        let events = p.deselect_all();
        match &events[0] {
            SceneEvent::SelectionChanged { selected, .. } => assert!(selected.is_empty()),
            _ => unreachable!(),
        }
        // Second deselect_all is a no-op.
        let events = p.deselect_all();
        assert!(events.is_empty());
    }

    #[test]
    fn delete_objects_prunes_the_orphaned_mesh() {
        let mut p = Project::default();
        let (mesh1, obj1) = add_cube(&mut p);
        let (mesh2, _obj2) = add_cube(&mut p);
        assert_ne!(mesh1, mesh2, "distinct meshes");
        p.delete_objects(&[obj1]);
        assert!(!p.meshes.contains_key(&mesh1), "deleted object's mesh is GC'd");
        assert!(p.meshes.contains_key(&mesh2), "the surviving object's mesh stays");
    }

    #[test]
    fn delete_objects_keeps_a_mesh_another_object_still_uses() {
        let mut p = Project::default();
        let mesh = p.register_mesh(unit_cube_mesh());
        let obj1 = p.register_object(NewSceneObject::at_origin(mesh, "a"));
        let _obj2 = p.register_object(NewSceneObject::at_origin(mesh, "b"));
        p.delete_objects(&[obj1]);
        assert!(p.meshes.contains_key(&mesh), "shared mesh kept while obj2 uses it");
    }

    #[test]
    fn delete_objects_clears_selection_for_deleted_ids() {
        let mut p = Project::default();
        let (_mesh, obj1) = add_cube(&mut p);
        let (_mesh2, obj2) = add_cube(&mut p);
        let _ = p.select(&[obj1, obj2], SelectMode::Replace);
        let events = p.delete_objects(&[obj1]);
        assert_eq!(events.len(), 2);
        assert!(matches!(
            events[0],
            SceneEvent::ObjectRemoved { object_id, .. } if object_id == obj1,
        ));
        match &events[1] {
            SceneEvent::SelectionChanged { selected, .. } => {
                assert_eq!(selected, &vec![obj2]);
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn unknown_object_ops_return_error() {
        let mut p = Project::default();
        let bad = ObjectId(42);
        assert!(matches!(
            p.set_object_transform(bad, Transform::IDENTITY),
            Err(SceneOpError::UnknownObject(_))
        ));
        assert!(p.delete_objects(&[bad]).is_empty());
    }

    #[test]
    fn project_round_trips_via_json() {
        let mut p = Project::default();
        let mesh_id = p.register_mesh(unit_cube_mesh());
        let _ = p.register_object(NewSceneObject {
            mesh: mesh_id,
            transform: Transform::translation(Vec3::new(5.0, 5.0, 0.0)),
            name: "test-cube".into(),
            visible: true,
            extruder_id: Some(2),
            group: None,
        });

        let json = serde_json::to_string(&p).unwrap();
        let parsed: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.meshes.len(), 1);
        assert_eq!(parsed.active_plate().scene.objects.len(), 1);
        let obj = parsed.active_plate().scene.objects.values().next().unwrap();
        assert_eq!(obj.name, "test-cube");
        assert_eq!(obj.extruder_id, Some(2));
    }

    #[test]
    fn auto_orient_identity_preserves_xy_and_settles_on_bed() {
        let mut p = Project::default();
        let (_, obj) = add_cube(&mut p);
        // Place it off-origin and floating above the bed.
        p.set_object_transform(obj, Transform::translation(Vec3::new(50.0, 30.0, 7.0)))
            .unwrap();
        let mesh_bb = p.meshes.values().next().unwrap().bounding_box;
        let center_local = Vec3::new(
            ((mesh_bb.min[0] + mesh_bb.max[0]) * 0.5) as f32,
            ((mesh_bb.min[1] + mesh_bb.max[1]) * 0.5) as f32,
            ((mesh_bb.min[2] + mesh_bb.max[2]) * 0.5) as f32,
        );
        let before = p
            .active_plate()
            .scene
            .objects
            .get(&obj)
            .unwrap()
            .transform
            .apply_point(center_local);
        // Identity rotation (engine "already optimal" / best = z-up): orient in
        // place — keep the XY center, just settle onto the bed.
        p.orient_objects(&[obj], Quat::IDENTITY, None).unwrap();

        let xform = p.active_plate().scene.objects.get(&obj).unwrap().transform;
        let wc = xform.apply_point(center_local);
        assert!(
            (wc.x - before.x).abs() < 1e-3,
            "x moved {} -> {}",
            before.x,
            wc.x
        );
        assert!(
            (wc.y - before.y).abs() < 1e-3,
            "y moved {} -> {}",
            before.y,
            wc.y
        );
        // Settled on the bed: min Z ≈ 0.
        let mut min_z = f32::INFINITY;
        for &x in &[mesh_bb.min[0] as f32, mesh_bb.max[0] as f32] {
            for &y in &[mesh_bb.min[1] as f32, mesh_bb.max[1] as f32] {
                for &z in &[mesh_bb.min[2] as f32, mesh_bb.max[2] as f32] {
                    min_z = min_z.min(xform.apply_point(Vec3::new(x, y, z)).z);
                }
            }
        }
        assert!(
            min_z.abs() < 1e-3,
            "min_z={min_z} (expected resting on the plate)"
        );
    }

    #[test]
    fn auto_orient_clamps_overhanging_footprint_onto_bed() {
        let mut p = Project::default();
        let (_, obj) = add_cube(&mut p);
        let bed_max_x = {
            let bed = p.active_plate().scene.bed.as_ref().unwrap();
            bed.extents.max[0] as f32
        };
        // Push the unit cube partly off the +X edge of the bed.
        p.set_object_transform(obj, Transform::translation(Vec3::new(bed_max_x - 0.5, 30.0, 0.0)))
            .unwrap();
        // Even with no reorientation, the clamp must pull the footprint back in.
        p.orient_objects(&[obj], Quat::IDENTITY, None).unwrap();

        let xform = p.active_plate().scene.objects.get(&obj).unwrap().transform;
        let mesh_bb = p.meshes.values().next().unwrap().bounding_box;
        let mut max_x = f32::NEG_INFINITY;
        for &x in &[mesh_bb.min[0] as f32, mesh_bb.max[0] as f32] {
            for &y in &[mesh_bb.min[1] as f32, mesh_bb.max[1] as f32] {
                for &z in &[mesh_bb.min[2] as f32, mesh_bb.max[2] as f32] {
                    max_x = max_x.max(xform.apply_point(Vec3::new(x, y, z)).x);
                }
            }
        }
        assert!(
            max_x <= bed_max_x + 1e-3,
            "footprint not clamped onto bed: max_x={max_x} > {bed_max_x}"
        );
    }

    #[test]
    fn lay_flat_on_pivots_about_contact_and_settles_lowest_to_z_zero() {
        let mut p = Project::default();
        let (_, obj) = add_cube(&mut p);
        p.set_object_transform(obj, Transform::translation(Vec3::new(40.0, 40.0, 20.0)))
            .unwrap();
        // A world point on the object's surface (the ray hit) and a lay-flat
        // rotation. The selection pivots about `contact` — so the clicked spot's
        // XY stays put when nothing overhangs — then seats so the object's *true
        // lowest vertex* (not the contact point) rests on the plate.
        let contact = Vec3::new(40.5, 40.5, 25.0);
        let old = p.active_plate().scene.objects.get(&obj).unwrap().transform;
        let local = old.to_mat4().inverse().transform_point3(contact);
        let rot = Quat::from_axis_angle(Vec3::X, 0.7);
        p.orient_objects(&[obj], rot, Some(contact)).unwrap();

        let new = p.active_plate().scene.objects.get(&obj).unwrap().transform;
        // The contact point pivots in place: its XY is preserved (no overhang).
        let placed = new.apply_point(local);
        assert!(
            (placed.x - contact.x).abs() < 1e-3,
            "contact x moved to {}",
            placed.x
        );
        assert!(
            (placed.y - contact.y).abs() < 1e-3,
            "contact y moved to {}",
            placed.y
        );
        // The true lowest vertex of the cube rests exactly on the plate.
        let mesh_bb = p.meshes.values().next().unwrap().bounding_box;
        let mut min_z = f32::INFINITY;
        for &x in &[mesh_bb.min[0] as f32, mesh_bb.max[0] as f32] {
            for &y in &[mesh_bb.min[1] as f32, mesh_bb.max[1] as f32] {
                for &z in &[mesh_bb.min[2] as f32, mesh_bb.max[2] as f32] {
                    min_z = min_z.min(new.apply_point(Vec3::new(x, y, z)).z);
                }
            }
        }
        assert!(
            min_z.abs() < 1e-3,
            "lowest vertex should rest at z=0, got {min_z}"
        );
    }

    #[test]
    fn lay_flat_on_clamps_overhanging_footprint_back_onto_bed() {
        let mut p = Project::default();
        let (_, obj) = add_cube(&mut p);
        let bed_max_x = p.active_plate().scene.bed.as_ref().unwrap().extents.max[0] as f32;
        // Unit cube hanging 0.5 off the +X edge. The post-rotation X/Y clamp must
        // pull the footprint back on (here identity rotation, so it's purely the
        // clamp doing the work).
        p.set_object_transform(obj, Transform::translation(Vec3::new(bed_max_x - 0.5, 30.0, 0.0)))
            .unwrap();
        p.orient_objects(
            &[obj],
            Quat::IDENTITY,
            Some(Vec3::new(bed_max_x, 30.5, 0.5)),
        )
        .unwrap();

        let xform = p.active_plate().scene.objects.get(&obj).unwrap().transform;
        let mesh_bb = p.meshes.values().next().unwrap().bounding_box;
        let mut max_x = f32::NEG_INFINITY;
        for &x in &[mesh_bb.min[0] as f32, mesh_bb.max[0] as f32] {
            for &y in &[mesh_bb.min[1] as f32, mesh_bb.max[1] as f32] {
                for &z in &[mesh_bb.min[2] as f32, mesh_bb.max[2] as f32] {
                    max_x = max_x.max(xform.apply_point(Vec3::new(x, y, z)).x);
                }
            }
        }
        assert!(
            max_x <= bed_max_x + 1e-3,
            "footprint not clamped: max_x={max_x}"
        );
    }

    #[test]
    fn orient_settles_true_lowest_vertex_and_suppresses_false_below_plate() {
        use crate::core::scene::bed::{object_out_of_bounds, OutOfBoundsReason};
        let mut p = Project::default();
        let r = 10.0_f32;
        let (mesh_id, obj) = add_octahedron(&mut p, r);
        // Center it well inside the bed and float it, so only Z is in play.
        let (cx, cy) = {
            let e = p.active_plate().scene.bed.as_ref().unwrap().extents;
            (
                ((e.min[0] + e.max[0]) * 0.5) as f32,
                ((e.min[1] + e.max[1]) * 0.5) as f32,
            )
        };
        p.set_object_transform(obj, Transform::translation(Vec3::new(cx, cy, 50.0)))
            .unwrap();
        // A 45° tilt: the bbox corner now dips well below the true geometry, so a
        // bbox-based settle would leave the solid floating and the bbox bounds
        // check over-reports below-plate.
        let rot = Quat::from_axis_angle(Vec3::X, std::f32::consts::FRAC_PI_4);
        let events = p.orient_objects(&[obj], rot, None).unwrap();

        // The *true* lowest vertex — not a bbox corner — rests on the plate. If
        // this regressed to a bbox settle it'd float at ≈ +0.71·r.
        let xform = p.active_plate().scene.objects.get(&obj).unwrap().transform;
        let mut true_min_z = f32::INFINITY;
        for v in p.meshes.get(&mesh_id).unwrap().vertices.chunks_exact(3) {
            true_min_z = true_min_z.min(xform.apply_point(Vec3::new(v[0], v[1], v[2])).z);
        }
        assert!(
            true_min_z.abs() < 1e-3,
            "true lowest vertex should rest at z=0, got {true_min_z}"
        );

        // The conservative bbox check *does* flag below-plate here (the rotated
        // box corner sits at ≈ -0.71·r)…
        let raw = {
            let plate = p.active_plate();
            let oobj = plate.scene.objects.get(&obj).unwrap();
            let omesh = p.meshes.get(&oobj.mesh).unwrap();
            object_out_of_bounds(oobj, omesh, plate.scene.bed.as_ref().unwrap())
        };
        assert!(
            raw.iter()
                .any(|r| matches!(r, OutOfBoundsReason::BelowBuildPlate)),
            "test premise: bbox check should over-report below-plate, got {raw:?}"
        );
        // …but the orient op, which just seated the solid flush, suppresses that
        // false positive — no below-plate warning reaches the UI.
        let emitted_below_plate = events.iter().any(|e| {
            matches!(
                e,
                SceneEvent::ObjectOutOfBounds { reasons, .. }
                    if reasons.iter().any(|r| matches!(r, OutOfBoundsReason::BelowBuildPlate))
            )
        });
        assert!(
            !emitted_below_plate,
            "below-plate must be suppressed for a freshly-seated object"
        );
    }

    #[test]
    fn orient_keeps_too_tall_warning_after_seating() {
        use crate::core::scene::bed::{BoundsAxis, OutOfBoundsReason};
        let mut p = Project::default();
        let (_, obj) = add_cube(&mut p);
        // A column taller than the build volume. Seating drops it to z=0, so it's
        // not below-plate — but it pierces the Z ceiling, which must still warn:
        // the seated suppression only drops below-plate, not too-tall.
        let z_max = p.active_plate().scene.bed.as_ref().unwrap().extents.max[2] as f32;
        p.set_object_transform(obj, Transform::scale(Vec3::new(1.0, 1.0, z_max * 2.0 + 100.0)))
            .unwrap();
        let events = p.orient_objects(&[obj], Quat::IDENTITY, None).unwrap();

        let reasons = events
            .iter()
            .find_map(|e| match e {
                SceneEvent::ObjectOutOfBounds {
                    object_id, reasons, ..
                } if *object_id == obj => Some(reasons.clone()),
                _ => None,
            })
            .expect("a too-tall object must still emit an out-of-bounds event");
        assert!(
            reasons.iter().any(|r| matches!(
                r,
                OutOfBoundsReason::OutOfBuildVolume {
                    axis: BoundsAxis::Z
                }
            )),
            "too-tall (Z) must still report after seating, got {reasons:?}"
        );
        assert!(
            !reasons
                .iter()
                .any(|r| matches!(r, OutOfBoundsReason::BelowBuildPlate)),
            "a seated object is not below-plate, got {reasons:?}"
        );
    }

    #[test]
    fn placement_seats_pegs_but_ignores_holes() {
        // A cut half's protruding peg must be seated + bounded with the object;
        // hole (cavity) volumes must not.
        let mut p = Project::default();
        let tri = |p: &mut Project, name: &str, z: f32| -> MeshId {
            p.register_mesh(NewMesh {
                vertices: vec![0.0, 0.0, z, 1.0, 0.0, z, 0.0, 1.0, z],
                indices: vec![0, 1, 2],
                paint_colors: None,
                support_paint: None,
                bounding_box: BoundingBox {
                    min: [0.0, 0.0, z as f64],
                    max: [1.0, 1.0, z as f64],
                },
                provenance: MeshProvenance::Primitive(name.into()),
            })
        };
        let main = tri(&mut p, "main", 0.0);
        let obj = p.register_object(NewSceneObject::at_origin(main, "half"));
        let peg = tri(&mut p, "peg", -0.5); // protrudes below the main mesh
        let hole = tri(&mut p, "hole", -0.9); // cavity — must be ignored
        let active = p.active_plate;
        p.plates[active].scene.object_modifiers.insert(
            obj,
            vec![
                Modifier { mesh: peg, kind: ModifierKind::Peg },
                Modifier { mesh: hole, kind: ModifierKind::Hole },
            ],
        );
        // Seat point folds in the peg (-0.5), not the hole below it.
        let min_z = p.combined_world_min_z(&[obj]).unwrap();
        assert!((min_z + 0.5).abs() < 1e-6, "seat should use the peg's lowest, got {min_z}");
        // Bounds fold in the peg too.
        let (lo, _) = p.combined_bbox_aabb(&[obj]).unwrap();
        assert!((lo.z + 0.5).abs() < 1e-6, "bbox should include the peg, got {}", lo.z);
        // World mesh = main (3) + peg (3); hole/marker excluded.
        let (verts, _) = p.objects_world_mesh(&[obj]).unwrap();
        assert_eq!(verts.len() / 3, 6, "world mesh is main + peg only");
    }

    #[test]
    fn orient_with_empty_mesh_does_not_collapse_to_negative_infinity() {
        let mut p = Project::default();
        // A mesh with no geometry — pathological, but the true-lowest-vertex
        // settle must not translate it to z = -∞ (a non-finite min_z).
        let mesh = NewMesh {
            vertices: vec![],
            indices: vec![],
            paint_colors: None,
            support_paint: None,
            bounding_box: BoundingBox {
                min: [0.0, 0.0, 0.0],
                max: [0.0, 0.0, 0.0],
            },
            provenance: MeshProvenance::Primitive("empty".into()),
        };
        let mesh_id = p.register_mesh(mesh);
        let obj = p.register_object(NewSceneObject::at_origin(mesh_id, "empty"));
        p.set_object_transform(obj, Transform::translation(Vec3::new(10.0, 10.0, 5.0)))
            .unwrap();
        p.orient_objects(&[obj], Quat::IDENTITY, None).unwrap();

        let xform = p.active_plate().scene.objects.get(&obj).unwrap().transform;
        assert!(
            xform.apply_point(Vec3::ZERO).is_finite(),
            "empty-mesh orient produced a non-finite transform"
        );
    }

    #[test]
    fn orient_aborts_without_mutating_a_sibling_when_an_id_is_missing() {
        let mut p = Project::default();
        let (_, a) = add_cube(&mut p);
        p.set_object_transform(a, Transform::translation(Vec3::new(20.0, 20.0, 9.0)))
            .unwrap();
        let before = p.active_plate().scene.objects.get(&a).unwrap().transform;
        // A bogus id alongside a real one (the auto-orient lock-release window
        // can drop an id mid-flight). The op must abort up front and leave the
        // surviving object completely untouched — no half-applied rotation.
        let missing = ObjectId(u64::MAX);
        let res = p.orient_objects(
            &[a, missing],
            Quat::from_axis_angle(Vec3::X, 0.5),
            Some(Vec3::ZERO),
        );
        assert!(matches!(res, Err(SceneOpError::UnknownObject(_))));
        let after = p.active_plate().scene.objects.get(&a).unwrap().transform;
        assert_eq!(
            before.to_mat4(),
            after.to_mat4(),
            "the surviving object must be untouched when the op aborts"
        );
    }

    #[test]
    fn align_face_coplanar_slides_tracked_point_onto_the_reference_plane() {
        let mut p = Project::default();
        let (_, obj) = add_cube(&mut p);
        // Identity yaw isolates the spatial slide: the tracked point must end
        // up at X = 10 (the reference plane), with Y/Z untouched.
        let track = Vec3::new(0.5, 0.5, 0.5);
        p.align_face_coplanar(&[obj], Quat::IDENTITY, Vec3::X, 10.0, track)
            .unwrap();
        let xform = p.active_plate().scene.objects.get(&obj).unwrap().transform;
        let moved = xform.apply_point(track);
        assert!(
            (moved.x - 10.0).abs() < 1e-3,
            "tracked X should slide to 10, got {}",
            moved.x
        );
        assert!(
            (moved.y - 0.5).abs() < 1e-3,
            "Y should not move, got {}",
            moved.y
        );
        assert!(
            (moved.z - 0.5).abs() < 1e-3,
            "Z should not move, got {}",
            moved.z
        );
    }

    #[test]
    fn align_face_coplanar_tracks_the_point_through_a_non_identity_yaw() {
        let mut p = Project::default();
        let (_, obj) = add_cube(&mut p);
        // A 90° yaw about Z, then slide along X to X=10. The tracked point (the
        // +X face center) is followed *through* the rotation about the cube's
        // center, which an identity yaw would skip. After a 90° turn that point
        // swings to the +Y side, so its Y proves the yaw was tracked while its X
        // proves the slide landed it on the reference plane.
        let track = Vec3::new(1.0, 0.5, 0.5);
        let yaw = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        p.align_face_coplanar(&[obj], yaw, Vec3::X, 10.0, track)
            .unwrap();
        let xform = p.active_plate().scene.objects.get(&obj).unwrap().transform;
        let moved = xform.apply_point(track);
        assert!(
            (moved.x - 10.0).abs() < 1e-3,
            "tracked X should land on 10, got {}",
            moved.x
        );
        assert!(
            (moved.y - 1.0).abs() < 1e-3,
            "a 90° yaw should swing the tracked point to Y=1, got {}",
            moved.y
        );
    }

    #[test]
    fn add_from_primitive_dedups_same_params_to_one_mesh() {
        let mut p = Project::default();
        let params = PrimitiveParams {
            width: 20.0,
            depth: 20.0,
            height: 20.0,
            radius: 0.0,
            radial_segments: 0,
        };
        let (m1, o1, _) = p.add_from_primitive(PrimitiveKind::Cube, params);
        let (m2, o2, _) = p.add_from_primitive(PrimitiveKind::Cube, params);
        assert_eq!(m1, m2);
        assert_ne!(o1, o2);
        assert_eq!(p.meshes.len(), 1);
    }

    #[test]
    fn add_from_primitive_with_different_params_creates_new_mesh() {
        let mut p = Project::default();
        let p1 = PrimitiveParams {
            width: 20.0,
            depth: 20.0,
            height: 20.0,
            radius: 0.0,
            radial_segments: 0,
        };
        let p2 = PrimitiveParams { width: 30.0, ..p1 };
        let (m1, _, _) = p.add_from_primitive(PrimitiveKind::Cube, p1);
        let (m2, _, _) = p.add_from_primitive(PrimitiveKind::Cube, p2);
        assert_ne!(m1, m2);
        assert_eq!(p.meshes.len(), 2);
    }

    #[test]
    fn add_from_primitive_emits_mesh_loaded_only_first_time() {
        let mut p = Project::default();
        let params = PrimitiveParams::defaults_for(PrimitiveKind::Cube);
        let (_, _, events1) = p.add_from_primitive(PrimitiveKind::Cube, params);
        assert!(events1
            .iter()
            .any(|e| matches!(e, SceneEvent::MeshLoaded { .. })));
        let (_, _, events2) = p.add_from_primitive(PrimitiveKind::Cube, params);
        assert!(!events2
            .iter()
            .any(|e| matches!(e, SceneEvent::MeshLoaded { .. })));
        assert!(events2
            .iter()
            .any(|e| matches!(e, SceneEvent::ObjectAdded { .. })));
    }

    #[test]
    fn load_mesh_lifts_mesh_with_negative_z_onto_bed() {
        // Regression for "complex models slice to 0 layers": STL
        // files often have vertices below Z=0 (model centered on
        // origin geometrically). load_mesh must lift so the
        // lowest mesh point lands on Z=0, otherwise libslic3r
        // clips the geometry as below-bed and the slice produces
        // no layers.
        let mut p = Project::default();
        let mesh = NewMesh {
            vertices: vec![-5.0, -5.0, -3.0, 5.0, -5.0, -3.0, 0.0, 5.0, 3.0],
            indices: vec![0, 1, 2],
            paint_colors: None,
            support_paint: None,
            bounding_box: BoundingBox {
                min: [-5.0, -5.0, -3.0],
                max: [5.0, 5.0, 3.0],
            },
            provenance: MeshProvenance::File("/tmp/test.stl".into()),
        };
        let (_, obj_id, _) = p.load_mesh(mesh);
        let obj = p.plates[0].scene.objects.get(&obj_id).unwrap();
        // The mesh's lowest Z (-3) should now land on Z=0 after
        // the transform applies.
        let lifted_min_z = obj.transform.apply_point(Vec3::new(0.0, 0.0, -3.0)).z;
        assert!(
            lifted_min_z.abs() < 1e-4,
            "lowest mesh vertex must land on Z=0, got {lifted_min_z}",
        );
    }

    #[test]
    fn load_mesh_at_origin_with_z_above_zero_does_not_sink_below() {
        // Mesh that's already sitting at Z=0+: lift should be 0
        // (or near-zero), but XY center applies to position on bed.
        let mut p = Project::default();
        let mesh = NewMesh {
            vertices: vec![0.0, 0.0, 0.0, 10.0, 0.0, 0.0, 0.0, 10.0, 5.0],
            indices: vec![0, 1, 2],
            paint_colors: None,
            support_paint: None,
            bounding_box: BoundingBox {
                min: [0.0, 0.0, 0.0],
                max: [10.0, 10.0, 5.0],
            },
            provenance: MeshProvenance::File("/tmp/test.stl".into()),
        };
        let (_, obj_id, _) = p.load_mesh(mesh);
        let obj = p.plates[0].scene.objects.get(&obj_id).unwrap();
        // Mesh point (0, 0, 0) should still be on the bed (Z=0)
        // after the lift-and-center transform.
        let z = obj.transform.apply_point(Vec3::new(0.0, 0.0, 0.0)).z;
        assert!(z.abs() < 1e-4, "Z=0 vertex stays on bed, got {z}");
    }

    #[test]
    fn transform_op_with_no_bed_emits_no_oob_event() {
        let mut p = Project::default();
        // Project::default now auto-binds the bundled printer + its
        // bed. This test pins the no-bed code path explicitly, so
        // clear it before the move.
        p.plates[0].scene.bed = None;
        let (_, obj) = add_cube(&mut p);
        let events = p
            .set_object_transform(obj, Transform::translation(Vec3::new(500.0, 0.0, 0.0)))
            .unwrap();
        assert!(events
            .iter()
            .all(|e| !matches!(e, SceneEvent::ObjectOutOfBounds { .. })));
    }

    #[test]
    fn translate_off_plate_emits_oob_warning_after_active_printer_set() {
        let mut p = Project::default();
        let (_, obj) = add_cube(&mut p);
        let bed_events = p.set_active_printer(Some(&a1_mini_for_test()));
        assert!(matches!(
            bed_events[0],
            SceneEvent::BedChanged { bed: Some(_), .. },
        ));

        let events = p
            .set_object_transform(obj, Transform::translation(Vec3::new(50.0, 0.0, 0.0)))
            .unwrap();
        assert!(events
            .iter()
            .all(|e| !matches!(e, SceneEvent::ObjectOutOfBounds { .. })));

        // A further +200 in X (250 absolute from origin) shoves the cube off
        // the bed; the op must flag the object out of bounds.
        let events = p
            .set_object_transform(obj, Transform::translation(Vec3::new(250.0, 0.0, 0.0)))
            .unwrap();
        assert!(events.iter().any(|e| matches!(
            e,
            SceneEvent::ObjectOutOfBounds { object_id, reasons, .. }
                if *object_id == obj && !reasons.is_empty()
        )));
    }

    #[test]
    fn rotate_around_explicit_pivot_below_plate_emits_below_plate_reason() {
        use crate::core::scene::bed::OutOfBoundsReason;
        let mut p = Project::default();
        let (_, obj) = add_cube(&mut p);
        p.set_active_printer(Some(&a1_mini_for_test()));
        // Rotate 180° about X around the world origin (the pivot). The unit cube
        // at the origin flips its z∈[0,1] extent down to z∈[-1,0], dropping below
        // the plate. Replicates the old rotate-around-pivot composition:
        // translate(pivot) · rotation · translate(-pivot) prefixed onto the
        // current (identity) transform; with pivot=origin that's just the rotation.
        let pivot = Vec3::new(0.0, 0.0, 0.0);
        let rotation = Quat::from_axis_angle(Vec3::X.normalize(), std::f32::consts::PI);
        let current = p.active_plate().scene.objects.get(&obj).unwrap().transform;
        let rotate_around_pivot = Transform::translation(pivot)
            .compose(Transform::rotation(rotation))
            .compose(Transform::translation(-pivot));
        let events = p
            .set_object_transform(obj, rotate_around_pivot.compose(current))
            .unwrap();
        let oob = events
            .iter()
            .find_map(|e| match e {
                SceneEvent::ObjectOutOfBounds { reasons, .. } => Some(reasons),
                _ => None,
            })
            .expect("expected OOB event");
        assert!(oob
            .iter()
            .any(|r| matches!(r, OutOfBoundsReason::BelowBuildPlate)));
    }

    #[test]
    fn objects_added_to_different_plates_are_independent() {
        let mut p = Project::default();
        let (id_b, _) = p.add_plate(None);
        let (id_c, _) = p.add_plate(None);
        let (_, obj_a) = add_cube(&mut p);
        p.set_active_plate(id_b).unwrap();
        let (_, obj_b) = add_cube(&mut p);
        p.set_active_plate(id_c).unwrap();
        let (_, obj_c) = add_cube(&mut p);

        assert_eq!(p.plates[0].scene.objects.len(), 1);
        assert_eq!(p.plates[1].scene.objects.len(), 1);
        assert_eq!(p.plates[2].scene.objects.len(), 1);
        assert!(p.plates[0].scene.objects.contains_key(&obj_a));
        assert!(p.plates[1].scene.objects.contains_key(&obj_b));
        assert!(p.plates[2].scene.objects.contains_key(&obj_c));
        // load_mesh allocates per call, so 3 meshes scene-wide.
        assert_eq!(p.meshes.len(), 3);
        assert_ne!(obj_a, obj_b);
        assert_ne!(obj_b, obj_c);
    }

    #[test]
    fn selection_is_per_plate() {
        let mut p = Project::default();
        let (id_b, _) = p.add_plate(None);
        let (_, obj_a) = add_cube(&mut p);
        p.set_active_plate(id_b).unwrap();
        let (_, obj_b) = add_cube(&mut p);

        p.set_active_plate(PlateId(1)).unwrap();
        p.select(&[obj_a], SelectMode::Replace);
        p.set_active_plate(id_b).unwrap();
        p.select(&[obj_b], SelectMode::Replace);

        assert!(p.plates[0].scene.selection.contains(&obj_a));
        assert!(!p.plates[0].scene.selection.contains(&obj_b));
        assert!(p.plates[1].scene.selection.contains(&obj_b));
        assert!(!p.plates[1].scene.selection.contains(&obj_a));
    }

    #[test]
    fn primitive_cache_dedups_across_plates() {
        let mut p = Project::default();
        let (id_b, _) = p.add_plate(None);
        let params = PrimitiveParams::defaults_for(PrimitiveKind::Cube);
        let (mesh_a, _, _) = p.add_from_primitive(PrimitiveKind::Cube, params);
        p.set_active_plate(id_b).unwrap();
        let (mesh_b, _, _) = p.add_from_primitive(PrimitiveKind::Cube, params);
        assert_eq!(mesh_a, mesh_b);
        assert_eq!(p.meshes.len(), 1);
    }

    #[test]
    fn clone_objects_lands_in_authored_order_regardless_of_input() {
        // `ids` is a selection *set* (its order is meaningless — it can arrive
        // from a HashSet, e.g. the click-to-select-group path). The clones must
        // follow the originals' authored order. Pass ids scrambled and confirm.
        let mut p = Project::default();
        let (ma, a) = add_cube(&mut p);
        let (mb, b) = add_cube(&mut p);
        let (mc, c) = add_cube(&mut p);

        // Order the scrambled selection through the plate; clone preserves it.
        let ids = p.active_plate().scene.objects.in_order(&[c, a, b]);
        let (new_ids, _) = p.clone_objects(&ids, 1);
        let meshes: Vec<MeshId> = new_ids
            .iter()
            .map(|id| p.active_plate().scene.objects.get(id).unwrap().mesh)
            .collect();
        assert_eq!(
            meshes,
            vec![ma, mb, mc],
            "clones follow authored order, not the input id order",
        );
    }

    #[test]
    fn clone_objects_copies_geometry_overrides_and_group_structure() {
        let mut p = Project::default();
        let (mesh_a, a) = add_cube(&mut p);
        let (mesh_b, b) = add_cube(&mut p);
        let active_id = p.active_plate().id;
        // Group the two cubes and give one of them a per-object override.
        p.group_objects(&[a, b], "duo".into()).unwrap();
        p.object_override_set(active_id, a, "layer_height".into(), "0.12".into())
            .unwrap();
        let src_group = p.active_plate().scene.objects[&a].group.unwrap();

        // Two copies of the whole group.
        let ids = p.active_plate().scene.objects.in_order(&[a, b]);
        let (new_ids, events) = p.clone_objects(&ids, 2);
        assert_eq!(new_ids.len(), 4, "2 copies × 2 members");
        assert_eq!(
            p.active_plate().scene.objects.len(),
            6,
            "2 originals + 4 clones"
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, SceneEvent::ObjectAdded { .. }))
                .count(),
            4,
        );

        // Geometry is shared — clones reuse their source's mesh, none minted.
        assert_eq!(p.meshes.len(), 2, "no new meshes for clones");
        // new_ids is copy-major: [copy0.a, copy0.b, copy1.a, copy1.b].
        let (c0a, c0b, c1a, c1b) = (new_ids[0], new_ids[1], new_ids[2], new_ids[3]);
        assert_eq!(p.active_plate().scene.objects[&c0a].mesh, mesh_a);
        assert_eq!(p.active_plate().scene.objects[&c1a].mesh, mesh_a);
        assert_eq!(p.active_plate().scene.objects[&c0b].mesh, mesh_b);
        assert_eq!(p.active_plate().scene.objects[&c1b].mesh, mesh_b);
        // Each copy is its own group (fresh id, distinct from the source and the
        // other copy); its two members share that one id; "duo" name carried.
        let g0 = p.active_plate().scene.objects[&c0a].group.unwrap();
        let g1 = p.active_plate().scene.objects[&c1a].group.unwrap();
        assert_eq!(p.active_plate().scene.objects[&c0b].group, Some(g0));
        assert_eq!(p.active_plate().scene.objects[&c1b].group, Some(g1));
        assert_ne!(g0, src_group);
        assert_ne!(g1, src_group);
        assert_ne!(g0, g1);
        assert_eq!(p.active_plate().scene.groups[&g0].name, "duo");
        assert_eq!(p.active_plate().scene.groups[&g1].name, "duo");

        // The override on `a` rode along to each copy's clone-of-a; the
        // clone-of-b carries none.
        let ov = &p.active_plate().scene.object_overrides;
        assert_eq!(ov[&c0a]["layer_height"], "0.12");
        assert_eq!(ov[&c1a]["layer_height"], "0.12");
        assert!(!ov.contains_key(&c0b));
        assert!(!ov.contains_key(&c1b));
    }

    #[test]
    fn clone_objects_carries_cut_connectors() {
        let mut p = Project::default();
        let (mesh_a, a) = add_cube(&mut p);
        attach_connectors(&mut p, a);
        let mesh_count = p.meshes.len();

        let ids = p.active_plate().scene.objects.in_order(&[a]);
        let (new_ids, _) = p.clone_objects(&ids, 1);
        let c = new_ids[0];
        let scene = &p.active_plate().scene;
        assert_eq!(scene.object_modifiers[&c].len(), 1, "peg carried to the clone");
        assert_eq!(scene.object_hole_markers[&c].len(), 1, "hole marker carried to the clone");
        // Connector meshes are reused, not re-registered (like the main mesh).
        assert_eq!(scene.objects[&c].mesh, mesh_a);
        assert_eq!(p.meshes.len(), mesh_count, "no new meshes minted for the clone");
    }

    #[test]
    fn move_objects_to_plate_relocates_a_set_preserving_transforms() {
        let mut p = Project::default();
        let (id_b, _) = p.add_plate(None);
        let (_, a) = add_cube(&mut p);
        let (_, b) = add_cube(&mut p);
        p.set_object_transform(a, Transform::translation(Vec3::new(20.0, 0.0, 0.0)))
            .unwrap();
        p.set_object_transform(b, Transform::translation(Vec3::new(60.0, 0.0, 0.0)))
            .unwrap();
        let (ta, tb) = (
            p.plates[0].scene.objects.get(&a).unwrap().transform,
            p.plates[0].scene.objects.get(&b).unwrap().transform,
        );
        let events = p.move_objects_to_plate(PlateId(1), id_b, &[a, b]).unwrap();
        assert!(!p.plates[0].scene.objects.contains_key(&a));
        assert!(!p.plates[0].scene.objects.contains_key(&b));
        // Transforms are preserved exactly (a pure relocation, no recentering).
        assert_eq!(p.plates[1].scene.objects.get(&a).unwrap().transform.to_mat4(), ta.to_mat4());
        assert_eq!(p.plates[1].scene.objects.get(&b).unwrap().transform.to_mat4(), tb.to_mat4());
        assert_eq!(
            events.iter().filter(|e| matches!(e, SceneEvent::ObjectRemoved { .. })).count(),
            2
        );
        assert_eq!(
            events.iter().filter(|e| matches!(e, SceneEvent::ObjectAdded { .. })).count(),
            2
        );
    }

    #[test]
    fn move_objects_to_plate_keeps_a_group_rigid_and_carries_its_name() {
        let mut p = Project::default();
        let (id_b, _) = p.add_plate(None);
        let (_, a) = add_cube(&mut p);
        let (_, b) = add_cube(&mut p);
        p.set_object_transform(b, Transform::translation(Vec3::new(40.0, 5.0, 0.0)))
            .unwrap();
        p.group_objects(&[a, b], "duo".into()).unwrap();
        let g = p.plates[0].scene.objects.get(&a).unwrap().group.unwrap();
        let origin = |p: &Project, plate: usize, id| {
            p.plates[plate].scene.objects.get(&id).unwrap().transform.apply_point(Vec3::ZERO)
        };
        let rel_before = origin(&p, 0, b) - origin(&p, 0, a);

        // Moving one member moves the whole group (ids expand to the group).
        p.move_objects_to_plate(PlateId(1), id_b, &[a]).unwrap();
        assert!(p.plates[1].scene.objects.contains_key(&a));
        assert!(p.plates[1].scene.objects.contains_key(&b));
        let rel_after = origin(&p, 1, b) - origin(&p, 1, a);
        assert!((rel_after - rel_before).length() < 1e-6, "group not rigid through the move");
        // The group's name travels with it.
        assert!(!p.plates[0].scene.groups.contains_key(&g));
        assert_eq!(p.plates[1].scene.groups.get(&g).unwrap().name, "duo");
    }

    #[test]
    fn move_objects_to_plate_preserves_authored_order_on_target() {
        // Same class as the clone bug: moving a group routed its members through
        // expand_to_groups' HashSet, so they used to land on the target plate in
        // random relative order. They must arrive in authored order.
        let mut p = Project::default();
        let (id_b, _) = p.add_plate(None);
        let (_, a) = add_cube(&mut p);
        let (_, b) = add_cube(&mut p);
        let (_, c) = add_cube(&mut p);
        p.group_objects(&[a, b, c], "trio".into()).unwrap();

        p.move_objects_to_plate(PlateId(1), id_b, &[a]).unwrap();
        let order: Vec<ObjectId> = p.plates[1].scene.objects.values().map(|o| o.id).collect();
        assert_eq!(order, vec![a, b, c], "moved group keeps authored order on target");
    }

    #[test]
    fn move_objects_to_plate_carries_material_slot_bindings() {
        use crate::core::printer::SlotRef;
        let mut p = Project::default();
        let (id_b, _) = p.add_plate(None);
        let (_, obj) = add_cube(&mut p);
        // The object uses material 3, pinned to a specific slot on the source.
        p.plates[0].scene.objects.get_mut(&obj).unwrap().extruder_id = Some(3);
        let slot = SlotRef { extruder: 1, slot: 2 };
        p.plates[0].material_to_slot.insert(3, slot);

        let events = p.move_objects_to_plate(PlateId(1), id_b, &[obj]).unwrap();
        // The binding travels with the object — exact, since same printer.
        assert_eq!(p.plates[1].material_to_slot.get(&3), Some(&slot));
        assert!(events.iter().any(|e| matches!(
            e,
            SceneEvent::MaterialSlotChanged { plate_id } if *plate_id == id_b
        )));
    }

    #[test]
    fn move_objects_to_plate_rejects_same_and_unknown_plate() {
        let mut p = Project::default();
        let (_, a) = add_cube(&mut p);
        assert!(matches!(
            p.move_objects_to_plate(PlateId(1), PlateId(1), &[a]),
            Err(SceneOpError::SamePlate(_))
        ));
        assert!(matches!(
            p.move_objects_to_plate(PlateId(1), PlateId(99), &[a]),
            Err(SceneOpError::UnknownPlate(_))
        ));
    }

    #[test]
    fn move_objects_to_plate_carries_cut_connectors() {
        let mut p = Project::default();
        let (id_b, _) = p.add_plate(None);
        let (_, a) = add_cube(&mut p);
        attach_connectors(&mut p, a);

        p.move_objects_to_plate(PlateId(1), id_b, &[a]).unwrap();
        // Sidecars moved with the object (same ObjectId) and the source is clean.
        assert!(!p.plates[0].scene.object_modifiers.contains_key(&a));
        assert!(!p.plates[0].scene.object_hole_markers.contains_key(&a));
        assert_eq!(p.plates[1].scene.object_modifiers[&a].len(), 1, "peg followed the move");
        assert_eq!(
            p.plates[1].scene.object_hole_markers[&a].len(),
            1,
            "hole marker followed the move",
        );
    }

    /// Attach one peg modifier + one hole marker to `id` on the active plate,
    /// mirroring `apply_cut`'s sidecar shape (see the recut carry test).
    fn attach_connectors(p: &mut Project, id: ObjectId) {
        let active = p.active_plate;
        let peg = p.register_mesh(NewMesh {
            vertices: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            indices: vec![0, 1, 2],
            paint_colors: None,
            support_paint: None,
            bounding_box: BoundingBox { min: [0.0; 3], max: [1.0, 1.0, 0.0] },
            provenance: MeshProvenance::Primitive("peg".into()),
        });
        p.plates[active]
            .scene
            .object_modifiers
            .insert(id, vec![Modifier { mesh: peg, kind: ModifierKind::Peg }]);
        p.plates[active].scene.object_hole_markers.insert(
            id,
            vec![crate::core::scene::state::HoleMarker {
                shape: crate::core::scene::state::HoleMarkerShape::Circle,
                radius: 1.0,
                center: [0.0, 0.0, 0.0],
                normal: [0.0, 0.0, 1.0],
                u_axis: [1.0, 0.0, 0.0],
            }],
        );
    }
}
