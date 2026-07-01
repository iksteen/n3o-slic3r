//! Per-plate scene-state types + scene-primitive types (mesh, object,
//! bed).
//!
//! [`PlateSceneState`] is the per-plate scene contents one entry of
//! [`crate::core::project::Project::plates`] carries via
//! `Plate.scene`. Root state + scene-wide registries (mesh storage,
//! id allocators, primitive cache) live on `Project`; mutation
//! methods live in `core::project::mutation`. This file holds the
//! type primitives + small geometry helpers the mutation module
//! consumes.
//!
//! Per the AD-8 invariant: `Project` (in Tauri's `Mutex<Project>`)
//! is the *only* place authoritative scene state lives. The
//! renderer's local mirror is a cache; the snapshot command rebuilds
//! it from scratch on reconnect.

use super::build_plate::BuildPlate;
use super::transform::Transform;
use crate::core::printer::profile::BoundingBox;
use glam::Vec3;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Opaque mesh identifier. Monotonic across the registry's lifetime;
/// never reused even after a mesh is freed. Surfaced to the frontend
/// as a string-encoded number via the standard serde representation.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MeshId(pub u64);

/// Opaque scene-object identifier. Same monotonic-never-reused
/// semantics as `MeshId`.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ObjectId(pub u64);

/// Stable, collision-free group identity (a UUID). Unlike a reused
/// integer high-water mark, it never collides on import/merge and
/// round-trips without renumbering, so it's a sound durable key for the
/// per-plate group state in [`PlateSceneState::groups`].
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GroupId(pub Uuid);

impl GroupId {
    /// Allocate a fresh, globally-unique group id.
    pub fn fresh() -> Self {
        Self(Uuid::new_v4())
    }
}

/// Per-group state. Members reference the group via
/// [`SceneObject::group`]; this carries the display name (and is the
/// natural home for future per-group state — print order, color, …).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Group {
    pub name: String,
}

/// Loaded mesh data + provenance.
///
/// The vertex / normal / index buffers live on the Rust side and
/// never cross IPC — the wgpu renderer uploads them straight to the
/// GPU Rust-side, keyed by [`MeshId`]. The `#[serde(skip)]`
/// annotations enforce that — sending a 47 MB mesh as JSON arrays
/// would be ~100 MB of stringified floats and gigabytes of JS-side
/// parse work.
///
/// Wire-side consumers see [`MeshHeader`] (lightweight metadata) only.
///
/// For multi-volume objects (3MF), each volume is its own `Mesh`
/// and the source 3MF spawns one `SceneObject` per volume.
///
/// **Eventual home: `core/geometry/`.** `Mesh` / `MeshHeader` /
/// `NewMesh` / `MeshProvenance` are general mesh-data types, not
/// scene-state types. `core/threemf` already imports them upward;
/// when a third consumer appears, extract them into a sibling
/// `core/geometry/` module so the dep direction stops being
/// scene→peers and starts being peers→geometry. See
/// `core/scene/mod.rs` for the architectural review note.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mesh {
    pub id: MeshId,
    /// Flat `[x0, y0, z0, x1, y1, z1, ...]` packed vertices. Not
    /// serialized — uploaded GPU-side, never crosses IPC.
    #[serde(skip, default)]
    pub vertices: std::sync::Arc<Vec<f32>>,
    /// Triangle vertex indices (3 per triangle). Not serialized.
    #[serde(skip, default)]
    pub indices: std::sync::Arc<Vec<u32>>,
    /// BBS per-triangle `paint_color` (MMU color-painting) strings, one
    /// per triangle in `indices`-triple order. `None` when the mesh has no
    /// painting; otherwise dense (`""` for unpainted faces). Opaque —
    /// libslic3r owns the encoding; we round-trip it through the slice
    /// `.3mf` so painted models slice multi-material. Not serialized (it
    /// travels with the mesh geometry in the 3MF, like the buffers above).
    #[serde(skip, default)]
    pub paint_colors: Option<std::sync::Arc<Vec<String>>>,
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

}

