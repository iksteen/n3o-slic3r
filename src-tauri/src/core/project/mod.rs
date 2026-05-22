//! Project model.
//!
//! Owns plate/printer binding, plate-level metadata (cycle counts,
//! composition order), material → slot bindings per (plate, printer),
//! and project persistence to/from .3mf with our own namespace
//! extensions.
//!
//! Owns FR-MP-1 through FR-MP-8 (PRD §6.2). Multi-printer + multi-plate
//! features land in Phase 5. Phase 1 ships only the slice-time
//! `SlicingContext` the cascade resolver consumes via its `Context`
//! trait — composite of active printer + plate + per-slot filaments.

pub mod context;

pub use context::SlicingContext;
