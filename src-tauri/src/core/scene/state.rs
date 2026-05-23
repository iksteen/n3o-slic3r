//! Scene state — the authoritative model the renderer reflects.
//!
//! Holds the registry of loaded meshes, the placed objects, the
//! current selection, the camera/gizmo state, and the active plate +
//! its exclusion zones. Every mutation goes through the Tauri command
//! surface (PR-2-2); the renderer is a read-only consumer that
//! mirrors this via emitted events.
//!
//! Per the AD-8 invariant: this is the *only* place authoritative
//! scene state lives. The renderer's local mirror is a cache;
//! `scene_snapshot()` rebuilds it from scratch on reconnect.

use super::build_plate::BuildPlate;
use super::events::{MirrorAxis, SceneEvent, SceneOpError, SelectMode};
use super::transform::Transform;
use crate::core::printer::profile::BoundingBox;
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Opaque mesh identifier. Monotonic across the registry's lifetime;
/// never reused even after a mesh is freed. Surfaced to the frontend
/// as a string-encoded number via the standard serde representation.
#[derive(
    Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct MeshId(pub u64);

/// Opaque scene-object identifier. Same monotonic-never-reused
/// semantics as `MeshId`.
#[derive(
    Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct ObjectId(pub u64);

/// Loaded mesh data + provenance.
///
/// The vertex / normal / index buffers live on the Rust side; they
/// reach the renderer via the dedicated `scene_mesh_buffers` Tauri
/// command (binary IPC), NOT the JSON event/snapshot path. The
/// `#[serde(skip)]` annotations enforce that — sending a 47 MB mesh
/// as JSON arrays would be ~100 MB of stringified floats and
/// gigabytes of JS-side parse work.
///
/// Wire-side consumers see [`MeshHeader`] (lightweight metadata) and
/// fetch the buffers separately.
///
/// For multi-volume objects (3MF), each volume is its own `Mesh`
/// and the source 3MF spawns one `SceneObject` per volume.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mesh {
    pub id: MeshId,
    /// Flat `[x0, y0, z0, x1, y1, z1, ...]` packed vertices. Not
    /// serialized — fetched via `scene_mesh_buffers`.
    #[serde(skip, default)]
    pub vertices: Vec<f32>,
    /// Flat `[x, y, z, ...]` per-vertex normals; same length as
    /// `vertices`. Not serialized.
    #[serde(skip, default)]
    pub normals: Vec<f32>,
    /// Triangle vertex indices (3 per triangle). Not serialized.
    #[serde(skip, default)]
    pub indices: Vec<u32>,
    pub bounding_box: BoundingBox,
    /// File path or in-app catalog handle the mesh came from.
    pub provenance: MeshProvenance,
}

impl Mesh {
    /// Lightweight metadata for the JSON wire (events + snapshots).
    pub fn header(&self) -> MeshHeader {
        MeshHeader {
            id: self.id,
            vertex_count: self.vertices.len() / 3,
            index_count: self.indices.len(),
            bounding_box: self.bounding_box,
            provenance: self.provenance.clone(),
        }
    }

    /// Encode vertex / normal / index buffers into one concatenated
    /// little-endian byte sequence: `[vertices_f32...][normals_f32...][indices_u32...]`.
    /// The frontend slices it by lengths from the matching `MeshHeader`.
    pub fn pack_buffers(&self) -> Vec<u8> {
        let vert_bytes = self.vertices.len() * std::mem::size_of::<f32>();
        let norm_bytes = self.normals.len() * std::mem::size_of::<f32>();
        let idx_bytes = self.indices.len() * std::mem::size_of::<u32>();
        let mut out = Vec::with_capacity(vert_bytes + norm_bytes + idx_bytes);
        for f in &self.vertices {
            out.extend_from_slice(&f.to_le_bytes());
        }
        for f in &self.normals {
            out.extend_from_slice(&f.to_le_bytes());
        }
        for i in &self.indices {
            out.extend_from_slice(&i.to_le_bytes());
        }
        out
    }
}

/// JSON-wire shape of a `Mesh` — everything except the heavy buffers.
/// `scene:mesh_loaded` events and `scene_snapshot` carry this; the
/// frontend follows up with `scene_mesh_buffers(id)` to fetch the
/// binary blob.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshHeader {
    pub id: MeshId,
    pub vertex_count: usize,
    pub index_count: usize,
    pub bounding_box: BoundingBox,
    pub provenance: MeshProvenance,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value")]
pub enum MeshProvenance {
    /// Loaded from a file on disk.
    File(String),
    /// Procedural primitive (PR-2-7's object library).
    Primitive(String),
}

/// A scene object — one placement of a Mesh, with a transform and
/// per-object metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneObject {
    pub id: ObjectId,
    pub mesh: MeshId,
    pub transform: Transform,
    pub name: String,
    #[serde(default = "default_visible")]
    pub visible: bool,
    /// Per-object filament/extruder assignment. `None` means
    /// "inherits from project default"; multi-color 3MFs (per
    /// PR-2-4) populate this from the source file's
    /// `model_settings.config` extruder metadata.
    #[serde(default)]
    pub extruder_id: Option<u8>,
    /// Parent object — `None` for top-level objects. Hierarchical
    /// grouping (Phase 5 multi-plate) builds on this.
    #[serde(default)]
    pub parent: Option<ObjectId>,
}

/// Caller-builds-this shape for inserting a fresh mesh. No `id`
/// field — the `SceneState` allocates it. Use for loaders + the
/// procedural-primitive path.
#[derive(Debug, Clone)]
pub struct NewMesh {
    pub vertices: Vec<f32>,
    pub normals: Vec<f32>,
    pub indices: Vec<u32>,
    pub bounding_box: BoundingBox,
    pub provenance: MeshProvenance,
}

/// Caller-builds-this shape for inserting a fresh scene object. No
/// `id` field — `SceneState` allocates it.
#[derive(Debug, Clone)]
pub struct NewSceneObject {
    pub mesh: MeshId,
    pub transform: Transform,
    pub name: String,
    pub visible: bool,
    pub extruder_id: Option<u8>,
    pub parent: Option<ObjectId>,
}

impl NewSceneObject {
    /// Default scene-object: visible, no parent, no extruder, named
    /// after its mesh.
    pub fn at_origin(mesh: MeshId, name: impl Into<String>) -> Self {
        Self {
            mesh,
            transform: Transform::IDENTITY,
            name: name.into(),
            visible: true,
            extruder_id: None,
            parent: None,
        }
    }
}

