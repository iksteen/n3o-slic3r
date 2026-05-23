//! Per-plate bindings (PR-5-1).
//!
//! [`PrinterBinding`] names the printer + build plate a plate is
//! assigned to. [`MaterialBinding`] maps a model material index
//! (the per-volume `extruder` field libslic3r consumes) to a physical
//! slot on the bound printer.
//!
//! Both serialize verbatim to the project `.3mf`'s
//! `Metadata/n3o_project.json` (PR-5-8). Field names are wire-stable
//! — changing them is a format-version bump.

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

/// Map a model material index to a physical printer slot +
/// filament profile (PR-5-6 / FR-MP-8).
///
/// Stored per-plate so a multi-printer project's plate-1 binding
/// can route material 1 → slot 2 on the A1 mini while plate-2
/// routes the same model material 1 → slot 1 on a U1 with a
/// different physical loadout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterialBinding {
    /// 1-based model material index — matches the per-volume
    /// `extruder` metadata libslic3r's 3MF importer surfaces from
    /// `<metadata key="extruder" value="N"/>`.
    pub model_material: u8,

    /// 1-based physical slot on the bound printer, matching
    /// libslic3r's `filament_map` 1-based convention. PR-5-6
    /// validates `1 <= physical_slot <= printer.slot_count` at
    /// cascade resolution + before slice.
    pub physical_slot: u8,

    /// Filament profile identity loaded into the bound slot.
    /// Matches `profiles/filaments/<identity>.toml`. The cascade
    /// resolves against the profile's `base_type` for filament
    /// predicates.
    pub filament_identity: String,
}

impl MaterialBinding {
    /// Both indices are 1-based; reject zero (libslic3r's "use
    /// default" sentinel doesn't make sense as an authored
    /// binding).
    pub fn validate(&self) -> Result<(), String> {
        if self.model_material == 0 {
            return Err("model_material must be >= 1 (0 is libslic3r's dontcare sentinel)".into());
        }
        if self.physical_slot == 0 {
            return Err("physical_slot must be >= 1".into());
        }
        if self.filament_identity.is_empty() {
            return Err("filament_identity must not be empty".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn material_binding_rejects_zero_indices() {
        let invalid = MaterialBinding {
            model_material: 0,
            physical_slot: 1,
            filament_identity: "Generic PLA".into(),
        };
        assert!(invalid.validate().is_err());

        let invalid = MaterialBinding {
            model_material: 1,
            physical_slot: 0,
            filament_identity: "Generic PLA".into(),
        };
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn material_binding_rejects_empty_filament() {
        let invalid = MaterialBinding {
            model_material: 1,
            physical_slot: 1,
            filament_identity: "".into(),
        };
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn material_binding_accepts_canonical_shape() {
        let valid = MaterialBinding {
            model_material: 2,
            physical_slot: 3,
            filament_identity: "Bambu PLA Basic".into(),
        };
        assert!(valid.validate().is_ok());
    }

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

    #[test]
    fn material_binding_serde_round_trips() {
        let mb = MaterialBinding {
            model_material: 4,
            physical_slot: 4,
            filament_identity: "Generic PLA".into(),
        };
        let json = serde_json::to_string(&mb).unwrap();
        let parsed: MaterialBinding = serde_json::from_str(&json).unwrap();
        assert_eq!(mb, parsed);
    }
}