/// JSON-wire shape of a `Mesh` — everything except the heavy buffers.
/// `scene:mesh_loaded` events and `scene_snapshot` carry this; it's
/// the only mesh data on the JSON wire (the buffers stay GPU-side).
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
    /// Procedural primitive (object library).
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
    /// "inherits from project default"; multi-color 3MFs populate
    /// this from the source file's `model_settings.config`
    /// extruder metadata.
    #[serde(default)]
    pub extruder_id: Option<u8>,
    /// Group membership. Objects sharing the same `Some(GroupId)` are
    /// volumes of one logical object (e.g. a cube with upper + lower
    /// halves painted different colors). `None` = solo object. Populated
    /// by the 3mf loader from BBS-style `<components>` + per-`<part>`
    /// model_settings entries and by user grouping; the writer and slice
    /// path emit a group as one ModelObject with multiple ModelVolumes so
    /// libslic3r doesn't treat each volume as a separate floating object.
    /// The group's name lives in [`PlateSceneState::groups`].
    #[serde(default)]
    pub group: Option<GroupId>,
}

/// A reassembly-connector volume attached to an object (keyed in
/// [`PlateSceneState::object_modifiers`]): a cut peg (printed solid) or hole
/// (subtracted). Kept out of the object's main mesh so the cut needs no 3D
/// boolean — the slice path applies it as a libslic3r MODEL_PART /
/// NEGATIVE_VOLUME volume, subtracted per-layer in 2D. The geometry lives in the
/// mesh pool keyed by `mesh`, posed in the object's local frame.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Modifier {
    pub mesh: MeshId,
    pub kind: ModifierKind,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ModifierKind {
    /// Positive volume — a peg, printed as part of the object.
    Peg,
    /// Negative volume — a hole, subtracted from the object at slice time.
    Hole,
}

/// Silhouette of a hole marker — mirrors `slic3r_ffi::ConnectorShape` with serde
/// so it can persist in the project file (the FFI enum has no serde derives).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum HoleMarkerShape {
    Triangle,
    Square,
    Hexagon,
    Circle,
}

impl From<slic3r_ffi::ConnectorShape> for HoleMarkerShape {
    fn from(s: slic3r_ffi::ConnectorShape) -> Self {
        use slic3r_ffi::ConnectorShape as C;
        match s {
            C::Triangle => Self::Triangle,
            C::Square => Self::Square,
            C::Hexagon => Self::Hexagon,
            C::Circle => Self::Circle,
        }
    }
}

impl HoleMarkerShape {
    /// Ring-vertex count the viewport uses to draw the silhouette.
    pub fn segments(self) -> u32 {
        match self {
            Self::Triangle => 3,
            Self::Square => 4,
            Self::Hexagon => 6,
            Self::Circle => 28,
        }
    }
}

/// Display-only mark of a hole's opening on a cut cross-section, keyed in
/// [`PlateSceneState::object_hole_markers`]. Not geometry: the viewport shades
/// the cap's own fragments within this silhouette (the [`ModifierKind::Hole`]
/// volume does the actual subtraction). Stored in the object's local frame.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct HoleMarker {
    pub shape: HoleMarkerShape,
    /// Opening radius, tolerance included (matches the subtracted hole).
    pub radius: f32,
    pub center: [f32; 3],
    /// Cut-plane normal the opening lies in.
    pub normal: [f32; 3],
    /// In-plane direction of the silhouette's first vertex (object-local),
    /// captured from the connector's actual orientation so the drawn polygon's
    /// corners line up with the peg/hole geometry. Unused for circles.
    pub u_axis: [f32; 3],
}

/// Caller-builds-this shape for inserting a fresh mesh. No `id`
/// field — `Project::register_mesh` allocates it. Use for loaders + the
/// procedural-primitive path.
#[derive(Debug, Clone)]
pub struct NewMesh {
    pub vertices: Vec<f32>,
    pub indices: Vec<u32>,
    /// Per-triangle `paint_color` strings (see [`Mesh::paint_colors`]).
    /// `None` for procedurally-generated / unpainted meshes.
    pub paint_colors: Option<Vec<String>>,
    pub bounding_box: BoundingBox,
    pub provenance: MeshProvenance,
}

/// Caller-builds-this shape for inserting a fresh scene object. No
/// `id` field — `Project::register_object` allocates it.
#[derive(Debug, Clone)]
pub struct NewSceneObject {
    pub mesh: MeshId,
    pub transform: Transform,
    pub name: String,
    pub visible: bool,
    pub extruder_id: Option<u8>,
    /// See [`SceneObject::group`]. Loaders populate; most procedural /
    /// user-add call sites pass `None`.
    pub group: Option<GroupId>,
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
            group: None,
        }
    }
}

