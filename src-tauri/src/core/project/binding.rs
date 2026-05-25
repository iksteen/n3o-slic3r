//! Per-plate printer binding (PR-5-1).
//!
//! [`PrinterBinding`] names the printer a plate is assigned to.
//! Cascade composition + slice loading read this to find the bound
//! [`PrinterInstance`](crate::core::printer::PrinterInstance); the
//! instance owns the currently-loaded bed, slot bindings, and per-
//! instance overrides.
//!
//! Material/slot bindings used to live here too but moved to
//! [`PrinterInstance.extruders[].slots[]`](crate::core::printer::instance)
//! in PR-S-5c — slots are properties of physical printers, not of
//! plates. The build plate followed the same path in a later refactor:
//! the bed is a property of the physical printer (which one is
//! currently installed), not of the plate's bind state. The slicer
//! composer reads it off the instance via `instance.bed.identity`.
//!
//! Field names serialize verbatim to the project `.3mf`'s
//! `Metadata/n3o_project.json` (PR-5-8). Changing them is a
//! format-version bump.

use serde::{Deserialize, Serialize};

/// Which printer a [`Plate`](super::model::Plate) is assigned to.
/// Cascade resolution + slice loading read this and the plate's
/// `printer_instance_id` to build the per-plate
/// [`SlicingContext`](super::SlicingContext).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrinterBinding {
    /// Profile identity matching the bundled printer catalog. Cascade-
    /// side printer predicates (`when.printer.model = "Bambu A1 mini"`)
    /// resolve against the loaded profile's `model` field, not this
    /// identity string — but the identity is what survives save/load
    /// so users editing profiles after save don't lose the binding.
    pub printer_identity: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn printer_binding_serde_round_trips() {
        let pb = PrinterBinding {
            printer_identity: "bambu-lab-a1-mini".into(),
        };
        let json = serde_json::to_string(&pb).unwrap();
        let parsed: PrinterBinding = serde_json::from_str(&json).unwrap();
        assert_eq!(pb, parsed);
    }
}
