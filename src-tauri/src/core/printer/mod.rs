//! Printer drivers.
//!
//! Each driver owns its send path end-to-end, including any wrapping,
//! transformation, or metadata injection the target printer requires.
//! The slicer produces canonical G-code via libslic3r; the driver
//! decides what to do with it before transmission. Adding a future
//! printer is a self-contained exercise — write the driver, declare
//! its capabilities, no shared-code changes (PRD §8.2 note).

pub mod bambu;
pub mod instance;
pub mod instance_library;
pub mod profile;
pub mod registry;
pub mod snapmaker;

pub use instance::{
    BedRef, ConnectionInfo, ExtruderState, NozzleMaterial, NozzleSku, PrinterInstance,
    SlotBinding,
};
pub use instance_library::{
    bundled_instances, lookup_instance, BAMBI_ID, SNAPPY_ID,
};
pub use profile::{BoundingBox, PrinterProfile, Toolhead};
pub use registry::{bundled_catalog, default_binding, lookup, CatalogEntry};

/// Tauri command: list all bundled [`PrinterInstance`]s (PR-S-3). The
/// frontend printer picker will switch to this once instance-aware
/// editing lands (PR-S-5). Until then both surfaces coexist:
/// `bundled_catalog()` for the existing PrinterProfile-based picker,
/// `printer_instance_list()` for the instance-aware path.
#[tauri::command]
pub fn printer_instance_list() -> Vec<PrinterInstance> {
    bundled_instances()
}
