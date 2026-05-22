//! Project model.
//!
//! Owns plate/printer binding, plate-level metadata (cycle counts,
//! composition order), material → slot bindings per (plate, printer),
//! and project persistence to/from .3mf with our own namespace
//! extensions.
//!
//! Owns FR-MP-1 through FR-MP-8 (PRD §6.2). Implementation lands in
//! Phase 5.
