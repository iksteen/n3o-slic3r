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
//! See `docs/dev/settings-model.md` §4 (Storage model — User library)
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

impl PrinterInstance {
    /// Number of installed AMS units, derived from the slot topology:
    /// each unit contributes `AMS_SLOTS_PER_UNIT` `FeedKind::Ams`
    /// slots on the first extruder. Mirrors the frontend `amsUnitsOf`.
    /// Used by the driver-sync path to reconcile the configured AMS
    /// count against the printer's reported physical loadout.
    pub fn ams_units(&self) -> u32 {
        let Some(extruder) = self.extruders.first() else {
            return 0;
        };
        let ams_slots = extruder
            .slots
            .iter()
            .filter(|s| matches!(s.feed, FeedKind::Ams))
            .count();
        (ams_slots / AMS_SLOTS_PER_UNIT) as u32
    }
}

/// Display label for an extruder, given its 0-based position and the
/// total extruder count. Multi-extruder printers (toolchangers) get
/// 1-based `T1..TN`; single-extruder printers get an empty label — the
/// slot label carries identity there.
fn extruder_label(ext_idx: usize, total_extruders: usize) -> String {
    if total_extruders <= 1 {
        String::new()
    } else {
        format!("T{}", ext_idx + 1)
    }
}

/// Where a slot sits within an AMS-style extruder. AMS-feed slots are
/// numbered 1-based across the extruder, grouped into units of
/// `AMS_SLOTS_PER_UNIT` once there is more than one unit's worth
/// (multi-unit topology); the trailing Direct-feed slot is the
/// external spool.
enum SlotPosition {
    Ams {
        unit_idx: usize,
        idx_in_unit: usize,
        multi_unit: bool,
    },
    Direct,
    None,
}

fn ams_slot_position(slot_idx: usize, slots: &[SlotBinding]) -> SlotPosition {
    let multi_unit = slots
        .iter()
        .filter(|s| matches!(s.feed, FeedKind::Ams))
        .count()
        > AMS_SLOTS_PER_UNIT;
    let mut idx_in_unit = 0;
    let mut unit_idx = 0;
    for (i, slot) in slots.iter().enumerate() {
        if matches!(slot.feed, FeedKind::Ams) {
            if idx_in_unit == AMS_SLOTS_PER_UNIT {
                idx_in_unit = 0;
                unit_idx += 1;
            }
            idx_in_unit += 1;
            if i == slot_idx {
                return SlotPosition::Ams {
                    unit_idx,
                    idx_in_unit,
                    multi_unit,
                };
            }
        } else if i == slot_idx {
            return SlotPosition::Direct;
        }
    }
    SlotPosition::None
}

/// AMS-unit letter for a 0-based unit index (`0 -> 'A'`).
fn unit_letter(unit_idx: usize) -> char {
    (b'A' + unit_idx as u8) as char
}

/// Long-form slot label (tooltips + picker dropdown), slot-scope only —
/// [`flatten_slots`] joins it with the extruder label. Single-slot
/// extruders surface identity through the extruder label (multi-extruder
/// printers) or a `Direct`/`AMS:1` feed-kind label (single-extruder).
/// Multi-slot extruders are AMS-style (`AMS:1`, or `AMS A:1` once
/// multi-unit, trailing `Ext`).
fn slot_label(slot_idx: usize, slots: &[SlotBinding], total_extruders: usize) -> String {
    if slots.len() == 1 {
        if total_extruders > 1 {
            return String::new();
        }
        return match slots[0].feed {
            FeedKind::Direct => "Direct".into(),
            FeedKind::Ams => "AMS:1".into(),
        };
    }
    match ams_slot_position(slot_idx, slots) {
        SlotPosition::Ams {
            unit_idx,
            idx_in_unit,
            multi_unit,
        } => {
            if multi_unit {
                format!("AMS {}:{}", unit_letter(unit_idx), idx_in_unit)
            } else {
                format!("AMS:{idx_in_unit}")
            }
        }
        SlotPosition::Direct => "Ext".into(),
        SlotPosition::None => String::new(),
    }
}

/// Compact slot label for a chip face. The extruder prefix (`T1`…)
/// appears only on multi-extruder printers; the per-slot part is dropped
/// when the extruder has a single slot (a toolchanger toolhead — the
/// extruder label *is* its identity, e.g. `T1`). Otherwise the slot part
/// is `Ext`/`1` (single-slot single-extruder), or AMS digits / `A:1`
/// (multi-unit) / `Ext`. Multi-slot multi-extruder printers (Bambu H2D:
/// AMS + external spool per extruder) get both — `T1·A:1`, `T1·Ext`.
fn slot_short_label(
    ext_idx: usize,
    total_extruders: usize,
    slot_idx: usize,
    slots: &[SlotBinding],
) -> String {
    let ext_part = if total_extruders > 1 {
        format!("T{}", ext_idx + 1)
    } else {
        String::new()
    };

    let slot_part = if slots.len() == 1 {
        // A single slot under a multi-extruder printer is a toolchanger
        // toolhead — the extruder label already identifies it.
        if total_extruders > 1 {
            String::new()
        } else {
            match slots[0].feed {
                FeedKind::Direct => "Ext".into(),
                FeedKind::Ams => "1".into(),
            }
        }
    } else {
        match ams_slot_position(slot_idx, slots) {
            SlotPosition::Ams {
                unit_idx,
                idx_in_unit,
                multi_unit,
            } => {
                if multi_unit {
                    format!("{}:{}", unit_letter(unit_idx), idx_in_unit)
                } else {
                    format!("{idx_in_unit}")
                }
            }
            SlotPosition::Direct => "Ext".into(),
            SlotPosition::None => String::new(),
        }
    };

    match (ext_part.is_empty(), slot_part.is_empty()) {
        (false, false) => format!("{ext_part}·{slot_part}"),
        (false, true) => ext_part,
        (true, false) => slot_part,
        (true, true) => format!("{}", slot_idx + 1),
    }
}

