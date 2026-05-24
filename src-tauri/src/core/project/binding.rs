//! Per-plate printer binding (PR-5-1).
//!
//! [`PrinterBinding`] names the printer + build plate a plate is
//! assigned to. Cascade composition + slice loading both read this
//! to build the per-plate [`SlicingContext`](super::SlicingContext).
//!
//! Material/slot bindings used to live here too but moved to
//! [`PrinterInstance.extruders[].slots[]`](crate::core::printer::instance)
//! in PR-S-5c — slots are properties of physical printers, not of
//! plates.
//!
//! Field names serialize verbatim to the project `.3mf`'s
//! `Metadata/n3o_project.json` (PR-5-8). Changing them is a
//! format-version bump.

use serde::{Deserialize, Serialize};

/// Which printer + build plate a [`Plate`](super::model::Plate) is
/// assigned to. Cascade resolution + slice loading both read this
/// to build the per-plate [`SlicingContext`](super::SlicingContext).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrinterBinding {
    /// Profile identity matching
    /// `profiles/printers/<printer_identity>.toml`. Cascade-side
    /// printer predicates (`when.printer.model = "Bambu A1 mini"`)
    /// resolve against the loaded profile's `model` field, not this
    /// identity string — but the identity is what survives save/load
    /// so users editing profiles after save don't lose the binding.
    pub printer_identity: String,

    /// Selected build plate within the printer's
    /// `supported_build_plates`. Mirrors the
    /// [`BuildPlate`](crate::core::scene::build_plate::BuildPlate)
    /// identity field. Changing the printer typically requires
    /// re-selecting the plate (PR-5-4 surfaces the warning).
    pub build_plate_identity: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn printer_binding_serde_round_trips() {
        let pb = PrinterBinding {
            printer_identity: "bambu-a1-mini".into(),
            build_plate_identity: "Textured PEI".into(),
        };
        let json = serde_json::to_string(&pb).unwrap();
        let parsed: PrinterBinding = serde_json::from_str(&json).unwrap();
        assert_eq!(pb, parsed);
    }
}