fn default_visible() -> bool {
    true
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

/// Per-plate scene state — everything one plate owns: placed
/// objects, selection, bed + exclusion zones,
/// active-build-plate identity. Phase 5 turns the historically
/// single-global scene into N of these.
///
/// Mesh data + the primitive-dedup cache + ID allocators are
/// **not** here — they live scene-wide on [`crate::core::project::Project`]
/// so a move-between-plates op doesn't have to copy mesh
/// buffers, and ids stay unique across plates.
/// The plate's objects, in **authored order** — position is the order of
/// record. It's what the Objects panel shows top-to-bottom and the order
/// libslic3r sees (where two objects of different materials overlap, the later
/// one wins), so it must be stable and reproducible — never HashMap iteration.
///
/// Storage is a `Vec` (order is the data; reorder = move an element;
/// serialization is a plain ordered array) **plus** an `id → index` map so
/// the by-id access every command needs stays O(1) — a Vec scan per lookup
/// would be wasteful given everything addresses objects by id. The map is
/// rebuilt on the rare structural ops (remove / reorder); pushes and lookups
/// touch it in O(1). The two are kept in sync internally — the fields are
/// private and only these methods mutate them. `Deref<Target=[SceneObject]>`
/// gives ordered iteration, `len`, `is_empty`, and indexing for free.
#[derive(Debug, Clone, Default)]
pub struct ObjectList {
    /// Objects in authored order — the source of truth for both contents and
    /// order. Owns the `SceneObject`s.
    objects: Vec<SceneObject>,
    /// `ObjectId` → its position in `objects`. A lookup acceleration structure,
    /// always kept consistent with `objects` by the mutators below.
    index: HashMap<ObjectId, usize>,
}

/// A set of object ids **proven to be in authored order**, the order of the
/// [`ObjectList`] they came from.
///
/// Its field is private and the only constructor is [`ObjectList::in_order`],
/// so an `OrderedIds` can never carry a caller's incidental ordering (e.g. a
/// `HashSet` iteration). Order-sensitive mutations — anything that materializes
/// or relocates a selection and thereby sets object order — take an
/// `OrderedIds` rather than a raw `&[ObjectId]`, so the type system forces order
/// to trace back to the authoritative object list. Derefs to `[ObjectId]` for
/// reading.
#[derive(Debug, Clone)]
pub struct OrderedIds(Vec<ObjectId>);

impl std::ops::Deref for OrderedIds {
    type Target = [ObjectId];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl ObjectList {
    fn from_objects(objects: Vec<SceneObject>) -> Self {
        let mut list = Self {
            objects,
            index: HashMap::new(),
        };
        list.rebuild_index();
        list
    }

    /// Re-derive `index` from `objects`. Called after any op that shifts
    /// positions (remove / reorder); O(n), but those ops are rare.
    fn rebuild_index(&mut self) {
        self.index = self
            .objects
            .iter()
            .enumerate()
            .map(|(i, o)| (o.id, i))
            .collect();
    }

    // The read surface mirrors `HashMap<ObjectId, SceneObject>` (taking
    // `&ObjectId`) so call sites read identically — but `get`/`get_mut`/
    // `contains_key` are O(1) via `index`, and `iter`/`values`/`keys` yield
    // results in authored (Vec) order instead of HashMap's random order.

    /// The object with this id, if present. O(1).
    pub fn get(&self, id: &ObjectId) -> Option<&SceneObject> {
        self.index.get(id).map(|&i| &self.objects[i])
    }

    /// Mutable handle to the object with this id, if present. O(1).
    pub fn get_mut(&mut self, id: &ObjectId) -> Option<&mut SceneObject> {
        let i = *self.index.get(id)?;
        Some(&mut self.objects[i])
    }

    /// Whether an object with this id is on the plate. O(1).
    pub fn contains_key(&self, id: &ObjectId) -> bool {
        self.index.contains_key(id)
    }

    /// Objects in authored order.
    pub fn values(&self) -> std::slice::Iter<'_, SceneObject> {
        self.objects.iter()
    }

    /// Objects in authored order, mutable. Safe: field edits can't change ids
    /// or the set, so `index` stays valid.
    pub fn values_mut(&mut self) -> std::slice::IterMut<'_, SceneObject> {
        self.objects.iter_mut()
    }

    /// `(id, object)` pairs in authored order — mirrors `HashMap::iter`.
    pub fn iter(&self) -> impl Iterator<Item = (&ObjectId, &SceneObject)> {
        self.objects.iter().map(|o| (&o.id, o))
    }

    /// Object ids in authored order.
    pub fn keys(&self) -> impl Iterator<Item = &ObjectId> {
        self.objects.iter().map(|o| &o.id)
    }

    /// The given ids in **authored order** (this list's order), as an
    /// [`OrderedIds`] — the proof-type an order-sensitive op (clone,
    /// move-to-plate) requires. `ids` is treated as an unordered *set*: its own
    /// order is ignored (it routinely arrives from a HashSet), unknown ids are
    /// dropped. This is the **only** way to construct an `OrderedIds`, so order
    /// always traces back to the authoritative object list — never a caller's
    /// incidental ordering.
    pub fn in_order(&self, ids: &[ObjectId]) -> OrderedIds {
        let want: HashSet<ObjectId> = ids.iter().copied().collect();
        OrderedIds(
            self.objects
                .iter()
                .filter(|o| want.contains(&o.id))
                .map(|o| o.id)
                .collect(),
        )
    }

    pub fn len(&self) -> usize {
        self.objects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    /// Append an object (ids are scene-wide unique, so this is never an
    /// upsert — a fresh object lands at the end / newest position). O(1).
    pub fn push(&mut self, obj: SceneObject) {
        self.index.insert(obj.id, self.objects.len());
        self.objects.push(obj);
    }

    /// Remove the object with this id, preserving the order of the rest.
    /// Returns it if it was present.
    pub fn remove(&mut self, id: &ObjectId) -> Option<SceneObject> {
        let i = self.index.get(id).copied()?;
        let obj = self.objects.remove(i);
        self.rebuild_index();
        Some(obj)
    }

    /// Move `id` to `new_index` (clamped), shifting the rest — the primitive a
    /// future "reorder in the panel" gesture drives. No-op if `id` is absent.
    pub fn move_object(&mut self, id: &ObjectId, new_index: usize) {
        let Some(from) = self.index.get(id).copied() else {
            return;
        };
        let to = new_index.min(self.objects.len() - 1);
        if from == to {
            return;
        }
        let obj = self.objects.remove(from);
        self.objects.insert(to, obj);
        self.rebuild_index();
    }
}

// `objects[&id]` panics on a missing key, mirroring `HashMap`'s indexing.
impl std::ops::Index<&ObjectId> for ObjectList {
    type Output = SceneObject;
    fn index(&self, id: &ObjectId) -> &SceneObject {
        self.get(id).expect("no object with that id")
    }
}

impl std::ops::IndexMut<&ObjectId> for ObjectList {
    fn index_mut(&mut self, id: &ObjectId) -> &mut SceneObject {
        self.get_mut(id).expect("no object with that id")
    }
}

// Persisted as a plain ordered array of objects; `index` is rebuilt on load.
impl Serialize for ObjectList {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.objects.serialize(s)
    }
}

