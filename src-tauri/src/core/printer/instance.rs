//! Printer instance — user-binding-state overlay on top of a vendor
//! [`PrinterProfile`].
//!
//! A `PrinterInstance` represents a physical printer the user owns. It
//! references a vendor profile by identity and carries the per-instance
//! state that doesn't live on the profile: connection info, the
//! currently-installed nozzle per extruder, slot → filament bindings,
//! the currently-loaded bed, and any per-instance config overrides
//! (printer-bucket only; filament/process overrides live elsewhere).
//!
//! Production stores instances as TOML files in the user library at
//! `<config>/n3o-slic3r/printers/`. First launch starts empty; the
//! add-printer wizard writes the first instance and edits round-trip
//! through `instance_storage::persist`. Tests that don't wire a
//! storage root fall back to the bambi + snappy fixtures from
//! `instance_library`.
//!
//! See `docs/design/settings-model.md` §4 (Storage model — User library)
//! for the durable-form intent.

use serde::{Deserialize, Serialize};

/// Pointer into a [`PrinterInstance`]'s extruder/slot grid. Used by
/// [`Plate.material_to_slot`](crate::core::project::Plate) to record
/// which filament feed each model material routes to.
///
/// `(0, 0)` is "first extruder's first slot." Indices are 0-based;
/// the libslic3r-side 1-based filament index is derived by walking
/// the printer's flat slot list at slice time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SlotRef {
    pub extruder: u8,
    pub slot: u8,
}

/// One physical printer the user has access to.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrinterInstance {
    /// Stable identifier — UUID-like or short slug. Used as the
    /// project-side reference (`Plate.printer_instance_ref`).
    pub id: String,

    /// Display name surfaced in pickers and panels. User-editable.
    /// Test fixtures use the friendly nicknames "Bambi" / "Snappy".
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

    /// Currently selected quality (process) profile for this
    /// instance — the slug the Quality picker is bound to and the
    /// cascade composer feeds into the slicer. Written by user
    /// interaction (the picker) and by the nozzle-swap fallback
    /// (`set_extruder_nozzle_diameter` resets it to the new
    /// nozzle's `default_process_profile` when the current
    /// selection becomes incompatible).
    pub quality_profile: String,

    /// Network connection details. `None` when the user hasn't
    /// configured connection yet — the instance still works for slicing,
    /// just not for sending to the printer.
    #[serde(default)]
    pub connection: Option<ConnectionInfo>,

    /// One entry per physical extruder, 0-indexed. For shared-
    /// toolhead printers (A1 mini, X1C) the vector has length 1;
    /// for tool changers (U1, XL) the vector matches the toolhead
    /// count. The frontend renders these as 1-based display labels
    /// (`T1..TN`) when there's more than one.
    pub extruders: Vec<ExtruderState>,

    /// Currently-loaded build plate. MVP: single value per instance.
    /// Post-MVP: `Vec<LoadedBed>` to support platecyclers.
    pub bed: BedRef,

    /// Per-instance printer-bucket overrides. Empty for MVP. The runtime
    /// cascade composer layers this on top of the vendor
    /// profile's printer cascade.
    #[serde(default)]
    pub config_overrides: std::collections::BTreeMap<String, String>,
}

/// AMS slot topology: each installed AMS unit contributes exactly
/// this many `FeedKind::Ams` slots on the first extruder, and one
/// trailing `FeedKind::Direct` slot ("Ext") is always present. So an
/// AMS-style printer with `n` units has `n * AMS_SLOTS_PER_UNIT + 1`
/// slots. `apply_ams_units` and `create_instance` both build the
/// layout from this constant; the frontend mirror is `amsUnitsOf`.
pub const AMS_SLOTS_PER_UNIT: usize = 4;

/// Per-extruder state — currently-installed nozzle plus the filament
/// feeds (slots) that pull into this extruder.
///
/// Display labels (`T1`, `T2`, `AMS:1`, `Ext`, …) are *not* stored
/// here — the frontend derives them from the extruder position +
/// total extruder count + per-slot `feed`. A runtime topology
/// change (user attaches a second AMS unit, swaps a nozzle) needs
/// only update the structural fields; the labels re-derive on the
/// next render.
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
///
/// `diameter` is a string symbol ("0.4", "0.25", "0.4+0.6") —
/// never a number we arithmetic on. The cascade composer matches
/// it to a `nozzles/<diameter>.toml` filename by exact-string
/// lookup, and the Quality picker filters fragments by exact-set
/// membership against `[meta] available_for` entries.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NozzleSku {
    pub diameter: String,
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

