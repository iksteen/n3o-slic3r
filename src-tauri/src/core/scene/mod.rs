//! Renderer-agnostic 3D scene state.
//!
//! Authoritative model for the 3D viewport: mesh registry, per-object
//! transforms and metadata, hierarchy, selection state, and
//! exclusion-zone data per the active plate's printer. (Transform mode
//! is renderer-local; camera and gizmo-pivot scene state were removed
//! as dormant view-state — see PRD §9.2.)
//! The frontend renderer (Three.js for MVP, possibly wgpu later) is a
//! read-only consumer that reflects state changes via Tauri events;
//! all mutations enter through commands defined here.
//!
//! This module exists so the renderer is swappable without rewriting
//! state management — see PRD FR-3D-7 and AD-8. The renderer-vs-state
//! ownership boundary is load-bearing; resist the urge to let the
//! frontend hold authoritative state even for "just this one case."
//!
//! Performance contract: state ops ≤5 ms p99 on 1000-object scenes
//! (PRD AD-8). Validated by `src-tauri/tests/scene_state_perf.rs`.
//!
//! ## Eventual relocation
//!
//! `Mesh` / `Transform` and the loader-side utilities (`LoadError`,
//! `compute_bounding_box`, `compute_vertex_normals`) are general
//! geometry types — `core/threemf` already imports them upward.
//! Once a third consumer appears (likely the Phase 6 preview's
//! mesh-handle plumbing), extract them into a sibling
//! `core/geometry/` module so threemf doesn't have to reach into
//! scene. Today the coupling is small enough that the move would be
//! cosmetic; documented here so future-us doesn't forget.

pub mod align;
pub mod arrange;
pub mod bed;
pub mod build_plate;
pub mod commands;
pub mod events;
pub mod loaders;
pub mod primitives;
pub mod state;
pub mod transform;

pub use bed::{bed_for_printer, object_out_of_bounds, BedMesh, BoundsAxis, OutOfBoundsReason};
pub use build_plate::BuildPlate;
pub use events::{MirrorAxis, SceneEvent, SceneOpError, SelectMode};
pub use state::{
    ActivePlate, ExclusionZone, Mesh, MeshId, MeshProvenance, ObjectId, PlateSceneState,
    SceneObject,
};
pub use transform::Transform;