impl<'de> Deserialize<'de> for ObjectList {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Self::from_objects(Vec::<SceneObject>::deserialize(d)?))
    }
}

/// Per-plate scene state — everything one plate owns: placed
/// objects, selection, bed + exclusion zones,
/// active-build-plate identity. Phase 5 turns the historically
/// single-global scene into N of these.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlateSceneState {
    pub objects: ObjectList,
    /// Live selection — transient UI state, deliberately **not
    /// persisted**. A reopened project starts with nothing selected.
    #[serde(skip)]
    pub selection: HashSet<ObjectId>,
    /// Vestigial active-build-plate handle — never assigned (the active
    /// build plate is a property of the bound `PrinterInstance`, not the
    /// project), so **not persisted**.
    #[serde(skip)]
    pub plate: Option<ActivePlate>,
    /// Derived from the bound printer profile alongside `bed`. **Not
    /// persisted** — re-derived on load with `bed` (see below).
    #[serde(skip)]
    pub exclusion_zones: Vec<ExclusionZone>,
    /// Active bed visualization + bounds. `None` when no printer is
    /// selected yet for this plate — loading meshes still works, just
    /// without the out-of-bounds check or grid. **Not persisted**: it's a
    /// pure function of the bound printer profile, so `format::read_project`
    /// re-derives it (and `exclusion_zones`) via `Plate::set_printer` on
    /// load rather than storing a snapshot that could drift from the profile.
    #[serde(skip)]
    pub bed: Option<super::bed::BedMesh>,
    /// Per-object cascade overrides. Outer key: object; inner map:
    /// **logical** cascade key → serialized value. The resolver consumes
    /// this as the highest-priority tier when the panel resolves with the
    /// Object tab active. Keys are logical, not libslic3r — the adapter
    /// translates to libslic3r vocabulary only at slice time. Empty for
    /// objects without authored overrides.
    #[serde(default)]
    pub object_overrides: HashMap<ObjectId, HashMap<String, String>>,
    /// Per-group state keyed by [`GroupId`] — the display name today.
    /// A group is a set of objects sharing a `SceneObject::group`; the
    /// membership persists on the objects, this map carries the name
    /// (and room for future per-group state). Empty for groups without
    /// an explicit name (the UI shows a default).
    #[serde(default)]
    pub groups: HashMap<GroupId, Group>,
    /// Per-object connector volumes (cut pegs/holes) keyed by object. Resolved
    /// at slice time as positive/negative volumes rather than baked into the
    /// object mesh (see [`Modifier`]). Empty for objects without connectors.
    #[serde(default)]
    pub object_modifiers: HashMap<ObjectId, Vec<Modifier>>,
    /// Per-object hole-opening markers (cut cross-section decals) keyed by
    /// object. Display-only — the viewport shades them onto the cap; never
    /// sliced. Empty for objects without cut holes.
    #[serde(default)]
    pub object_hole_markers: HashMap<ObjectId, Vec<HoleMarker>>,
}

