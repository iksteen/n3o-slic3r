//! Capability predicates for printer-aware option visibility.
//!
//! Each predicate names a printer-side capability that gates whether
//! a given libslic3r option is meaningful for that printer. The
//! settings panel consumes the evaluation result to hide options
//! when the active printer doesn't satisfy the predicate.
//!
//! ## Scoping decision
//!
//! Predicates are evaluated against [`PrinterProfile`] alone — no
//! plate, no filament, no scene state. That keeps the
//! `slicer_options_for_printer` Tauri command cheap and idempotent:
//! a printer-switch invalidates the cached result; nothing else
//! does.
//!
//! ## Vocabulary stability
//!
//! The set of predicates is small and grows with audit. The initial
//! set covers what FR-UI-7's exit criterion gates on (A1 mini hides
//! toolchange options; U1 hides purge volumes matrix). Each new
//! predicate added later should:
//!
//! - Map to a discoverable property of `PrinterProfile` (or push a
//!   new field onto it).
//! - Cover a non-empty option set in libslic3r's catalog — adding a
//!   predicate that nothing references is dead weight.
//! - Have a tested A1 mini + Snapmaker U1 + synthetic-toolchanger
//!   case in the printer-aware-view test suite.

use serde::{Deserialize, Serialize};

use crate::core::printer::profile::PrinterProfile;

/// Typed predicates the Settings UI consults to decide whether to
/// hide a row for the active printer.
///
/// Variants are mutually exclusive per option — `capability_for_key`
/// returns `None` for the vast majority of options (those that make
/// sense everywhere). When two predicates would both apply to a
/// single option in the future, encode the conjunction as a new
/// dedicated variant rather than reaching for a `Vec<Predicate>` —
/// the discrete enum keeps wire-format + frontend handling simple.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum CapabilityPredicate {
    /// Filament-map / multi-material options. Hidden when the
    /// printer is a single-material rig. Bambu A1 mini (4 AMS slots,
    /// 1 toolhead) satisfies this; a single-slot Voron does not.
    RequiresMultiSlot,

    /// Toolchanger-only options — e.g. extruder clearance geometry,
    /// per-toolhead docking. Hidden on AMS-style filament-swap
    /// printers like the A1 mini where one toolhead serves all
    /// slots.
    RequiresToolchanger,

    /// Prime/purge tower geometry options. Toolchangers (U1, XL)
    /// physically swap the head and don't purge between filaments;
    /// AMS-style printers do, and need the prime tower. Hidden on
    /// toolchangers.
    RequiresPurgeTower,

    /// Bambu-vendor-only options — BBS-namespaced metadata,
    /// machine-specific G-code macros that only Bambu's firmware
    /// honors. Hidden on non-Bambu printers.
    RequiresBblPrinter,
}

impl CapabilityPredicate {
    /// True when the printer satisfies this predicate (i.e. options
    /// gated on it should be shown). False means hide.
    pub fn satisfied_by(self, printer: &PrinterProfile) -> bool {
        match self {
            // Multi-material: either AMS-fed (single toolhead with
            // an AMS host attached) or toolchanger (multiple
            // toolheads). Single-material printers fail both clauses.
            Self::RequiresMultiSlot => printer.has_multiple_slots(),

            // Toolchanger heuristic: more than one physical toolhead.
            // AMS-style (toolheads.len() == 1, ams_max > 0) does NOT
            // satisfy this — its multi-material is filament swap,
            // not head swap.
            Self::RequiresToolchanger => printer.toolheads.len() > 1,

            // Purge-tower heuristic: AMS-style printers, where the
            // single toolhead has to flush each color through it.
            // Toolchangers skip purging because they swap heads;
            // single-material printers don't purge because they only
            // print one filament.
            Self::RequiresPurgeTower => printer.toolheads.len() == 1 && printer.ams_max > 0,

            Self::RequiresBblPrinter => printer.model.starts_with("Bambu"),
        }
    }
}

