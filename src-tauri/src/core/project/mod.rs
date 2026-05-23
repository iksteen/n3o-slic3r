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
//! - **`context`** (PR-1-7): the slice-time `SlicingContext` the
//!   cascade resolver consumes via its `Context` trait — composite
//!   of active printer + plate + per-slot filaments. Built per
//!   slice call from per-plate state; not stored.
//!
//! - **`binding`** (PR-5-1): typed `PrinterBinding` (plate ←→
//!   printer + build plate) and `MaterialBinding` (model material
//!   index → physical slot → filament profile).
//!
//! - **`metadata`** (PR-5-1): `PlateMetadata` carrying cycle count
//!   + composition order. PlateCycler-relevant.
//!
//! - **`model`** (PR-5-1): root `Project` + `Plate` + `PlateId` +
//!   `PlateSceneState` (stubbed pending PR-5-2). The serializable
//!   shape PR-5-8's `.3mf` save/load round-trips.

pub mod binding;
pub mod context;
pub mod metadata;
pub mod model;

pub use binding::{MaterialBinding, PrinterBinding};
pub use context::SlicingContext;
pub use metadata::{PlateMetadata, CYCLE_COUNT_MAX, CYCLE_COUNT_MIN};
pub use model::{Plate, PlateId, PlateSceneState, Project, ProjectMutError};
