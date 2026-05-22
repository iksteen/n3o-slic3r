//! Translation adapter — logical settings ↔ libslic3r `DynamicPrintConfig`.
//!
//! Converts the cascade resolver's output into the flat key-value config
//! libslic3r consumes. Handles identity mappings (most settings) and
//! dimensional expansion (settings libslic3r split across hardcoded
//! dimensions — bed temperature × 6 plate types, retraction across nozzle
//! states, etc.).
//!
//! Owns FR-CAS-14 through FR-CAS-17 (PRD §6.1). Also the home for
//! libslic3r's dispatch quirks (`curr_bed_type`, `wipe_tower`,
//! `filament_map` / `nozzle_volume_type` / `wall_filament` normalization).
//! See `docs/libslic3r-workarounds.md` for the current set of quirks the
//! shim already compensates for; this module inherits the contract.
//!
//! Submodules:
//! - **`manifest`** — drop list + typo remap from Phase 0.5 findings.
//! - **`adapter`** — `adapt()` / `adapt_with_overrides()` that build a
//!   `slic3r_ffi::Config` from a `Resolved` (or `ResolvedOverrides`).

pub mod adapter;
pub mod manifest;

pub use adapter::{adapt, adapt_with_overrides, AdaptError, AdaptEvent, AdaptResult};
pub use manifest::Manifest;