/// 8 corners of a mesh's axis-aligned bounding box, as world-space
/// `Vec3`s. Transform helper for computing post-rotation /
/// post-mirror extents of an object's footprint.
pub(crate) fn mesh_bb_corners(bb: &BoundingBox) -> [Vec3; 8] {
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

#[cfg(test)]
mod object_list_tests {
    use super::*;

    fn obj(id: u64) -> SceneObject {
        SceneObject {
            id: ObjectId(id),
            mesh: MeshId(1),
            transform: Transform::IDENTITY,
            name: format!("o{id}"),
            visible: true,
            extruder_id: None,
            group: None,
        }
    }

    fn ids(list: &ObjectList) -> Vec<u64> {
        list.values().map(|o| o.id.0).collect()
    }

    #[test]
    fn push_keeps_order_and_indexes_by_id() {
        let mut list = ObjectList::default();
        for id in [10, 7, 22, 3] {
            list.push(obj(id));
        }
        assert_eq!(ids(&list), [10, 7, 22, 3], "values() is authored order");
        // by-id lookup resolves regardless of position
        assert_eq!(list.get(&ObjectId(22)).unwrap().name, "o22");
        assert!(list.contains_key(&ObjectId(7)));
        assert!(list.get(&ObjectId(999)).is_none());
    }

    #[test]
    fn remove_preserves_order_and_rebuilds_index() {
        let mut list = ObjectList::default();
        for id in [1, 2, 3, 4] {
            list.push(obj(id));
        }
        assert_eq!(list.remove(&ObjectId(2)).unwrap().id, ObjectId(2));
        assert_eq!(ids(&list), [1, 3, 4]);
        // the index must still point shifted elements at the right object
        assert_eq!(list.get(&ObjectId(4)).unwrap().name, "o4");
        assert!(!list.contains_key(&ObjectId(2)));
        assert!(list.remove(&ObjectId(2)).is_none());
    }

    #[test]
    fn move_object_reorders_and_keeps_index_valid() {
        let mut list = ObjectList::default();
        for id in [1, 2, 3, 4] {
            list.push(obj(id));
        }
        list.move_object(&ObjectId(4), 0);
        assert_eq!(ids(&list), [4, 1, 2, 3]);
        list.move_object(&ObjectId(4), 99); // index clamps to the end
        assert_eq!(ids(&list), [1, 2, 3, 4]);
        assert_eq!(list.get(&ObjectId(1)).unwrap().name, "o1");
    }

    #[test]
    fn serde_round_trips_as_ordered_array() {
        let mut list = ObjectList::default();
        for id in [5, 1, 9] {
            list.push(obj(id));
        }
        let json = serde_json::to_string(&list).unwrap();
        assert!(json.starts_with('['), "serializes as an array: {json}");
        let back: ObjectList = serde_json::from_str(&json).unwrap();
        assert_eq!(ids(&back), [5, 1, 9], "order survives the round-trip");
        // index is rebuilt on load, so by-id lookup works again
        assert_eq!(back.get(&ObjectId(9)).unwrap().name, "o9");
    }
}
