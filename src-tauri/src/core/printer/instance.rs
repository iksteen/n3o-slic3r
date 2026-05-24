//! Printer instance — user-binding-state overlay on top of a vendor
//! [`PrinterProfile`] (PR-S-3).
//!
//! A `PrinterInstance` represents a physical printer the user owns. It
//! references a vendor profile by identity and carries the per-instance
//! state that doesn't live on the profile: connection info, the
//! currently-installed nozzle per extruder, slot → filament bindings,
//! the currently-loaded bed, and any per-instance config overrides
//! (printer-bucket only; filament/process overrides live elsewhere).
//!
//! MVP scope: instances are bundled fixtures (Bambi + Snappy) loaded at
//! startup. No persistence to user config yet; no UI to add or edit
//! instances. The picker just sees the two fixtures. Subsequent tickets
//! add the user-library file and the editor surfaces.
//!
//! See `docs/design/settings-model.md` §4 (Storage model — User library)
//! for the durable-form intent.

use serde::{Deserialize, Serialize};

/// One physical printer the user has access to.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrinterInstance {
    /// Stable identifier — UUID-like or short slug. Used as the
    /// project-side reference (`Plate.printer_instance_ref`).
    pub id: String,

    /// Display name surfaced in pickers and panels. User-editable
    /// future; for the bundled fixtures it's the friendly name
    /// ("Bambi", "Snappy").
    pub display_name: String,

    /// Identity of the vendor [`PrinterProfile`] this instance is
    /// built on. Looked up in [`crate::core::printer::bundled_catalog`].
    pub vendor_profile_ref: String,

    /// Slug of the printer-bucket vendor fragment this instance loads
    /// at slice time. Resolved against
    /// [`crate::core::profile_library::load_fragment`] with
    /// [`Bucket::Printer`](crate::core::profile_library::Bucket::Printer).
    /// E.g. `"bambu-lab-a1-mini-0.4-nozzle"`.
    ///
    /// Carried as a separate field (rather than derived from
    /// `vendor_profile_ref` + nozzle) because the printer fragment is
    /// nozzle-variant-specific upstream — `vendor_profile_ref` names
    /// the printer model, `printer_fragment_slug` names the specific
    /// (model, nozzle-variant) tuple.
    pub printer_fragment_slug: String,

    /// Default filament fragment to use for slots that haven't been
    /// bound to a specific filament yet. Resolved against
    /// [`Bucket::Filament`](crate::core::profile_library::Bucket::Filament).
    pub default_filament_fragment_slug: String,

    /// Default process fragment for plates that don't specify one.
    /// MVP: every plate uses this default; future process binding
    /// makes this overridable per plate.
    pub default_process_fragment_slug: String,

    /// Network connection details. `None` when the user hasn't
    /// configured connection yet — the instance still works for slicing,
    /// just not for sending to the printer.
    #[serde(default)]
    pub connection: Option<ConnectionInfo>,

    /// One entry per physical extruder (T0..T(N-1)). For shared-toolhead
    /// printers (A1 mini, X1C) the vector has length 1; for tool changers
    /// (U1, XL) the vector matches the toolhead count.
    pub extruders: Vec<ExtruderState>,

    /// Currently-loaded build plate. MVP: single value per instance.
    /// Post-MVP: `Vec<LoadedBed>` to support platecyclers.
    pub bed: BedRef,

    /// Per-instance printer-bucket overrides. Empty for MVP. The runtime
    /// cascade composer (PR-S-5) layers this on top of the vendor
    /// profile's printer cascade.
    #[serde(default)]
    pub config_overrides: std::collections::BTreeMap<String, String>,
}

/// Per-extruder state — currently-installed nozzle plus the filament
/// feeds (slots) that pull into this extruder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtruderState {
    /// What's physically screwed into this extruder right now. The user
    /// confirms swaps in the UI; there's no firmware sensor for
    /// installed-nozzle SKU on consumer printers.
    pub installed_nozzle: NozzleSku,

    /// Filament feeds this extruder pulls from. Length 1 for
    /// direct-feed extruders (U1 toolhead, A1 mini standalone),
    /// length N for AMS-fed extruders (slot 0 = external direct,
    /// slots 1..4 = AMS). Bindings may be `None` while the user
    /// hasn't picked a filament for that slot yet.
    pub slots: Vec<SlotBinding>,
}

/// Currently-installed nozzle on an extruder. SKU is the small
/// vocabulary of (diameter, material) the printer supports — the
/// printer profile's available nozzle catalog gates which SKUs are
/// pickable per extruder.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NozzleSku {
    pub diameter_mm: f32,
    pub material: NozzleMaterial,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NozzleMaterial {
    Brass,
    Hardened,
    Stainless,
    HighFlowHardened,
    HighFlowStainless,
}

/// Slot → filament binding. `None` = unbound (no filament picked).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotBinding {
    /// Identity string matching an entry in the user's filament
    /// library. `None` while no filament is bound.
    #[serde(default)]
    pub filament_identity: Option<String>,
}

/// Reference to a build plate currently installed on the printer.
/// MVP carries only the bed type identity; future extensions can
/// add per-bed conditioning state, wear tracking, etc.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BedRef {
    /// Bed type identity (e.g. `"Bambu Cool Plate SuperTack"`,
    /// `"Snapmaker Textured PEI"`). Must be in the vendor profile's
    /// `supported_build_plates` for the resolution to succeed.
    pub identity: String,
}

/// Network connection details for sending prints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionInfo {
    pub host: String,
    pub serial: String,
    pub access_code: String,
    /// True when the printer's "LAN Mode With Developer Tools Enabled"
    /// is on (Bambu-specific; bypasses RSA-SHA256 payload signing).
    /// Currently required for our Bambu driver to send.
    pub dev_mode: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_round_trips_json() {
        let inst = PrinterInstance {
            id: "test".into(),
            display_name: "Test".into(),
            vendor_profile_ref: "bambu-a1-mini".into(),
            printer_fragment_slug: "bambu-lab-a1-mini-0.4-nozzle".into(),
            default_filament_fragment_slug: "bambu-pla-basic-bbl-a1m".into(),
            default_process_fragment_slug: "0.20mm-standard-bbl-a1m".into(),
            connection: None,
            extruders: vec![ExtruderState {
                installed_nozzle: NozzleSku {
                    diameter_mm: 0.4,
                    material: NozzleMaterial::Stainless,
                },
                slots: vec![SlotBinding {
                    filament_identity: None,
                }],
            }],
            bed: BedRef {
                identity: "Bambu Cool Plate SuperTack".into(),
            },
            config_overrides: Default::default(),
        };
        let json = serde_json::to_string(&inst).expect("ser");
        let parsed: PrinterInstance = serde_json::from_str(&json).expect("de");
        assert_eq!(parsed.id, "test");
        assert_eq!(parsed.extruders.len(), 1);
        assert_eq!(parsed.extruders[0].installed_nozzle.diameter_mm, 0.4);
        assert_eq!(
            parsed.extruders[0].installed_nozzle.material,
            NozzleMaterial::Stainless,
        );
        assert_eq!(parsed.bed.identity, "Bambu Cool Plate SuperTack");
    }

    #[test]
    fn nozzle_material_serializes_snake_case() {
        let json = serde_json::to_string(&NozzleMaterial::HighFlowHardened).unwrap();
        assert_eq!(json, "\"high_flow_hardened\"");
    }
}
