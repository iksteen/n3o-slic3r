//! Project model.
//!
//! Owns plate/printer binding, plate-level metadata (cycle counts,
//! composition order), material → slot bindings per (plate, printer),
//! and project persistence to/from .3mf with our own namespace
//! extensions.
//!
//! Owns FR-MP-1 through FR-MP-8 (PRD §6.2).
//!
//! Module layout:
//!
//! - **`context`**: the slice-time `SlicingContext` the cascade
//!   resolver consumes via its `Context` trait — composite of
//!   active printer + plate + per-slot filaments. Built per slice
//!   call from per-plate state; not stored.
//!
//! - **`metadata`**: `PlateMetadata` carrying cycle count +
//!   composition order. PlateCycler-relevant.
//!
//! - **`model`**: root `Project` + `Plate` + `PlateId`. Each
//!   `Plate` composes `core::scene::state::PlateSceneState` for
//!   its scene contents. The serializable shape `.3mf` save/load
//!   round-trips.

pub mod autosave;
pub mod commands;
pub mod context;
pub mod format;
pub mod metadata;
pub mod model;
pub mod mutation;

pub use context::SlicingContext;
pub use metadata::PlateMetadata;
pub use crate::core::scene::state::PlateSceneState;
pub use model::{Plate, PlateId, Project, ProjectMutError};
pub use mutation::PLATE_NAME_MAX;