/// Flattened, pre-labeled slot for the frontend. Carries the labels
/// Rust derives from topology (extruder count + per-slot `feed`)
/// alongside the slot's binding state; the renderer reads these fields
/// directly instead of re-deriving labels from the slot grid. `ref`
/// locates the slot for write-back commands.
#[derive(Debug, Clone, Serialize)]
pub struct SlotView {
    #[serde(rename = "ref")]
    pub slot_ref: SlotRef,
    /// Long label — extruder + slot joined (`"T1 — AMS A:1"`, `"AMS:1"`,
    /// `"Ext"`), falling back to `"Slot N"` when both parts are empty.
    pub label: String,
    /// Compact chip label (`"A:1"`, `"1"`, `"Ext"`, `"T1"`).
    pub short_label: String,
    pub feed: FeedKind,
    pub filament_identity: Option<String>,
    pub color: Option<String>,
    pub tag_uid: Option<String>,
}

/// Flatten an instance's extruder × slot grid into pre-labeled slot
/// views — one per `(extruder, slot)`, in slice order. The single
/// source of truth for slot labels: the frontend renders these, it
/// does not re-derive them.
pub fn flatten_slots(instance: &PrinterInstance) -> Vec<SlotView> {
    let total_ext = instance.extruders.len();
    let mut out = Vec::new();
    for (e_idx, ext) in instance.extruders.iter().enumerate() {
        let ext_label = extruder_label(e_idx, total_ext);
        for (s_idx, slot) in ext.slots.iter().enumerate() {
            let s_label = slot_label(s_idx, &ext.slots, total_ext);
            let label = match (ext_label.is_empty(), s_label.is_empty()) {
                (false, false) => format!("{ext_label} — {s_label}"),
                (false, true) => ext_label.clone(),
                (true, false) => s_label,
                (true, true) => format!("Slot {}", s_idx + 1),
            };
            out.push(SlotView {
                slot_ref: SlotRef {
                    extruder: e_idx as u8,
                    slot: s_idx as u8,
                },
                label,
                short_label: slot_short_label(e_idx, total_ext, s_idx, &ext.slots),
                feed: slot.feed,
                filament_identity: slot.filament_identity.clone(),
                color: slot.color.clone(),
                tag_uid: slot.tag_uid.clone(),
            });
        }
    }
    out
}

/// Frontend-facing instance snapshot: the persisted [`PrinterInstance`]
/// plus the Rust-computed slot views and AMS-unit count the renderer
/// would otherwise re-derive. Serialize-only and never persisted — the
/// on-disk library writes the inner `PrinterInstance`, so the derived
/// labels can't go stale on disk.
#[derive(Debug, Clone, Serialize)]
pub struct PrinterInstanceView {
    #[serde(flatten)]
    pub instance: PrinterInstance,
    pub slots: Vec<SlotView>,
    pub ams_units: u32,
}

impl PrinterInstanceView {
    pub fn of(instance: PrinterInstance) -> Self {
        let slots = flatten_slots(&instance);
        let ams_units = instance.ams_units();
        Self {
            instance,
            slots,
            ams_units,
        }
    }
}

#[cfg(test)]
mod label_tests {
    use super::*;

    fn binding(feed: FeedKind) -> SlotBinding {
        SlotBinding {
            feed,
            filament_identity: None,
            color: None,
            tag_uid: None,
        }
    }

    /// Build an instance from a per-extruder feed layout — the only
    /// thing `flatten_slots` reads.
    fn instance(extruders: Vec<Vec<FeedKind>>) -> PrinterInstance {
        PrinterInstance {
            id: "t".into(),
            display_name: "T".into(),
            vendor_profile_ref: "x".into(),
            printer_fragment_slug: "x".into(),
            default_filament_fragment_slug: "x".into(),
            quality_profile: "x".into(),
            connection: None,
            extruders: extruders
                .into_iter()
                .map(|feeds| ExtruderState {
                    installed_nozzle: NozzleSku {
                        diameter: "0.4".into(),
                        material: NozzleMaterial::Stainless,
                    },
                    slots: feeds.into_iter().map(binding).collect(),
                })
                .collect(),
            bed: BedRef {
                identity: "b".into(),
            },
            config_overrides: Default::default(),
        }
    }