fn default_visible() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CameraState {
    /// World-space camera position.
    pub position: [f32; 3],
    /// Look-at target.
    pub target: [f32; 3],
    /// World-space up vector. Z-up by convention (matches libslic3r
    /// + the build plate's Z=0 origin).
    pub up: [f32; 3],
    pub projection: ProjectionMode,
    /// Vertical field of view in radians, used by perspective.
    pub fov_y_radians: f32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProjectionMode {
    Perspective,
    /// Cut candidate per the Execution Plan; keep the enum variant
    /// so the renderer doesn't have to special-case its absence.
    Orthographic,
}

impl Default for CameraState {
    fn default() -> Self {
        Self {
            position: [200.0, -200.0, 200.0],
            target: [0.0, 0.0, 0.0],
            up: [0.0, 0.0, 1.0],
            projection: ProjectionMode::Perspective,
            fov_y_radians: std::f32::consts::FRAC_PI_4,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GizmoState {
    pub mode: GizmoMode,
    /// World-space pivot point for rotation/scale ops. `None` =
    /// pivot at the selected object's center (the resolver computes
    /// it at apply time).
    pub pivot: Option<[f32; 3]>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum GizmoMode {
    None,
    Translate,
    Rotate,
    Scale,
}

impl Default for GizmoState {
    fn default() -> Self {
        Self {
            mode: GizmoMode::None,
            pivot: None,
        }
    }
}

/// The active build plate's identity + transform. MVP ships one;
/// Phase 5 extends this to a `Vec<ActivePlate>`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivePlate {
    pub build_plate: BuildPlate,
    /// Where the plate sits in world space. Defaults to identity
    /// (plate origin = world origin). Multi-plate scenes (Phase 5)
    /// translate plates apart.
    #[serde(default)]
    pub transform: Transform,
}

/// A printer's exclusion zone, as world-space coordinates after the
/// active plate's transform. Cached in the scene state so the
/// renderer + collision check don't recompute on every transform op.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExclusionZone {
    pub label: String,
    pub bounds: BoundingBox,
}

/// The root authoritative scene model. Tauri commands lock this
/// behind a `Mutex<SceneState>` (or `RwLock` if PR-2-11's
/// scene-state perf gate shows contention) and mutate via the
/// methods below. The renderer never reaches into this directly —
/// it sees only the emitted events.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SceneState {
    pub meshes: HashMap<MeshId, Mesh>,
    pub objects: HashMap<ObjectId, SceneObject>,
    pub selection: HashSet<ObjectId>,
    pub camera: CameraState,
    pub gizmo: GizmoState,
    pub plate: Option<ActivePlate>,
    pub exclusion_zones: Vec<ExclusionZone>,
    /// Active bed visualization + bounds (PR-2-6). `None` when no
    /// printer is selected yet — the scene is still usable for
    /// loading meshes, just without the out-of-bounds check or grid.
    #[serde(default)]
    pub bed: Option<super::bed::BedMesh>,
    /// Primitive mesh cache (PR-2-7). Each (kind, params) tuple
    /// resolves to one MeshId so re-instancing the same procedural
    /// primitive yields multiple SceneObjects sharing geometry.
    /// Linear scan is fine — the cache stays small in practice
    /// (a handful of distinct shapes per session).
    #[serde(default, skip)]
    primitive_cache: Vec<(super::primitives::PrimitiveKind, super::primitives::PrimitiveParams, MeshId)>,
    next_mesh_id: u64,
    next_object_id: u64,
}

impl SceneState {
    pub fn new() -> Self {
        Self::default()
    }

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

    /// Register a mesh. Always allocates a fresh `MeshId`; the
    /// caller hands in a `NewMesh` (no id field) so there's no
    /// possibility of an ID collision or sentinel ambiguity.
    pub fn register_mesh(&mut self, new_mesh: NewMesh) -> MeshId {
        let id = self.next_mesh_id();
        self.meshes.insert(
            id,
            Mesh {
                id,
                vertices: new_mesh.vertices,
                normals: new_mesh.normals,
                indices: new_mesh.indices,
                bounding_box: new_mesh.bounding_box,
                provenance: new_mesh.provenance,
            },
        );
        id
    }

    /// Register a scene object. Always allocates a fresh `ObjectId`.
    pub fn register_object(&mut self, new_obj: NewSceneObject) -> ObjectId {
        let id = self.next_object_id();
        self.objects.insert(
            id,
            SceneObject {
                id,
                mesh: new_obj.mesh,
                transform: new_obj.transform,
                name: new_obj.name,
                visible: new_obj.visible,
                extruder_id: new_obj.extruder_id,
                parent: new_obj.parent,
            },
        );
        id
    }

    // ---- Mutation methods ------------------------------------------
    //
    // Each method takes &mut self and returns the events the renderer
    // needs to apply. The Tauri command wrappers in commands.rs are
    // thin layers that emit each event via Window::emit before
    // returning. Tests bypass the Tauri layer and inspect the returned
    // event list directly.

    /// Register a mesh and place one default `SceneObject` at origin.
    /// Returns (mesh_id, object_id, events).
    pub fn load_mesh(&mut self, new_mesh: NewMesh) -> (MeshId, ObjectId, Vec<SceneEvent>) {
        let obj_name = match &new_mesh.provenance {
            MeshProvenance::File(p) => p
                .rsplit_once('/')
                .map(|(_, leaf)| leaf.to_string())
                .unwrap_or_else(|| p.clone()),
            MeshProvenance::Primitive(name) => name.clone(),
        };
        let mesh_id = self.register_mesh(new_mesh);
        let obj_id = self.register_object(NewSceneObject::at_origin(mesh_id, obj_name));

        let mesh_header = self.meshes.get(&mesh_id).unwrap().header();
        let obj_clone = self.objects.get(&obj_id).unwrap().clone();
        let events = vec![
            SceneEvent::MeshLoaded(mesh_header),
            SceneEvent::ObjectAdded(obj_clone),
        ];
        (mesh_id, obj_id, events)
    }

    /// Apply a selection change. Returns one `SelectionChanged` event
    /// (sorted for deterministic output) or empty if the selection
    /// didn't actually change.
    pub fn select(&mut self, ids: &[ObjectId], mode: SelectMode) -> Vec<SceneEvent> {
        let before: HashSet<ObjectId> = self.selection.iter().copied().collect();
        match mode {
            SelectMode::Replace => {
                self.selection = ids.iter().copied().filter(|id| self.objects.contains_key(id)).collect();
            }
            SelectMode::Add => {
                for id in ids {
                    if self.objects.contains_key(id) {
                        self.selection.insert(*id);
                    }
                }
            }
            SelectMode::Toggle => {
                for id in ids {
                    if !self.objects.contains_key(id) {
                        continue;
                    }
                    if !self.selection.insert(*id) {
                        self.selection.remove(id);
                    }
                }
            }
        }
        if self.selection == before {
            return Vec::new();
        }
        let mut sorted: Vec<ObjectId> = self.selection.iter().copied().collect();
        sorted.sort();
        vec![SceneEvent::SelectionChanged { selected: sorted }]
    }

    /// Clear the selection.
    pub fn deselect_all(&mut self) -> Vec<SceneEvent> {
        if self.selection.is_empty() {
            return Vec::new();
        }
        self.selection.clear();
        vec![SceneEvent::SelectionChanged { selected: Vec::new() }]
    }

    /// Apply a delta translation to an object.
    pub fn translate_object(
        &mut self,
        id: ObjectId,
        delta: Vec3,
    ) -> Result<Vec<SceneEvent>, SceneOpError> {
        let obj = self
            .objects
            .get_mut(&id)
            .ok_or(SceneOpError::UnknownObject(id))?;
        obj.transform = Transform::translation(delta).compose(obj.transform);
        let clone = obj.clone();
        let mut events = vec![SceneEvent::ObjectUpdated(clone)];
        events.extend(self.out_of_bounds_event(id));
        Ok(events)
    }

    /// Rotate an object around `axis` by `radians`. Pivot defaults to
    /// the object's current world-space center; explicit pivot via
    /// `pivot_override` is for the gizmo's "rotate around custom
    /// point" mode (PR-2-10).
    pub fn rotate_object(
        &mut self,
        id: ObjectId,
        axis: Vec3,
        radians: f32,
        pivot_override: Option<Vec3>,
    ) -> Result<Vec<SceneEvent>, SceneOpError> {
        let mesh_bb = {
            let obj = self
                .objects
                .get(&id)
                .ok_or(SceneOpError::UnknownObject(id))?;
            self.meshes
                .get(&obj.mesh)
                .ok_or(SceneOpError::UnknownMesh(obj.mesh))?
                .bounding_box
                .clone()
        };

        let obj = self.objects.get_mut(&id).unwrap();
        let pivot = match pivot_override {
            Some(p) => p,
            None => {
                let local_center = Vec3::new(
                    ((mesh_bb.min[0] + mesh_bb.max[0]) * 0.5) as f32,
                    ((mesh_bb.min[1] + mesh_bb.max[1]) * 0.5) as f32,
                    ((mesh_bb.min[2] + mesh_bb.max[2]) * 0.5) as f32,
                );
                obj.transform.apply_point(local_center)
            }
        };
        let rotation = Quat::from_axis_angle(axis.normalize(), radians);
        // Rotate-around-pivot: translate(-pivot) → rotate → translate(+pivot),
        // applied as a world-space *prefix* to the current transform.
        let rotate_around_pivot = Transform::translation(pivot)
            .compose(Transform::rotation(rotation))
            .compose(Transform::translation(-pivot));
        obj.transform = rotate_around_pivot.compose(obj.transform);
        let clone = obj.clone();
        let mut events = vec![SceneEvent::ObjectUpdated(clone)];
        events.extend(self.out_of_bounds_event(id));
        Ok(events)
    }

    /// Scale an object by per-axis factors. Non-uniform scale emits
    /// an extra `NonUniformScale` warning event so the UI can flag
    /// the object; it's not blocking. Dimensional cascade settings
    /// (line widths, top-surface thresholds) reason about physical
    /// extents, and stretching one axis silently breaks those.
    pub fn scale_object(
        &mut self,
        id: ObjectId,
        factor: Vec3,
    ) -> Result<Vec<SceneEvent>, SceneOpError> {
        let obj = self
            .objects
            .get_mut(&id)
            .ok_or(SceneOpError::UnknownObject(id))?;
        obj.transform = Transform::scale(factor).compose(obj.transform);
        let clone = obj.clone();
        let mut events = vec![SceneEvent::ObjectUpdated(clone)];
        if is_non_uniform(factor) {
            events.push(SceneEvent::NonUniformScale { id });
        }
        events.extend(self.out_of_bounds_event(id));
        Ok(events)
    }

    /// Mirror an object across a world-axis through the object's
    /// world-space center. Two mirrors across the same axis return
    /// the object to its original transform (modulo float error).
    pub fn mirror_object(
        &mut self,
        id: ObjectId,
        axis: MirrorAxis,
    ) -> Result<Vec<SceneEvent>, SceneOpError> {
        let center = self.world_center(id)?;
        let factor = match axis {
            MirrorAxis::X => Vec3::new(-1.0, 1.0, 1.0),
            MirrorAxis::Y => Vec3::new(1.0, -1.0, 1.0),
            MirrorAxis::Z => Vec3::new(1.0, 1.0, -1.0),
        };
        // Mirror-around-center: translate(-c) → scale(±1) → translate(+c),
        // applied as a world-space *prefix* to the current transform.
        let mirror_around_center = Transform::translation(center)
            .compose(Transform::scale(factor))
            .compose(Transform::translation(-center));
        let obj = self.objects.get_mut(&id).unwrap();
        obj.transform = mirror_around_center.compose(obj.transform);
        let clone = obj.clone();
        let mut events = vec![SceneEvent::ObjectUpdated(clone)];
        events.extend(self.out_of_bounds_event(id));
        Ok(events)
    }

    /// Lay-flat heuristic: re-orient the object to minimize its
    /// world-space Z extent, then drop it so the new minimum Z is
    /// exactly the active plate's surface (Z=0 for an identity-
    /// transform plate). Searches the 24 axis-aligned cube
    /// rotations and picks the one that produces the smallest Z
    /// extent — fast, deterministic, no mesh-face analysis. MVP
    /// per the ticket; PR-2-7's library + Phase 4 UI can introduce
    /// "lay flat on selected face" later when the user can pick a
    /// face from the viewport.
    pub fn lay_flat_object(
        &mut self,
        id: ObjectId,
    ) -> Result<Vec<SceneEvent>, SceneOpError> {
        let mesh_bb = {
            let obj = self
                .objects
                .get(&id)
                .ok_or(SceneOpError::UnknownObject(id))?;
            self.meshes
                .get(&obj.mesh)
                .ok_or(SceneOpError::UnknownMesh(obj.mesh))?
                .bounding_box
                .clone()
        };
        let local_corners = mesh_bb_corners(&mesh_bb);
        let local_center = Vec3::new(
            ((mesh_bb.min[0] + mesh_bb.max[0]) * 0.5) as f32,
            ((mesh_bb.min[1] + mesh_bb.max[1]) * 0.5) as f32,
            ((mesh_bb.min[2] + mesh_bb.max[2]) * 0.5) as f32,
        );

        let obj = self.objects.get_mut(&id).unwrap();
        let current = obj.transform.to_mat4();

        // Decompose current transform into scale, rotation,
        // translation so we can preserve scale + center position
        // while replacing the rotation with a candidate one. glam's
        // decomposition is sound for affine matrices without shear
        // — our transforms are built from translate/rotate/scale
        // composition only, so this holds.
        let (current_scale, _current_rot, current_trans) =
            current.to_scale_rotation_translation();
        // World-space center under the current transform — we keep
        // this fixed so the object doesn't drift sideways when
        // re-oriented; only Z adjusts (to drop to the bed).
        let current_world_center = obj.transform.apply_point(local_center);

        let (best_rotation, best_min_z) = cube_rotations()
            .into_iter()
            .map(|rot| {
                let candidate = glam::Mat4::from_scale_rotation_translation(
                    current_scale,
                    rot,
                    current_trans,
                );
                let (min_z, max_z) = z_extent(&local_corners, &candidate);
                (rot, min_z, max_z)
            })
            .min_by(|a, b| {
                let extent_a = a.2 - a.1;
                let extent_b = b.2 - b.1;
                extent_a
                    .partial_cmp(&extent_b)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(rot, min_z, _)| (rot, min_z))
            .expect("24 rotations is non-empty");

        // Rebuild the transform with the chosen rotation, keeping the
        // X/Y of the world-space center fixed; the new Z translation
        // drops the object so its minimum Z lands on the plate.
        let chosen = glam::Mat4::from_scale_rotation_translation(
            current_scale,
            best_rotation,
            current_trans,
        );
        // After the candidate is applied to local corners we know the
        // world-space min Z. Translate by -(min_z) so it lands at 0.
        // Also re-pin XY center.
        let post_rot_center = chosen.transform_point3(local_center);
        let delta = glam::Vec3::new(
            current_world_center.x - post_rot_center.x,
            current_world_center.y - post_rot_center.y,
            -best_min_z,
        );
        let final_xform = glam::Mat4::from_translation(delta) * chosen;
        obj.transform = Transform::from_mat4(final_xform);

        let clone = obj.clone();
        let mut events = vec![SceneEvent::ObjectUpdated(clone)];
        events.extend(self.out_of_bounds_event(id));
        Ok(events)
    }

    /// World-space center of an object's mesh bounding box. Pulled
    /// out so mirror + future bbox-anchored ops share one path.
    fn world_center(&self, id: ObjectId) -> Result<Vec3, SceneOpError> {
        let obj = self
            .objects
            .get(&id)
            .ok_or(SceneOpError::UnknownObject(id))?;
        let mesh_bb = self
            .meshes
            .get(&obj.mesh)
            .ok_or(SceneOpError::UnknownMesh(obj.mesh))?
            .bounding_box
            .clone();
        let local_center = Vec3::new(
            ((mesh_bb.min[0] + mesh_bb.max[0]) * 0.5) as f32,
            ((mesh_bb.min[1] + mesh_bb.max[1]) * 0.5) as f32,
            ((mesh_bb.min[2] + mesh_bb.max[2]) * 0.5) as f32,
        );
        Ok(obj.transform.apply_point(local_center))
    }

    /// Replace an object's transform wholesale. Used by auto-arrange
    /// (PR-2-8) and the gizmo's drag-finalization step.
    pub fn set_object_transform(
        &mut self,
        id: ObjectId,
        transform: Transform,
    ) -> Result<Vec<SceneEvent>, SceneOpError> {
        let obj = self
            .objects
            .get_mut(&id)
            .ok_or(SceneOpError::UnknownObject(id))?;
        obj.transform = transform;
        let clone = obj.clone();
        let mut events = vec![SceneEvent::ObjectUpdated(clone)];
        events.extend(self.out_of_bounds_event(id));
        Ok(events)
    }

    /// Delete one or more objects. Removes from selection if
    /// present. Returns one `ObjectRemoved` event per id plus (if
    /// the selection changed) a `SelectionChanged` event.
    pub fn delete_objects(&mut self, ids: &[ObjectId]) -> Vec<SceneEvent> {
        let mut events = Vec::new();
        let mut selection_changed = false;
        for id in ids {
            if self.objects.remove(id).is_some() {
                events.push(SceneEvent::ObjectRemoved { id: *id });
                if self.selection.remove(id) {
                    selection_changed = true;
                }
            }
        }
        if selection_changed {
            let mut sorted: Vec<ObjectId> = self.selection.iter().copied().collect();
            sorted.sort();
            events.push(SceneEvent::SelectionChanged { selected: sorted });
        }
        events
    }

    /// Duplicate an object. The clone gets a fresh `ObjectId` and
    /// is offset by `+10mm` in X to avoid z-fighting with the
    /// original.
    pub fn duplicate_object(
        &mut self,
        id: ObjectId,
    ) -> Result<(ObjectId, Vec<SceneEvent>), SceneOpError> {
        let original = self
            .objects
            .get(&id)
            .ok_or(SceneOpError::UnknownObject(id))?
            .clone();
        let new_id = self.register_object(NewSceneObject {
            mesh: original.mesh,
            transform: Transform::translation(Vec3::new(10.0, 0.0, 0.0))
                .compose(original.transform),
            name: format!("{} (copy)", original.name),
            visible: original.visible,
            extruder_id: original.extruder_id,
            parent: original.parent,
        });
        let cloned_obj = self.objects.get(&new_id).unwrap().clone();
        Ok((new_id, vec![SceneEvent::ObjectAdded(cloned_obj)]))
    }

    /// Set the gizmo mode + pivot. Returns one event when state
    /// actually changed.
    pub fn set_gizmo(&mut self, new_gizmo: GizmoState) -> Vec<SceneEvent> {
        if (self.gizmo.mode == new_gizmo.mode)
            && (self.gizmo.pivot == new_gizmo.pivot)
        {
            return Vec::new();
        }
        self.gizmo = new_gizmo.clone();
        vec![SceneEvent::GizmoChanged(new_gizmo)]
    }

    /// Replace the camera state. Always emits an event (camera
    /// state's equality check is expensive enough to skip).
    pub fn set_camera(&mut self, camera: CameraState) -> Vec<SceneEvent> {
        self.camera = camera.clone();
        vec![SceneEvent::CameraChanged(camera)]
    }

    /// Add (or re-instance) a procedural primitive. Looks up the
    /// `(kind, params)` tuple in the cache; if missing, generates
    /// the mesh once and stashes the `MeshId`. Always creates a
    /// fresh SceneObject placed at plate origin so re-clicking
    /// "Add cube" in the library palette piles new objects on top
    /// rather than replacing the previous one.
    pub fn add_from_primitive(
        &mut self,
        kind: super::primitives::PrimitiveKind,
        params: super::primitives::PrimitiveParams,
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
                let new_mesh = super::primitives::generate(kind, params);
                let id = self.register_mesh(new_mesh);
                self.primitive_cache.push((kind, params, id));
                let header = self.meshes.get(&id).unwrap().header();
                events.push(SceneEvent::MeshLoaded(header));
                id
            }
        };

        let name = match kind {
            super::primitives::PrimitiveKind::Cube => "Cube",
            super::primitives::PrimitiveKind::Cylinder => "Cylinder",
            super::primitives::PrimitiveKind::Sphere => "Sphere",
            super::primitives::PrimitiveKind::Cone => "Cone",
            super::primitives::PrimitiveKind::Torus => "Torus",
        };
        let obj_id = self.register_object(NewSceneObject::at_origin(mesh_id, name));
        let obj_clone = self.objects.get(&obj_id).unwrap().clone();
        events.push(SceneEvent::ObjectAdded(obj_clone));
        events.extend(self.out_of_bounds_event(obj_id));
        (mesh_id, obj_id, events)
    }

    /// Install the active printer's bed. Recomputes the bed
    /// visualization, caches it on the scene state, and emits a
    /// `BedChanged` event the renderer subscribes to. Pass `None`
    /// to clear the bed (e.g., when the user closes the project).
    pub fn set_active_printer(
        &mut self,
        printer: Option<&crate::core::printer::profile::PrinterProfile>,
    ) -> Vec<SceneEvent> {
        let new_bed = printer.map(super::bed::bed_for_printer);
        // Mirror exclusion zones onto the scene's flat field for
        // consumers that read them directly (the snapshot wire
        // format keeps both — `bed.exclusion_zones` is the
        // authoritative copy, and `exclusion_zones` here is the
        // legacy view PR-2-2's snapshot already exposes).
        self.exclusion_zones = new_bed
            .as_ref()
            .map(|b| b.exclusion_zones.clone())
            .unwrap_or_default();
        self.bed = new_bed.clone();
        vec![SceneEvent::BedChanged(new_bed)]
    }

    /// Check `object_id` against the active bed and emit warnings
    /// for every reason it's out of bounds. No bed = no check
    /// (silently). Designed to be called by every transform op so
    /// the UI can flash a non-blocking warning the instant the
    /// user nudges an object off the plate.
    fn out_of_bounds_event(&self, object_id: ObjectId) -> Option<SceneEvent> {
        let bed = self.bed.as_ref()?;
        let obj = self.objects.get(&object_id)?;
        let mesh = self.meshes.get(&obj.mesh)?;
        let reasons = super::bed::object_out_of_bounds(obj, mesh, bed);
        if reasons.is_empty() {
            None
        } else {
            Some(SceneEvent::ObjectOutOfBounds {
                id: object_id,
                reasons,
            })
        }
    }

    /// Compute the world-space bounding box of all visible objects.
    /// Used by `Frame All` in the renderer.
    pub fn visible_bounds(&self) -> Option<BoundingBox> {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        let mut any = false;
        for obj in self.objects.values() {
            if !obj.visible {
                continue;
            }
            let mesh = match self.meshes.get(&obj.mesh) {
                Some(m) => m,
                None => continue,
            };
            // Apply the object's transform to each of the bounding
            // box's 8 corners and union them.
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
}

fn is_non_uniform(factor: Vec3) -> bool {
    let eps = 1e-5_f32;
    (factor.x - factor.y).abs() > eps
        || (factor.x - factor.z).abs() > eps
        || (factor.y - factor.z).abs() > eps
}

fn mesh_bb_corners(bb: &BoundingBox) -> [Vec3; 8] {
    let mn = [bb.min[0] as f32, bb.min[1] as f32, bb.min[2] as f32];
    let mx = [bb.max[0] as f32, bb.max[1] as f32, bb.max[2] as f32];
    [
        Vec3::new(mn[0], mn[1], mn[2]),
        Vec3::new(mx[0], mn[1], mn[2]),
        Vec3::new(mn[0], mx[1], mn[2]),
        Vec3::new(mx[0], mx[1], mn[2]),
        Vec3::new(mn[0], mn[1], mx[2]),
        Vec3::new(mx[0], mn[1], mx[2]),
        Vec3::new(mn[0], mx[1], mx[2]),
        Vec3::new(mx[0], mx[1], mx[2]),
    ]
}

/// Return (min_z, max_z) of the corners after applying `xform`.
fn z_extent(corners: &[Vec3; 8], xform: &glam::Mat4) -> (f32, f32) {
    let mut min_z = f32::INFINITY;
    let mut max_z = f32::NEG_INFINITY;
    for c in corners {
        let p = xform.transform_point3(*c);
        if p.z < min_z {
            min_z = p.z;
        }
        if p.z > max_z {
            max_z = p.z;
        }
    }
    (min_z, max_z)
}

/// The 24 proper rotations of a cube. Generated as compositions of
/// identity + (90/180/270)° around each of the three principal axes
/// — that yields 24 distinct rotations (the full chiral octahedral
/// group). Used by [`SceneState::lay_flat_object`] to pick the
/// orientation that minimizes the world-space Z extent.
fn cube_rotations() -> Vec<Quat> {
    use std::f32::consts::FRAC_PI_2;
    let face_rots = [
        Quat::IDENTITY,
        Quat::from_rotation_y(FRAC_PI_2),
        Quat::from_rotation_y(std::f32::consts::PI),
        Quat::from_rotation_y(-FRAC_PI_2),
        Quat::from_rotation_x(FRAC_PI_2),
        Quat::from_rotation_x(-FRAC_PI_2),
    ];
    let z_spins = [
        Quat::IDENTITY,
        Quat::from_rotation_z(FRAC_PI_2),
        Quat::from_rotation_z(std::f32::consts::PI),
        Quat::from_rotation_z(-FRAC_PI_2),
    ];
    let mut out = Vec::with_capacity(24);
    for f in face_rots {
        for s in z_spins {
            out.push(f * s);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit_cube_mesh() -> NewMesh {
        // 8-corner cube — enough geometry for tests that don't care
        // about visual quality. Normals left zeroed since the
        // mutation-method tests don't shade.
        NewMesh {
            vertices: vec![
                0.0, 0.0, 0.0, //
                1.0, 0.0, 0.0, //
                0.0, 1.0, 0.0, //
                1.0, 1.0, 0.0, //
                0.0, 0.0, 1.0, //
                1.0, 0.0, 1.0, //
                0.0, 1.0, 1.0, //
                1.0, 1.0, 1.0, //
            ],
            normals: vec![0.0; 24],
            indices: vec![
                0, 1, 2, 1, 3, 2, // bottom
                4, 6, 5, 5, 6, 7, // top
                0, 4, 1, 1, 4, 5, // front
                2, 3, 6, 3, 7, 6, // back
                0, 2, 4, 2, 6, 4, // left
                1, 5, 3, 3, 5, 7, // right
            ],
            bounding_box: BoundingBox {
                min: [0.0, 0.0, 0.0],
                max: [1.0, 1.0, 1.0],
            },
            provenance: MeshProvenance::Primitive("unit-cube".into()),
        }
    }

    #[test]
    fn empty_scene_has_no_visible_bounds() {
        let s = SceneState::new();
        assert!(s.visible_bounds().is_none());
    }

    #[test]
    fn monotonic_ids_dont_reuse() {
        let mut s = SceneState::new();
        let a = s.next_mesh_id();
        let b = s.next_mesh_id();
        let c = s.next_object_id();
        let d = s.next_object_id();
        // IDs start at 1.
        assert_eq!(a, MeshId(1));
        assert_eq!(b, MeshId(2));
        assert_eq!(c, ObjectId(1));
        assert_eq!(d, ObjectId(2));
    }

    #[test]
    fn register_mesh_allocates_monotonically() {
        let mut s = SceneState::new();
        let id = s.register_mesh(unit_cube_mesh());
        assert_eq!(id, MeshId(1));
        let id2 = s.register_mesh(unit_cube_mesh());
        assert_eq!(id2, MeshId(2));
        assert_ne!(id, id2);
    }

    #[test]
    fn visible_bounds_unions_objects() {
        let mut s = SceneState::new();
        let mesh_id = s.register_mesh(unit_cube_mesh());
        let _ = s.register_object(NewSceneObject::at_origin(mesh_id, "cube0"));
        let _ = s.register_object(NewSceneObject {
            transform: Transform::translation(Vec3::new(10.0, 0.0, 0.0)),
            name: "cube1".into(),
            ..NewSceneObject::at_origin(mesh_id, "cube1")
        });

        let bb = s.visible_bounds().expect("two visible objects");
        assert_eq!(bb.min, [0.0, 0.0, 0.0]);
        assert!((bb.max[0] - 11.0).abs() < 1e-3, "got {bb:?}");
    }

    #[test]
    fn invisible_objects_skipped_in_bounds() {
        let mut s = SceneState::new();
        let mesh_id = s.register_mesh(unit_cube_mesh());
        let _ = s.register_object(NewSceneObject {
            visible: false,
            transform: Transform::translation(Vec3::new(100.0, 0.0, 0.0)),
            ..NewSceneObject::at_origin(mesh_id, "hidden")
        });
        assert!(s.visible_bounds().is_none());
    }

    // ---- Mutation-method tests --------------------------------------

    fn add_cube(s: &mut SceneState) -> (MeshId, ObjectId) {
        let (mesh_id, obj_id, events) = s.load_mesh(unit_cube_mesh());
        assert_eq!(events.len(), 2, "load_mesh emits mesh_loaded + object_added");
        assert!(matches!(events[0], SceneEvent::MeshLoaded(_)));
        assert!(matches!(events[1], SceneEvent::ObjectAdded(_)));
        (mesh_id, obj_id)
    }

    #[test]
    fn load_then_select_then_translate_emits_expected_event_stream() {
        let mut s = SceneState::new();
        let (_mesh, obj) = add_cube(&mut s);

        let events = s.select(&[obj], SelectMode::Replace);
        assert_eq!(events.len(), 1);
        match &events[0] {
            SceneEvent::SelectionChanged { selected } => {
                assert_eq!(selected, &vec![obj]);
            }
            other => panic!("expected SelectionChanged, got {other:?}"),
        }

        let events = s.translate_object(obj, Vec3::new(5.0, 0.0, 0.0)).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            SceneEvent::ObjectUpdated(o) => {
                let center = o.transform.apply_point(Vec3::new(0.5, 0.5, 0.5));
                assert!((center - Vec3::new(5.5, 0.5, 0.5)).length() < 1e-5);
            }
            other => panic!("expected ObjectUpdated, got {other:?}"),
        }
    }

    #[test]
    fn select_no_op_emits_no_event() {
        let mut s = SceneState::new();
        let (_mesh, obj) = add_cube(&mut s);
        let _ = s.select(&[obj], SelectMode::Replace);
        let events = s.select(&[obj], SelectMode::Replace);
        assert!(events.is_empty(), "re-selecting same set is a no-op");
    }

    #[test]
    fn select_unknown_object_is_skipped() {
        let mut s = SceneState::new();
        let (_mesh, obj) = add_cube(&mut s);
        let events = s.select(&[obj, ObjectId(9999)], SelectMode::Replace);
        match &events[0] {
            SceneEvent::SelectionChanged { selected } => {
                assert_eq!(selected, &vec![obj], "unknown id filtered out");
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn deselect_all_emits_event_when_selection_was_nonempty() {
        let mut s = SceneState::new();
        let (_mesh, obj) = add_cube(&mut s);
        let _ = s.select(&[obj], SelectMode::Replace);
        let events = s.deselect_all();
        match &events[0] {
            SceneEvent::SelectionChanged { selected } => assert!(selected.is_empty()),
            _ => unreachable!(),
        }
        // Second deselect_all is a no-op.
        let events = s.deselect_all();
        assert!(events.is_empty());
    }

    #[test]
    fn rotate_around_object_center() {
        let mut s = SceneState::new();
        let (_mesh, obj) = add_cube(&mut s);
        // Cube center is at (0.5, 0.5, 0.5); rotate 180° around Z.
        let _ = s.rotate_object(obj, Vec3::Z, std::f32::consts::PI, None).unwrap();
        let o = s.objects.get(&obj).unwrap();
        // After rotation, the corner that was at (0,0,0) maps to
        // (1,1,0) (rotated 180° around the cube's center).
        let corner = o.transform.apply_point(Vec3::ZERO);
        assert!((corner - Vec3::new(1.0, 1.0, 0.0)).length() < 1e-4, "got {corner:?}");
    }

    #[test]
    fn rotate_with_explicit_pivot_preserves_relative_position() {
        let mut s = SceneState::new();
        let (_mesh, obj) = add_cube(&mut s);
        // Pivot at world origin; rotate 90° around Z.
        let _ = s
            .rotate_object(obj, Vec3::Z, std::f32::consts::FRAC_PI_2, Some(Vec3::ZERO))
            .unwrap();
        let o = s.objects.get(&obj).unwrap();
        // Corner (1,0,0) rotates around the origin to (0,1,0).
        let corner = o.transform.apply_point(Vec3::X);
        assert!((corner - Vec3::Y).length() < 1e-4, "got {corner:?}");
    }

    #[test]
    fn delete_objects_clears_selection_for_deleted_ids() {
        let mut s = SceneState::new();
        let (_mesh, obj1) = add_cube(&mut s);
        let (_mesh2, obj2) = add_cube(&mut s);
        let _ = s.select(&[obj1, obj2], SelectMode::Replace);
        let events = s.delete_objects(&[obj1]);
        // One ObjectRemoved + one SelectionChanged.
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], SceneEvent::ObjectRemoved { id } if id == obj1));
        match &events[1] {
            SceneEvent::SelectionChanged { selected } => {
                assert_eq!(selected, &vec![obj2]);
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn duplicate_object_offsets_by_10mm_x() {
        let mut s = SceneState::new();
        let (_mesh, obj) = add_cube(&mut s);
        let (new_id, events) = s.duplicate_object(obj).unwrap();
        assert_ne!(new_id, obj);
        match &events[0] {
            SceneEvent::ObjectAdded(o) => {
                let new_corner = o.transform.apply_point(Vec3::ZERO);
                assert!((new_corner - Vec3::new(10.0, 0.0, 0.0)).length() < 1e-5);
                assert!(o.name.contains("(copy)"));
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn unknown_object_ops_return_error() {
        let mut s = SceneState::new();
        let bad = ObjectId(42);
        assert!(matches!(
            s.translate_object(bad, Vec3::ZERO),
            Err(SceneOpError::UnknownObject(_))
        ));
        assert!(matches!(
            s.rotate_object(bad, Vec3::Z, 0.0, None),
            Err(SceneOpError::UnknownObject(_))
        ));
        assert!(matches!(
            s.delete_objects(&[bad]),
            ref v if v.is_empty()
        ), "delete of unknown id is a no-op, not an error");
    }

    #[test]
    fn gizmo_change_no_op_emits_nothing() {
        let mut s = SceneState::new();
        let initial = s.gizmo.clone();
        let events = s.set_gizmo(initial);
        assert!(events.is_empty());
        let mut next = s.gizmo.clone();
        next.mode = GizmoMode::Rotate;
        let events = s.set_gizmo(next);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], SceneEvent::GizmoChanged(_)));
    }

    #[test]
    fn full_state_round_trips_via_json() {
        let mut s = SceneState::new();
        let mesh_id = s.register_mesh(unit_cube_mesh());
        let _ = s.register_object(NewSceneObject {
            mesh: mesh_id,
            transform: Transform::translation(Vec3::new(5.0, 5.0, 0.0)),
            name: "test-cube".into(),
            visible: true,
            extruder_id: Some(2),
            parent: None,
        });
        s.gizmo.mode = GizmoMode::Translate;

        let json = serde_json::to_string(&s).unwrap();
        let parsed: SceneState = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.meshes.len(), 1);
        assert_eq!(parsed.objects.len(), 1);
        let obj = parsed.objects.values().next().unwrap();
        assert_eq!(obj.name, "test-cube");
        assert_eq!(obj.extruder_id, Some(2));
        assert_eq!(parsed.gizmo.mode, GizmoMode::Translate);
    }

    #[test]
    fn double_mirror_across_x_returns_to_original() {
        let mut s = SceneState::new();
        let (_, obj) = add_cube(&mut s);
        let probe = Vec3::new(0.25, 0.5, 0.5);
        let before = s.objects.get(&obj).unwrap().transform.apply_point(probe);

        s.mirror_object(obj, MirrorAxis::X).unwrap();
        s.mirror_object(obj, MirrorAxis::X).unwrap();

        let after = s.objects.get(&obj).unwrap().transform.apply_point(probe);
        assert!(
            (after - before).length() < 1e-4,
            "double mirror drifted: {before:?} → {after:?}"
        );
    }

    #[test]
    fn mirror_x_flips_x_through_world_center() {
        let mut s = SceneState::new();
        let (_, obj) = add_cube(&mut s);
        // Cube is unit-extent at origin. World-space center is
        // (0.5, 0.5, 0.5). Probe (1, 0.5, 0.5) should flip to
        // (0, 0.5, 0.5) after mirror-around-center across X.
        s.mirror_object(obj, MirrorAxis::X).unwrap();
        let probe = Vec3::new(1.0, 0.5, 0.5);
        let mirrored = s.objects.get(&obj).unwrap().transform.apply_point(probe);
        assert!(
            (mirrored - Vec3::new(0.0, 0.5, 0.5)).length() < 1e-5,
            "got {mirrored:?}"
        );
    }

    #[test]
    fn non_uniform_scale_emits_warning_event() {
        let mut s = SceneState::new();
        let (_, obj) = add_cube(&mut s);
        let events = s.scale_object(obj, Vec3::new(2.0, 1.0, 1.0)).unwrap();
        assert!(matches!(events[0], SceneEvent::ObjectUpdated(_)));
        assert!(
            matches!(events.get(1), Some(SceneEvent::NonUniformScale { id }) if *id == obj),
            "expected NonUniformScale, got {events:?}"
        );
    }

    #[test]
    fn uniform_scale_does_not_emit_warning() {
        let mut s = SceneState::new();
        let (_, obj) = add_cube(&mut s);
        let events = s.scale_object(obj, Vec3::new(1.5, 1.5, 1.5)).unwrap();
        assert_eq!(events.len(), 1, "uniform scale: only ObjectUpdated");
        assert!(matches!(events[0], SceneEvent::ObjectUpdated(_)));
    }

    #[test]
    fn lay_flat_settles_rotated_cube_to_z_zero_min() {
        let mut s = SceneState::new();
        let (_, obj) = add_cube(&mut s);
        // Push the cube up, then tilt it 30° around X so its Z
        // extent is bigger than 1. lay_flat should re-orient and
        // drop min Z back to 0.
        s.translate_object(obj, Vec3::new(0.0, 0.0, 5.0)).unwrap();
        s.rotate_object(obj, Vec3::X, 0.5, None).unwrap();
        s.lay_flat_object(obj).unwrap();

        // Recompute world-space bbox of the cube after lay_flat.
        let xform = s.objects.get(&obj).unwrap().transform;
        let mesh_bb = &s.meshes.values().next().unwrap().bounding_box;
        let mut min_z = f32::INFINITY;
        let mut max_z = f32::NEG_INFINITY;
        for &x in &[mesh_bb.min[0] as f32, mesh_bb.max[0] as f32] {
            for &y in &[mesh_bb.min[1] as f32, mesh_bb.max[1] as f32] {
                for &z in &[mesh_bb.min[2] as f32, mesh_bb.max[2] as f32] {
                    let p = xform.apply_point(Vec3::new(x, y, z));
                    min_z = min_z.min(p.z);
                    max_z = max_z.max(p.z);
                }
            }
        }
        assert!(min_z.abs() < 1e-4, "expected min_z ≈ 0, got {min_z}");
        // The cube is symmetric so the smallest Z extent is 1.
        assert!(
            (max_z - 1.0).abs() < 1e-4,
            "expected extent 1, got max_z={max_z}"
        );
    }

    #[test]
    fn add_from_primitive_dedups_same_params_to_one_mesh() {
        use super::super::primitives::{PrimitiveKind, PrimitiveParams};
        let mut s = SceneState::new();
        let p = PrimitiveParams {
            width: 20.0,
            depth: 20.0,
            height: 20.0,
            radius: 0.0,
            radial_segments: 0,
        };
        let (m1, o1, _) = s.add_from_primitive(PrimitiveKind::Cube, p);
        let (m2, o2, _) = s.add_from_primitive(PrimitiveKind::Cube, p);
        assert_eq!(
            m1, m2,
            "same (kind, params) should share one mesh after dedup"
        );
        assert_ne!(o1, o2, "each call creates a fresh object");
        // Cache should only have one entry.
        assert_eq!(s.meshes.len(), 1, "only one mesh registered");
    }

    #[test]
    fn add_from_primitive_with_different_params_creates_new_mesh() {
        use super::super::primitives::{PrimitiveKind, PrimitiveParams};
        let mut s = SceneState::new();
        let p1 = PrimitiveParams {
            width: 20.0,
            depth: 20.0,
            height: 20.0,
            radius: 0.0,
            radial_segments: 0,
        };
        let p2 = PrimitiveParams { width: 30.0, ..p1 };
        let (m1, _, _) = s.add_from_primitive(PrimitiveKind::Cube, p1);
        let (m2, _, _) = s.add_from_primitive(PrimitiveKind::Cube, p2);
        assert_ne!(m1, m2, "different params allocate distinct meshes");
        assert_eq!(s.meshes.len(), 2);
    }

    #[test]
    fn add_from_primitive_emits_mesh_loaded_only_first_time() {
        use super::super::primitives::{PrimitiveKind, PrimitiveParams};
        let mut s = SceneState::new();
        let p = PrimitiveParams::defaults_for(PrimitiveKind::Cube);
        let (_, _, events1) = s.add_from_primitive(PrimitiveKind::Cube, p);
        assert!(
            events1
                .iter()
                .any(|e| matches!(e, SceneEvent::MeshLoaded(_))),
            "first add emits MeshLoaded"
        );
        let (_, _, events2) = s.add_from_primitive(PrimitiveKind::Cube, p);
        assert!(
            !events2
                .iter()
                .any(|e| matches!(e, SceneEvent::MeshLoaded(_))),
            "second add (dedup) does not re-emit MeshLoaded"
        );
        assert!(events2
            .iter()
            .any(|e| matches!(e, SceneEvent::ObjectAdded(_))));
    }

    #[test]
    fn transform_op_with_no_bed_emits_no_oob_event() {
        let mut s = SceneState::new();
        let (_, obj) = add_cube(&mut s);
        let events = s.translate_object(obj, Vec3::new(500.0, 0.0, 0.0)).unwrap();
        // Without an active printer, the bed is None so OOB is silent.
        assert!(events
            .iter()
            .all(|e| !matches!(e, SceneEvent::ObjectOutOfBounds { .. })));
    }

    fn a1_mini_for_test() -> crate::core::printer::profile::PrinterProfile {
        use crate::core::printer::profile::{BoundingBox, PrinterProfile, Toolhead};
        PrinterProfile {
            model: "Bambu A1 mini".into(),
            slot_count: 4,
            supported_build_plates: vec!["Textured PEI".into()],
            toolheads: vec![Toolhead {
                nozzle_diameter: 0.4,
                hotend_type: "stainless_steel".into(),
                max_temp: 300.0,
                slot_indices: vec![0, 1, 2, 3],
            }],
            build_volume: BoundingBox {
                min: [0.0, 0.0, 0.0],
                max: [180.0, 180.0, 180.0],
            },
            exclusion_zones: vec![],
        }
    }

    #[test]
    fn translate_off_plate_emits_oob_warning_after_active_printer_set() {
        let mut s = SceneState::new();
        let (_, obj) = add_cube(&mut s);
        let bed_events = s.set_active_printer(Some(&a1_mini_for_test()));
        assert!(matches!(bed_events[0], SceneEvent::BedChanged(Some(_))));

        // 50 mm in: still inside the 180x180x180 build volume.
        let events = s.translate_object(obj, Vec3::new(50.0, 0.0, 0.0)).unwrap();
        assert!(events
            .iter()
            .all(|e| !matches!(e, SceneEvent::ObjectOutOfBounds { .. })));

        // Another 200 mm in X: now off the bed.
        let events = s.translate_object(obj, Vec3::new(200.0, 0.0, 0.0)).unwrap();
        assert!(events.iter().any(|e| matches!(
            e,
            SceneEvent::ObjectOutOfBounds { id, reasons } if *id == obj && !reasons.is_empty()
        )));
    }

    #[test]
    fn rotate_around_explicit_pivot_below_plate_emits_below_plate_reason() {
        use super::super::bed::OutOfBoundsReason;
        let mut s = SceneState::new();
        let (_, obj) = add_cube(&mut s);
        s.set_active_printer(Some(&a1_mini_for_test()));
        // Cube starts at world [0..1, 0..1, 0..1]. Rotate 180° around
        // the X-axis through the origin (explicit pivot at (0,0,0))
        // — that's a Y-and-Z flip about origin, so cube ends up at
        // [0..1, -1..0, -1..0]. min_z = -1 → BelowBuildPlate.
        let events = s
            .rotate_object(
                obj,
                Vec3::X,
                std::f32::consts::PI,
                Some(Vec3::new(0.0, 0.0, 0.0)),
            )
            .unwrap();
        let oob = events
            .iter()
            .find_map(|e| match e {
                SceneEvent::ObjectOutOfBounds { reasons, .. } => Some(reasons),
                _ => None,
            })
            .expect("expected OOB event");
        assert!(
            oob.iter()
                .any(|r| matches!(r, OutOfBoundsReason::BelowBuildPlate)),
            "expected BelowBuildPlate among {oob:?}",
        );
    }

    #[test]
    fn cube_rotations_are_24_distinct() {
        let rots = cube_rotations();
        assert_eq!(rots.len(), 24);
        // Verify uniqueness: any two rotations applied to (1,2,3)
        // should produce different points (or be the same rotation,
        // which we check via near-equality).
        let probe = Vec3::new(1.0, 2.0, 3.0);
        let points: Vec<Vec3> = rots.iter().map(|q| *q * probe).collect();
        let mut distinct = 0;
        for (i, a) in points.iter().enumerate() {
            let unique = !points
                .iter()
                .take(i)
                .any(|b| (a - b).length() < 1e-3);
            if unique {
                distinct += 1;
            }
        }
        assert_eq!(distinct, 24, "expected 24 distinct rotations");
    }
}
