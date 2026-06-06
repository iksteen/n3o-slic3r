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
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

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
///
/// **Eventual home: `core/geometry/`.** `Mesh` / `MeshHeader` /
/// `NewMesh` / `MeshProvenance` are general mesh-data types, not
/// scene-state types. `core/threemf` already imports them upward;
/// when a third consumer appears (Phase 6 preview's mesh-handle
/// plumbing is the likely candidate), extract them into a sibling
/// `core/geometry/` module so the dep direction stops being
/// scene→peers and starts being peers→geometry. See
/// `core/scene/mod.rs` for the architectural review note.
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
    /// BBS per-triangle `paint_color` (MMU color-painting) strings, one
    /// per triangle in `indices`-triple order. `None` when the mesh has no
    /// painting; otherwise dense (`""` for unpainted faces). Opaque —
    /// libslic3r owns the encoding; we round-trip it through the slice
    /// `.3mf` so painted models slice multi-material. Not serialized (it
    /// travels with the mesh geometry in the 3MF, like the buffers above).
    #[serde(skip, default)]
    pub paint_colors: Option<Vec<String>>,
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
    /// Parent object — `None` for top-level objects. Hierarchical
    /// grouping (Phase 5 multi-plate) builds on this.
    #[serde(default)]
    pub parent: Option<ObjectId>,
    /// Multi-volume group identity. Objects sharing the same
    /// `Some(id)` are volumes of one logical object (e.g. a cube
    /// with upper + lower halves painted different colors). `None` =
    /// solo object. Populated by the 3mf loader from BBS-style
    /// `<components>` + per-`<part>` model_settings entries; the
    /// writer and slice path emit groups as one ModelObject with
    /// multiple ModelVolumes so libslic3r doesn't treat each volume
    /// as a separate floating object. The id is scoped per-Project
    /// and not stable across loads.
    #[serde(default)]
    pub group_id: Option<u32>,
}

/// Caller-builds-this shape for inserting a fresh mesh. No `id`
/// field — `Project::register_mesh` allocates it. Use for loaders + the
/// procedural-primitive path.
#[derive(Debug, Clone)]
pub struct NewMesh {
    pub vertices: Vec<f32>,
    pub normals: Vec<f32>,
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
    pub parent: Option<ObjectId>,
    /// See [`SceneObject::group_id`]. Loaders populate; most
    /// procedural / user-add call sites pass `None`.
    pub group_id: Option<u32>,
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
            group_id: None,
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
/// **not** here — they live scene-wide on [`SceneState`] so a
/// move-between-plates op doesn't have to copy mesh
/// buffers, and ids stay unique across plates.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlateSceneState {
    pub objects: HashMap<ObjectId, SceneObject>,
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
    /// User-facing names for groups (`group_id` → display name). A group
    /// is a set of objects sharing a `SceneObject::group_id` — the same
    /// mechanism the 3MF loader uses for multi-volume objects, now also
    /// driven by user grouping. The grouping itself persists as the
    /// objects' `group_id`; this map carries the label. Empty for
    /// groups without an explicit name (the UI shows a default).
    #[serde(default)]
    pub group_names: HashMap<u32, String>,
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

/// Return (min_z, max_z) of the corners after applying `xform`.
pub(crate) fn z_extent(corners: &[Vec3; 8], xform: &glam::Mat4) -> (f32, f32) {
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
