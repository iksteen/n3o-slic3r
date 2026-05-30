//! `SlicingContext` — the cascade resolver's context object.
//!
//! Holds the *active* printer + build plate + per-slot filament
//! profiles for the project's current slice. Implements
//! [`crate::core::cascade::Context`] so the resolver can read
//! `printer.model`, `filament.type`, `plate.type`, etc. as dotted
//! predicate values.
//!
//! Project state proper (per-plate cycle counts, multi-plate
//! composition, .3mf persistence) is Phase 5 / FR-MP-* work; this
//! file ships only the slice-time context Phase 1's resolver needs.

use crate::core::cascade::Context;
use crate::core::filament::profile::FilamentProfile;
use crate::core::printer::profile::PrinterProfile;
use crate::core::scene::build_plate::BuildPlate;
use std::sync::Arc;

/// A snapshot of the project state the resolver needs.
///
/// Cloning is `Arc`-cheap — references are shared, not deep-copied —
/// so the resolver can hold a borrow for a single resolve call
/// without worrying about lifetime gymnastics across the Tauri
/// command boundary.
#[derive(Debug, Clone)]
pub struct SlicingContext {
    pub printer: Arc<PrinterProfile>,
    pub plate: Arc<BuildPlate>,
    pub filaments: Vec<Arc<FilamentProfile>>,
    /// Which slot's filament drives the `filament.*` predicates.
    /// Multi-color models slice once per active slot; production
    /// callers iterate over the active material map and rebuild
    /// `SlicingContext::active_slot` between calls.
    pub active_slot: usize,
}

impl SlicingContext {
    /// Convenience constructor for the canonical single-filament
    /// slice (slot 0 active, all other slots populated).
    pub fn new(
        printer: Arc<PrinterProfile>,
        plate: Arc<BuildPlate>,
        filaments: Vec<Arc<FilamentProfile>>,
    ) -> Self {
        Self {
            printer,
            plate,
            filaments,
            active_slot: 0,
        }
    }

    pub fn active_filament(&self) -> Option<&FilamentProfile> {
        self.filaments.get(self.active_slot).map(Arc::as_ref)
    }
}

impl Context for SlicingContext {
    fn predicate_value(&self, key: &str) -> Option<&str> {
        match key {
            "printer.model" => Some(&self.printer.model),
            "plate.type" => Some(&self.plate.identity),
            "filament.type" => self.active_filament().map(|f| f.base_type.as_str()),
            "filament.name" => self.active_filament().map(|f| f.identity.as_str()),
            "filament.vendor" => self.active_filament().and_then(|f| f.vendor.as_deref()),
            "filament.color" => self.active_filament().and_then(|f| f.color.as_deref()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a1_mini() -> Arc<PrinterProfile> {
        use crate::core::printer::profile::{BoundingBox, Toolhead};
        Arc::new(PrinterProfile {
            model: "Bambu Lab A1 mini".into(),
            supported_build_plates: vec!["Textured PEI Plate".into()],
            toolheads: vec![Toolhead {
                default_nozzle_diameter: "0.4".to_string(),
                hotend_type: "stainless_steel".into(),
                max_temp: 300.0,
            }],
            build_volume: BoundingBox::default(),
            exclusion_zones: vec![],
            ..Default::default()
        })
    }

    fn textured_pei() -> Arc<BuildPlate> {
        Arc::new(BuildPlate {
            identity: "Textured PEI Plate".into(),
            libslic3r_curr_bed_type: "Textured PEI Plate".into(),
        })
    }

    fn pla(name: &str, color: &str) -> Arc<FilamentProfile> {
        Arc::new(FilamentProfile {
            identity: name.into(),
            base_type: "PLA".into(),
            vendor: Some("Bambu Lab".into()),
            color: Some(color.into()),
        })
    }

    fn petg(name: &str) -> Arc<FilamentProfile> {
        Arc::new(FilamentProfile {
            identity: name.into(),
            base_type: "PETG".into(),
            vendor: None,
            color: None,
        })
    }

    #[test]
    fn canonical_a1_pla_pei_predicates() {
        let ctx = SlicingContext::new(a1_mini(), textured_pei(), vec![pla("PLA Cyan", "#0A2989")]);
        assert_eq!(
            ctx.predicate_value("printer.model"),
            Some("Bambu Lab A1 mini")
        );
        assert_eq!(
            ctx.predicate_value("plate.type"),
            Some("Textured PEI Plate")
        );
        assert_eq!(ctx.predicate_value("filament.type"), Some("PLA"));
        assert_eq!(ctx.predicate_value("filament.name"), Some("PLA Cyan"));
        assert_eq!(ctx.predicate_value("filament.color"), Some("#0A2989"));
        assert_eq!(ctx.predicate_value("filament.vendor"), Some("Bambu Lab"));
        assert!(ctx.predicate_value("nonexistent").is_none());
    }

    #[test]
    fn active_slot_swap_changes_filament_predicates() {
        let mut ctx = SlicingContext::new(
            a1_mini(),
            textured_pei(),
            vec![pla("PLA", "#FFFFFF"), petg("PETG")],
        );
        assert_eq!(ctx.predicate_value("filament.type"), Some("PLA"));
        ctx.active_slot = 1;
        assert_eq!(ctx.predicate_value("filament.type"), Some("PETG"));
        assert_eq!(ctx.predicate_value("filament.name"), Some("PETG"));
        // Slot 1's PETG has no vendor — predicate returns None.
        assert!(ctx.predicate_value("filament.vendor").is_none());
    }

    #[test]
    fn active_slot_out_of_range_returns_none() {
        let ctx = SlicingContext {
            printer: a1_mini(),
            plate: textured_pei(),
            filaments: vec![pla("PLA", "#FFF")],
            active_slot: 5,
        };
        assert!(ctx.predicate_value("filament.type").is_none());
    }

    #[test]
    fn slicing_context_resolves_via_resolver() {
        use crate::core::cascade::{loader::parse_cascade_str, resolver::resolve, Cascade};
        use std::path::Path;

        let cascade = Cascade {
            rules: parse_cascade_str(
                "\
[[rule]]
when.filament.type = \"PLA\"
when.plate.type = \"Textured PEI Plate\"
set.bed_temp = 65
",
                Path::new("test.toml"),
            )
            .unwrap(),
        };
        let ctx = SlicingContext::new(a1_mini(), textured_pei(), vec![pla("PLA Cyan", "#0A2989")]);
        let resolved = resolve(&cascade, &ctx);
        assert_eq!(
            resolved.get("bed_temp").map(|v| v.value.as_str()),
            Some("65"),
            "resolver picks up SlicingContext's predicate values"
        );
    }
}