    use FeedKind::{Ams, Direct};

    fn labels(inst: &PrinterInstance) -> Vec<String> {
        flatten_slots(inst).into_iter().map(|s| s.label).collect()
    }

    #[test]
    fn single_direct_slot_is_direct() {
        let slots = flatten_slots(&instance(vec![vec![Direct]]));
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].label, "Direct");
        assert_eq!(slots[0].short_label, "Ext");
        assert_eq!(slots[0].slot_ref, SlotRef { extruder: 0, slot: 0 });
    }

    #[test]
    fn toolchanger_labels_by_extruder() {
        let inst = instance(vec![vec![Direct]; 4]);
        assert_eq!(labels(&inst), ["T1", "T2", "T3", "T4"]);
        let shorts: Vec<_> = flatten_slots(&inst)
            .into_iter()
            .map(|s| s.short_label)
            .collect();
        assert_eq!(shorts, ["T1", "T2", "T3", "T4"]);
    }

    #[test]
    fn single_ams_unit_labels() {
        let inst = instance(vec![vec![Ams, Ams, Ams, Ams, Direct]]);
        assert_eq!(labels(&inst), ["AMS:1", "AMS:2", "AMS:3", "AMS:4", "Ext"]);
        let shorts: Vec<_> = flatten_slots(&inst)
            .into_iter()
            .map(|s| s.short_label)
            .collect();
        assert_eq!(shorts, ["1", "2", "3", "4", "Ext"]);
    }

    #[test]
    fn h2d_shape_two_extruders_each_with_ams_plus_external() {
        // Bambu H2D: 2 independently-fed extruders, each [AMS×4, Ext].
        // The extruder prefix disambiguates slots that would otherwise
        // collide across extruders — long labels compose, chips carry
        // both extruder + slot.
        let ext = || vec![Ams, Ams, Ams, Ams, Direct];
        let inst = instance(vec![ext(), ext()]);
        assert_eq!(
            labels(&inst),
            [
                "T1 — AMS:1", "T1 — AMS:2", "T1 — AMS:3", "T1 — AMS:4", "T1 — Ext", "T2 — AMS:1",
                "T2 — AMS:2", "T2 — AMS:3", "T2 — AMS:4", "T2 — Ext",
            ]
        );
        let shorts: Vec<_> = flatten_slots(&inst)
            .into_iter()
            .map(|s| s.short_label)
            .collect();
        assert_eq!(
            shorts,
            [
                "T1·1", "T1·2", "T1·3", "T1·4", "T1·Ext", "T2·1", "T2·2", "T2·3", "T2·4", "T2·Ext",
            ]
        );
    }

    #[test]
    fn multi_ams_unit_letter_prefix() {
        // 3 units = 12 AMS slots + 1 Ext → A:1..4, B:1..4, C:1..4, Ext.
        let mut feeds = vec![Ams; 12];
        feeds.push(Direct);
        let inst = instance(vec![feeds]);
        assert_eq!(
            labels(&inst),
            [
                "AMS A:1", "AMS A:2", "AMS A:3", "AMS A:4", "AMS B:1", "AMS B:2", "AMS B:3",
                "AMS B:4", "AMS C:1", "AMS C:2", "AMS C:3", "AMS C:4", "Ext",
            ]
        );
        let shorts: Vec<_> = flatten_slots(&inst)
            .into_iter()
            .map(|s| s.short_label)
            .collect();
        assert_eq!(
            shorts,
            [
                "A:1", "A:2", "A:3", "A:4", "B:1", "B:2", "B:3", "B:4", "C:1", "C:2", "C:3", "C:4",
                "Ext",
            ]
        );
    }
}

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

    /// Last-synced RFID tag id (Bambu `tag_uid`) for the spool in this
    /// slot. Persisted so the UI can mark an RFID-auto-detected slot
    /// read-only without a live connection. `None` (or all-zeros, per
    /// [`crate::core::driver::status::rfid_detected`]) = not
    /// RFID-detected, so the slot is user-editable. Written by the
    /// destructive driver sync alongside `filament_identity`/`color`.
    #[serde(default)]
    pub tag_uid: Option<String>,
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
/// 8-hex-char LAN access code; U1 and generic Moonraker printers need
/// a Moonraker port (usually 80).
///
/// Device serial is intentionally NOT persisted here: the Bambu
/// driver resolves it at connect time (peer-cert CN), Moonraker
/// printers have none, and nothing in the UI ever authored it.
/// Keeping it out of the stored connection avoids a dead
/// round-tripped field and a stale-serial footgun.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConnectionInfo {
    Bambu { host: String, access_code: String },
    U1 { host: String, port: u16 },
    /// Generic Klipper printer speaking vanilla Moonraker (same
    /// fields as U1, which is Moonraker plus a vendor webcam stack).
    Moonraker { host: String, port: u16 },
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
                    tag_uid: None,
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
