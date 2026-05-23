//! Renderer-agnostic 3D scene state.
//!
//! Authoritative model for the 3D viewport: mesh registry, per-object
//! transforms and metadata, hierarchy, selection state, gizmo state,
//! camera state, exclusion-zone data per the active plate's printer.
//! The frontend renderer (Three.js for MVP, possibly wgpu later) is a
//! read-only consumer that reflects state changes via Tauri events;
//! all mutations enter through commands defined here.
//!
//! This module exists so the renderer is swappable without rewriting
//! state management — see PRD FR-3D-7 and AD-8. The renderer-vs-state
//! ownership boundary is load-bearing; resist the urge to let the
//! frontend hold authoritative state even for "just this one case."
//!
//! Implementation lands in Phase 2. Performance contract: state ops
//! ≤5ms p99 on 1000-object scenes (PRD AD-8).
//!
//! Phase 1 ships only the `BuildPlate` descriptor here — the cascade
//! adapter (PR-1-6) needs it for `curr_bed_type`, and the Phase 2
//! renderer will extend it with mesh + adhesion + visuals.

pub mod arrange;
pub mod bed;
pub mod build_plate;
pub mod commands;
pub mod events;
pub mod library;
pub mod loaders;
pub mod primitives;
pub mod state;
pub mod transform;

pub use bed::{bed_for_printer, object_out_of_bounds, BedMesh, BoundsAxis, OutOfBoundsReason};
pub use build_plate::{BuildPlate, SurfaceKind};
pub use events::{MirrorAxis, SceneEvent, SceneOpError, SelectMode};
pub use state::{
    ActivePlate, CameraState, ExclusionZone, GizmoMode, GizmoState, Mesh, MeshId,
    MeshProvenance, ObjectId, ProjectionMode, SceneObject, SceneState,
};
pub use transform::Transform;