/// Return the capability predicate (if any) that gates this option.
///
/// `None` means "always meaningful" — most libslic3r options fall
/// through. The mapping table is hand-curated from
/// `external/OrcaSlicer/src/libslic3r/PrintConfig.cpp`'s
/// `ConfigOptionDef::condition` predicates plus per-printer-class
/// audit; extend as new options surface that the UI should gate.
pub fn capability_for_key(key: &str) -> Option<CapabilityPredicate> {
    match key {
        // Multi-material filament mapping. Meaningful only when more
        // than one filament slot exists.
        "filament_map" | "filament_map_mode" | "filament_maps" => {
            Some(CapabilityPredicate::RequiresMultiSlot)
        }

        // Purge-tower geometry + flush volumes. AMS-style only —
        // toolchangers don't purge.
        "enable_prime_tower"
        | "prime_tower_enable_framework"
        | "prime_tower_width"
        | "prime_tower_brim_width"
        | "wipe_tower_x"
        | "wipe_tower_y"
        | "wipe_tower_rotation_angle"
        | "flush_volumes_matrix"
        | "flush_volumes_vector"
        | "purge_volumes_matrix"
        | "flush_into_infill"
        | "flush_into_objects"
        | "flush_into_support" => Some(CapabilityPredicate::RequiresPurgeTower),

        // Toolchanger geometry — head docking clearance, per-head
        // load/unload macros. Hidden on AMS-style printers.
        "extruder_clearance_height_to_rod"
        | "extruder_clearance_height_to_lid"
        | "extruder_clearance_radius"
        | "machine_load_filament_time"
        | "machine_unload_filament_time" => Some(CapabilityPredicate::RequiresToolchanger),

        // Bambu-vendor-only. The catalog grows as we discover more
        // BBS-namespaced keys; for the MVP we gate the ones that
        // appear in the bundled A1 mini cascade.
        "best_object_pos" | "scan_first_layer" | "ams_stash_speed" => {
            Some(CapabilityPredicate::RequiresBblPrinter)
        }

        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::printer::profile::{BoundingBox, Toolhead};

    fn a1_mini() -> PrinterProfile {
        PrinterProfile {
            model: "Bambu Lab A1 mini".into(),
            // AMS-fed: one toolhead, one or more AMS units. The
            // capability predicates read multi-material status off
            // `ams_max`, so it must be set explicitly for tests that
            // depend on RequiresMultiSlot/RequiresPurgeTower.
            ams_max: 1,
            supported_build_plates: vec!["Textured PEI".into()],
            toolheads: vec![Toolhead {
                default_nozzle_diameter: "0.4".to_string(),
                hotend_type: "stainless_steel".into(),
                max_temp: 300.0,
            }],
            build_volume: BoundingBox {
                min: [0.0, 0.0, 0.0],
                max: [180.0, 180.0, 180.0],
            },
            exclusion_zones: vec![],
            ..Default::default()
        }
    }

    fn snapmaker_u1() -> PrinterProfile {
        PrinterProfile {
            model: "Snapmaker U1".into(),
            supported_build_plates: vec!["Textured PEI".into()],
            toolheads: (0..4)
                .map(|_i| Toolhead {
                    default_nozzle_diameter: "0.4".to_string(),
                    hotend_type: "stainless_steel".into(),
                    max_temp: 300.0,
                })
                .collect(),
            build_volume: BoundingBox {
                min: [0.0, 0.0, 0.0],
                max: [220.0, 220.0, 220.0],
            },
            exclusion_zones: vec![],
            ..Default::default()
        }
    }

    fn single_material_voron() -> PrinterProfile {
        PrinterProfile {
            model: "Voron 2.4 350".into(),
            supported_build_plates: vec!["Textured PEI".into()],
            toolheads: vec![Toolhead {
                default_nozzle_diameter: "0.4".to_string(),
                hotend_type: "stainless_steel".into(),
                max_temp: 300.0,
            }],
            build_volume: BoundingBox {
                min: [0.0, 0.0, 0.0],
                max: [350.0, 350.0, 350.0],
            },
            exclusion_zones: vec![],
            ..Default::default()
        }
    }

    #[test]
    fn a1_mini_shows_purge_tower_hides_toolchanger() {
        let p = a1_mini();
        assert!(CapabilityPredicate::RequiresMultiSlot.satisfied_by(&p));
        assert!(CapabilityPredicate::RequiresPurgeTower.satisfied_by(&p));
        assert!(!CapabilityPredicate::RequiresToolchanger.satisfied_by(&p));
        assert!(CapabilityPredicate::RequiresBblPrinter.satisfied_by(&p));
    }

    #[test]
    fn u1_shows_toolchanger_hides_purge_tower() {
        let p = snapmaker_u1();
        assert!(CapabilityPredicate::RequiresMultiSlot.satisfied_by(&p));
        assert!(!CapabilityPredicate::RequiresPurgeTower.satisfied_by(&p));
        assert!(CapabilityPredicate::RequiresToolchanger.satisfied_by(&p));
        assert!(!CapabilityPredicate::RequiresBblPrinter.satisfied_by(&p));
    }

    #[test]
    fn single_material_printer_hides_multi_slot_options() {
        let p = single_material_voron();
        assert!(!CapabilityPredicate::RequiresMultiSlot.satisfied_by(&p));
        assert!(!CapabilityPredicate::RequiresPurgeTower.satisfied_by(&p));
        assert!(!CapabilityPredicate::RequiresToolchanger.satisfied_by(&p));
        assert!(!CapabilityPredicate::RequiresBblPrinter.satisfied_by(&p));
    }

    #[test]
    fn capability_table_picks_up_canonical_keys() {
        assert_eq!(
            capability_for_key("filament_map_mode"),
            Some(CapabilityPredicate::RequiresMultiSlot),
        );
        assert_eq!(
            capability_for_key("purge_volumes_matrix"),
            Some(CapabilityPredicate::RequiresPurgeTower),
        );
        assert_eq!(
            capability_for_key("extruder_clearance_radius"),
            Some(CapabilityPredicate::RequiresToolchanger),
        );
        assert_eq!(capability_for_key("chamber_temperature"), None);
        assert_eq!(capability_for_key("layer_height"), None);
    }

    #[test]
    fn a1_mini_hides_toolchanger_keys_via_table() {
        let p = a1_mini();
        // Toolchanger-only key should be hidden:
        let pred = capability_for_key("extruder_clearance_radius").unwrap();
        assert!(!pred.satisfied_by(&p));
        // Purge-tower key should show:
        let pred = capability_for_key("flush_volumes_matrix").unwrap();
        assert!(pred.satisfied_by(&p));
        // Unrelated key should be unconstrained:
        assert!(capability_for_key("layer_height").is_none());
    }
}
