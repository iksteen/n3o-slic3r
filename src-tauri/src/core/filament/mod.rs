//! Filament profile library and printer-state-to-profile resolution.
//!
//! Reads live filament state from each connected printer (AMS lite
//! via MQTT on Bambu; per-toolhead loaded filament via HTTP on U1),
//! resolves model material → physical slot bindings per (plate,
//! printer), detects mismatches (material family, temperature range,
//! color), and emits the right sync-on-send metadata for each printer
//! driver.
//!
//! Owns FR-FS-1 through FR-FS-14 (PRD §6.8). Live sync + driver wiring
//! land in Phase 7. Phase 1 ships only the declarative
//! `FilamentProfile` descriptor the cascade resolver reads via
//! `Context::predicate_value`.

pub mod profile;
pub mod registry;

pub use profile::FilamentProfile;
pub use registry::{bundled_catalog, lookup};