/// Whether a slot pulls its filament from the printer's external
/// direct-feed or through an AMS-style swap unit.
///
/// The distinction drives the pre-slice gate: within one extruder the
/// user may NOT mix `Direct` and `Ams` slots in the same print —
/// Bambu firmware physically can't pull from both feed paths in one
/// job. (Multiple `Ams` slots on the same extruder are fine; the AMS
/// firmware swaps within.) Printers with no AMS at all (Snapmaker U1,
/// Prusa XL, A1 mini standalone) ship every slot as `Direct`; the
/// per-extruder constraint is trivially satisfied since they only
/// have one slot per extruder anyway.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FeedKind {
    Direct,
    Ams,
}

/// Slot → filament binding. `None` = unbound (no filament picked).
///
/// Display labels (`"AMS:1"`, `"Ext"`, etc.) are *not* stored here
/// — see [`ExtruderState`]'s note. The frontend derives them from
/// position + `feed`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotBinding {
    /// What feed path this slot uses. Drives the AMS-vs-Direct split
    /// in `ams_bindings_for_plate` / `ams_mapping_for_plate` (Direct
    /// slots emit the `{255, 0}` external-spool sentinel; AMS slots
    /// emit the 1-based AMS-feed index).
    #[serde(default = "default_feed_kind")]
    pub feed: FeedKind,

    /// Identity string matching an entry in the user's filament
    /// library. `None` while no filament is bound.
    #[serde(default)]
    pub filament_identity: Option<String>,

    /// User-assigned spool color as a CSS-style hex string
    /// (e.g. `"#ff8800"`). `None` = unassigned; the UI renders a
    /// neutral placeholder swatch in that case. Authoritative
    /// per-slot — a future driver-sync path (e.g. Bambu AMS readout)
    /// writes here, not into the filament profile.
    #[serde(default)]
    pub color: Option<String>,
}

fn default_feed_kind() -> FeedKind {
    FeedKind::Direct
}

/// Reference to a build plate currently installed on the printer.
/// MVP carries only the bed type identity; future extensions can
/// add per-bed conditioning state, wear tracking, etc.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BedRef {
    /// Bed type identity, matching libslic3r's `curr_bed_type` enum
    /// (e.g. `"Supertack Plate"`, `"Cool Plate"`, `"Textured PEI Plate"`).
    /// Must be in the vendor profile's `supported_build_plates` for
    /// the resolution to succeed.
    pub identity: String,
}

/// Network connection details for sending prints. Driver-tagged so
/// each kind only carries the fields it needs — Bambu needs an
/// 8-digit LAN access code; U1 needs a Moonraker port (usually 80).
///
/// Device serial is intentionally NOT persisted here: the drivers
/// resolve it at connect time (Bambu via mDNS, U1 via
/// `/machine/system_info`), and nothing in the UI ever authored it.
/// Keeping it out of the stored connection avoids a dead round-tripped
/// field and a stale-serial footgun. The runtime [`DriverConfig`] still
/// carries an optional serial for the probe/trust path.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConnectionInfo {
    Bambu { host: String, access_code: String },
    U1 { host: String, port: u16 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_round_trips_json() {
        let inst = PrinterInstance {
            id: "test".into(),
            display_name: "Test".into(),
            vendor_profile_ref: "bambu-lab-a1-mini".into(),
            printer_fragment_slug: "bambu-lab-a1-mini".into(),
            default_filament_fragment_slug: "bambu-pla-basic-bbl-a1m".into(),
            quality_profile: "0.20mm-standard".into(),
            connection: None,
            extruders: vec![ExtruderState {
                installed_nozzle: NozzleSku {
                    diameter: "0.4".to_string(),
                    material: NozzleMaterial::Stainless,
                },
                slots: vec![SlotBinding {
                    feed: FeedKind::Direct,
                    filament_identity: None,
                    color: None,
                }],
            }],
            bed: BedRef {
                identity: "Supertack Plate".into(),
            },
            config_overrides: Default::default(),
        };
        let json = serde_json::to_string(&inst).expect("ser");
        let parsed: PrinterInstance = serde_json::from_str(&json).expect("de");
        assert_eq!(parsed.id, "test");
        assert_eq!(parsed.extruders.len(), 1);
        assert_eq!(parsed.extruders[0].installed_nozzle.diameter, "0.4");
        assert_eq!(
            parsed.extruders[0].installed_nozzle.material,
            NozzleMaterial::Stainless,
        );
        assert_eq!(parsed.bed.identity, "Supertack Plate");
    }

    #[test]
    fn nozzle_material_serializes_snake_case() {
        let json = serde_json::to_string(&NozzleMaterial::HighFlowHardened).unwrap();
        assert_eq!(json, "\"high_flow_hardened\"");
    }

    /// Older persisted instances (and the bundled fixtures pre-color)
    /// don't carry a `color` field. The serde default keeps them
    /// loading as unassigned rather than failing.
    #[test]
    fn slot_binding_color_defaults_to_none_when_absent() {
        let raw = r#"{"label":"Ext","feed":"direct","filament_identity":null}"#;
        let s: SlotBinding = serde_json::from_str(raw).expect("legacy slot binding parses");
        assert_eq!(s.color, None);
    }
}
