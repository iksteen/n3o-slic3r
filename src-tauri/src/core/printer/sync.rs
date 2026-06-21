//! Driver-report → SlotBinding sync resolver (PR-7c-2).
//!
//! The sync button on the slot chip strip pulls the printer's
//! current spool loadout into the PrinterInstance. The driver layer
//! already exposes the wire shape — Bambu's MQTT `ams.units[].trays`
//! and U1's per-toolhead filament report. This module decides what
//! each driver-reported tray translates to in our world:
//!
//! - **Identity** (`SlotBinding.filament_identity`):
//!   - Bambu: exact match on `tray_info_idx` ↔ bundled
//!     `FilamentFragmentSummary.filament_id` (the GFA00-style
//!     vendor SKU). Fall back to `generic-<base_type>` on miss.
//!   - U1: keep the slot's current identity when its base_type
//!     matches the reported `material_type`; otherwise fall back
//!     to `generic-<material_type>`.
//! - **Color** (`SlotBinding.color`): always overwrite from the
//!   driver report, converting BBL's `RRGGBBAA` (no `#`) to our
//!   `#RRGGBB` CSS form.
//!
//! The sync mirrors the report destructively (see
//! `memory/feedback_sync_is_destructive_by_design.md`): a slot the report
//! **enumerates as empty** — an AMS tray with no spool, an unmounted U1
//! toolhead — is **cleared** (`filament_identity` + `color` → `None`), not
//! left at its prior binding. The Bambu external spool is the exception:
//! it's a single optional field, not an enumerated slot, and `None` is
//! ambiguous (no external feed / a manually-filled Direct slot, since
//! vt_tray has no RFID), so it's written only when a spool is reported.
//! Manual entry remains a first-class write path
//! (`memory/project_filament_id_sources.md`) — sync just overwrites it.

use crate::core::driver::status::{AmsFilament, DriverExtra, U1Extra, U1Filament};
use crate::core::printer::instance::{PrinterInstance, SlotBinding};
use crate::core::profile_library::FilamentFragmentSummary;

/// One slot's intended new state, as resolved from a driver report.
/// Either both fields are written together (mutate transaction
/// rewrites both) or the slot is omitted from the diff entirely.
#[derive(Debug, Clone, PartialEq)]
pub struct SlotUpdate {
    pub extruder_idx: usize,
    pub slot_idx: usize,
    /// The new identity. `Some(slug)` writes a binding; `None` together
    /// with `color: None` clears the slot — emitted when the driver
    /// reports the physical slot as empty, so a removed spool is synced
    /// out (the sync is destructive by design — it mirrors the report).
    pub filament_identity: Option<String>,
    /// CSS-style hex color (`#RRGGBB`), or `None` for a cleared slot.
    pub color: Option<String>,
    /// RFID tag id (Bambu `tag_uid`) of the reported spool, or `None`
    /// for a cleared slot, an untagged/third-party spool, or a feed
    /// with no RFID (vt_tray external spool, U1 toolheads). Persisted
    /// onto the slot so the UI can mark RFID-auto-detected slots
    /// read-only — see [`crate::core::driver::status::rfid_detected`].
    pub tag_uid: Option<String>,
}

/// Reconcile an instance to a driver status report in one atomic
/// mutation: first match the AMS-unit count to the reported physical
/// loadout, then fill the (resized) slots with the reported filament
/// identities + colors. Single registry lock, single persist.
///
/// AMS-count eligibility (AMS-style printer, count within `ams_max`)
/// is delegated to [`instance_registry::validate_ams_request`] — a
/// non-AMS printer or an over-`ams_max` report fails that check, which
/// we treat as "leave the count alone" rather than re-deriving the
/// predicate here. A shrink is allowed (`rebuild_ams_slots` preserves
/// overlapping bindings); transient/placeholder driver reports are
/// already filtered upstream in the status pipeline.
pub fn apply_from_driver(
    instance_id: &str,
    extra: &DriverExtra,
    library: &[FilamentFragmentSummary],
) -> Result<PrinterInstance, crate::core::printer::instance_registry::InstanceMutError> {
    use crate::core::printer::instance_registry as reg;
    reg::mutate_instance(instance_id, |inst| {
        if let Some(reported) = ams_units_reported(extra) {
            if reported != inst.ams_units()
                && reg::validate_ams_request(inst, instance_id, reported, None).is_ok()
            {
                reg::rebuild_ams_slots(inst, instance_id, reported);
            }
        }
        // Resolve against the (possibly resized) topology, then apply.
        // `resolve_updates` returns an owned Vec, so the immutable
        // borrow of `inst` ends before the mutating loop.
        let updates = resolve_updates(inst, extra, library);
        for u in &updates {
            if let Some(ext) = inst.extruders.get_mut(u.extruder_idx) {
                if let Some(slot) = ext.slots.get_mut(u.slot_idx) {
                    slot.filament_identity = u.filament_identity.clone();
                    slot.color = u.color.clone();
                    slot.tag_uid = u.tag_uid.clone();
                }
            }
        }
        Ok(())
    })
}

/// Resolve a driver-report into per-slot updates for `instance`. A slot
/// the report enumerates produces an update — a loaded one writes the
/// resolved identity + color, an empty one clears them. Slots the report
/// doesn't enumerate (the external spool when absent) are omitted; the
/// caller leaves those alone.
pub fn resolve_updates(
    instance: &PrinterInstance,
    extra: &DriverExtra,
    library: &[FilamentFragmentSummary],
) -> Vec<SlotUpdate> {
    match extra {
        DriverExtra::Bambu(b) => resolve_bambu(instance, b, library),
        DriverExtra::U1(u) => resolve_u1(instance, u, library),
    }
}

/// The number of AMS units the driver currently reports, when it
/// reports an AMS state at all. `Some(n)` lets the sync reconcile the
/// instance's AMS-unit count to the physical loadout before resolving
/// per-slot updates.
///
/// `None` means "leave the count alone": either the printer has no
/// AMS topology to sync (U1 toolchanger), or it hasn't reported an
/// AMS state yet — and we must not destructively zero the AMS slots
/// off a not-yet-populated status snapshot. The external spool alone
/// (Bambu `vt_tray` with no AMS) is therefore NOT treated as "0 AMS
/// units".
pub fn ams_units_reported(extra: &DriverExtra) -> Option<u32> {
    match extra {
        DriverExtra::Bambu(b) => b.ams.as_ref().map(|a| a.units.len() as u32),
        DriverExtra::U1(_) => None,
    }
}

fn resolve_bambu(
    instance: &PrinterInstance,
    extra: &crate::core::driver::status::BambuExtra,
    library: &[FilamentFragmentSummary],
) -> Vec<SlotUpdate> {
    // A1 mini: one extruder, slots 0..3 are the AMS-feed trays in
    // their natural order, slot 4 is the Direct external spool
    // (mirrored from `print.vt_tray`). Multi-AMS topologies (X1C
    // with 4 units) would stack as unit*4 + tray; we keep the same
    // indexing rule here so it doesn't need re-touching when that
    // lands.
    let mut out = Vec::new();
    let extruder_idx = 0usize;
    let Some(ext) = instance.extruders.first() else {
        return out;
    };
    if let Some(ams) = &extra.ams {
        for (unit_pos, unit) in ams.units.iter().enumerate() {
            for tray in &unit.trays {
                let slot_idx = unit_pos * 4 + tray.id as usize;
                // Skip slots that aren't AMS-feed — the trailing
                // Direct (Ext) slot is owned by vt_tray below.
                if !slot_at_index_is_ams_feed(ext, slot_idx) {
                    continue;
                }
                // An empty tray clears the slot (destructive sync); a
                // loaded one writes the reported identity + color.
                out.push(match &tray.identity {
                    Some(identity) => SlotUpdate {
                        extruder_idx,
                        slot_idx,
                        filament_identity: resolve_bambu_identity(
                            ext.slots.get(slot_idx),
                            identity,
                            library,
                        ),
                        color: Some(hex8_to_css(&identity.color)),
                        tag_uid: identity.tag_uid.clone(),
                    },
                    None => SlotUpdate {
                        extruder_idx,
                        slot_idx,
                        filament_identity: None,
                        color: None,
                        tag_uid: None,
                    },
                });
            }
        }
    }
    // External spool → first Direct slot on the same extruder. On the A1
    // mini that's slot 4 (after the 4 AMS feeds); future layouts
    // (Direct-only without AMS, or a non-trailing Direct) work the same —
    // find the first Direct and write into it.
    //
    // Unlike the AMS trays (each enumerated with an explicit empty state),
    // the external spool is a single optional field — `None` is ambiguous
    // (no external feed / nothing reported / a Direct slot the user filled
    // manually, since vt_tray has no RFID). So we only write when a spool
    // is reported and leave the slot alone otherwise, rather than clearing
    // a possibly-manual binding.
    if let Some(vt) = &extra.external_spool {
        if let Some(slot_idx) = first_direct_slot(ext) {
            out.push(SlotUpdate {
                extruder_idx,
                slot_idx,
                filament_identity: resolve_bambu_identity(
                    ext.slots.get(slot_idx),
                    vt,
                    library,
                ),
                color: Some(hex8_to_css(&vt.color)),
                // vt_tray (external spool) has no RFID reader — never
                // RFID-detected, so it stays user-editable.
                tag_uid: None,
            });
        }
    }
    out
}

fn resolve_u1(
    instance: &PrinterInstance,
    extra: &U1Extra,
    library: &[FilamentFragmentSummary],
) -> Vec<SlotUpdate> {
    let mut out = Vec::new();
    for (ext_idx, slot) in extra.toolhead_filaments.iter().enumerate() {
        // U1 has one slot per toolhead; the slot binding sits at
        // (extruder=ext_idx, slot=0). Skip toolheads the instance
        // doesn't declare.
        let Some(extruder) = instance.extruders.get(ext_idx) else {
            continue;
        };
        let Some(current) = extruder.slots.first() else {
            continue;
        };
        // A mounted filament writes the resolved identity + color; an
        // empty toolhead clears the slot (destructive sync).
        out.push(match slot {
            Some(filament) => SlotUpdate {
                extruder_idx: ext_idx,
                slot_idx: 0,
                filament_identity: resolve_u1_identity(current, filament, library),
                color: Some(hex8_to_css(&filament.color)),
                // U1 toolheads have no AMS RFID concept.
                tag_uid: None,
            },
            None => SlotUpdate {
                extruder_idx: ext_idx,
                slot_idx: 0,
                filament_identity: None,
                color: None,
                tag_uid: None,
            },
        });
    }
    out
}

fn resolve_bambu_identity(
    current: Option<&SlotBinding>,
    identity: &AmsFilament,
    library: &[FilamentFragmentSummary],
) -> Option<String> {
    // Keep the current, more-specific binding when it's still
    // consistent with the report. The AMS round-trip is *lossy* for
    // generic sub-variants: "Generic PLA", "…Silk", "…Matte", "…CF" all
    // collapse to the same `GFL99` sentinel + `tray_type:"PLA"` on the
    // printer (there's no field for the variant). So a destructive
    // overwrite could only ever downgrade the user's pick. If the bound
    // fragment's SKU and material both still match what the printer
    // reports, the user's choice stands — only the color follows the
    // report. (Same rule the U1 path uses, see `resolve_u1_identity`.)
    if let Some(slug) = current.and_then(|c| c.filament_identity.as_deref()) {
        if let Some(cur) = library.iter().find(|f| f.identity == slug) {
            if cur.filament_id.as_deref() == identity.filament_id.as_deref()
                && cur.base_type.eq_ignore_ascii_case(identity.tray_type.trim())
            {
                return Some(slug.to_owned());
            }
        }
    }
    // No current match (a genuine change) — resolve fresh. Trust the
    // reported SKU only when it uniquely identifies one fragment: a real
    // RFID tag carries a unique vendor SKU (`GFA05` → Bambu PLA Silk),
    // but Bambu stamps a generic sentinel (`GFL99` PLA / `GFG99` PETG /
    // `GFB99` ABS …) on any non-RFID spool, and many bundled generics
    // share each one. A first-match on a shared sentinel resolved the
    // user's "Generic PLA" to whichever generic sorts first (it was
    // `bambu-pla` → "Bambu PLA", the revert bug), so a shared SKU is not
    // a discriminator.
    if let Some(fid) = identity.filament_id.as_deref() {
        let mut hits = library
            .iter()
            .filter(|f| f.filament_id.as_deref() == Some(fid));
        if let Some(first) = hits.next() {
            if hits.next().is_none() {
                return Some(first.identity.clone());
            }
        }
    }
    // Shared sentinel / unknown SKU: the variant is carried by the
    // sub-brand label. We stamp the fragment's display name there on
    // write (and RFID spools carry their own), so match it back to pin
    // the exact variant — Generic PLA *Silk* survives the round-trip
    // instead of collapsing to plain Generic PLA. Material must agree so
    // a stale label can't cross a real material change.
    if let Some(sub_brand) = identity
        .sub_brand
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if let Some(hit) = library.iter().find(|f| {
            f.display_name.eq_ignore_ascii_case(sub_brand)
                && f.base_type.eq_ignore_ascii_case(identity.tray_type.trim())
        }) {
            return Some(hit.identity.clone());
        }
    }
    // Nothing specific to go on → the generic variant matching the
    // reported material type, rather than the wrong vendor profile.
    generic_identity_for(&identity.tray_type, library)
}

fn resolve_u1_identity(
    current: &SlotBinding,
    filament: &U1Filament,
    library: &[FilamentFragmentSummary],
) -> Option<String> {
    // Keep the current binding when its base_type already matches —
    // the user's manual pick of a specific brand+product stands.
    if let Some(slug) = current.filament_identity.as_deref() {
        if let Some(entry) = library.iter().find(|f| f.identity == slug) {
            if entry
                .base_type
                .eq_ignore_ascii_case(&filament.material_type)
            {
                return Some(slug.to_owned());
            }
        }
    }
    generic_identity_for(&filament.material_type, library)
}

fn generic_identity_for(material: &str, library: &[FilamentFragmentSummary]) -> Option<String> {
    // Match on the exact slug `generic-<material lowercased>` —
    // the convention is `generic-pla` for the base variant with
    // suffixes like `-cf` / `-silk` for specialty composites. A
    // vendor+base_type match alone would let `generic-pla-silk`
    // shadow `generic-pla` (or any other variant ordering
    // accident); the slug pattern is the reliable signal.
    let want = format!("generic-{}", material.to_ascii_lowercase());
    library
        .iter()
        .find(|f| f.identity == want)
        .map(|f| f.identity.clone())
}

fn slot_at_index_is_ams_feed(
    extruder: &crate::core::printer::instance::ExtruderState,
    slot_idx: usize,
) -> bool {
    extruder
        .slots
        .get(slot_idx)
        .map(|s| matches!(s.feed, crate::core::printer::instance::FeedKind::Ams))
        .unwrap_or(false)
}

fn first_direct_slot(extruder: &crate::core::printer::instance::ExtruderState) -> Option<usize> {
    extruder
        .slots
        .iter()
        .position(|s| matches!(s.feed, crate::core::printer::instance::FeedKind::Direct))
}

/// Convert Bambu's `RRGGBBAA` (no `#`) to our `#RRGGBB` CSS form.
/// Alpha is dropped — slot color is treated as opaque; translucent
/// material is a separate concept owned by the palette entry.
fn hex8_to_css(rrggbbaa: &str) -> String {
    let raw = rrggbbaa.trim().trim_start_matches('#');
    let rgb = if raw.len() >= 6 { &raw[..6] } else { raw };
    format!("#{}", rgb.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::driver::status::{
        AmsFilament, AmsState, AmsTray, AmsUnit, BambuExtra, DriverExtra, U1Extra, U1Filament,
    };
    use crate::core::printer::instance::{
        BedRef, ExtruderState, FeedKind, NozzleMaterial, NozzleSku, PrinterInstance, SlotBinding,
    };

    fn bambi() -> PrinterInstance {
        PrinterInstance {
            id: "b".into(),
            display_name: "Bambi".into(),
            vendor_profile_ref: "bambu-lab-a1-mini".into(),
            printer_fragment_slug: "bambu-lab-a1-mini".into(),
            default_filament_fragment_slug: "bambu-pla-basic".into(),
            quality_profile: "0.20mm-standard".into(),
            connection: None,
            extruders: vec![ExtruderState {
                installed_nozzle: NozzleSku {
                    diameter: "0.4".to_string(),
                    material: NozzleMaterial::Stainless,
                },
                slots: vec![
                    SlotBinding {
                        feed: FeedKind::Ams,
                        filament_identity: None,
                        color: None,
                        tag_uid: None,
                    },
                    SlotBinding {
                        feed: FeedKind::Ams,
                        filament_identity: None,
                        color: None,
                        tag_uid: None,
                    },
                    SlotBinding {
                        feed: FeedKind::Ams,
                        filament_identity: None,
                        color: None,
                        tag_uid: None,
                    },
                    SlotBinding {
                        feed: FeedKind::Ams,
                        filament_identity: None,
                        color: None,
                        tag_uid: None,
                    },
                    SlotBinding {
                        feed: FeedKind::Direct,
                        filament_identity: None,
                        color: None,
                        tag_uid: None,
                    },
                ],
            }],
            bed: BedRef {
                identity: "Bambu Cool Plate SuperTack".into(),
            },
            config_overrides: Default::default(),
        }
    }

    fn snappy() -> PrinterInstance {
        PrinterInstance {
            id: "s".into(),
            display_name: "Snappy".into(),
            vendor_profile_ref: "snapmaker-u1".into(),
            printer_fragment_slug: "snapmaker-u1".into(),
            default_filament_fragment_slug: "snapmaker-pla".into(),
            quality_profile: "0.20-standard".into(),
            connection: None,
            extruders: vec![0, 1, 2, 3]
                .into_iter()
                .map(|_| ExtruderState {
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
                })
                .collect(),
            bed: BedRef {
                identity: "Snapmaker Textured PEI".into(),
            },
            config_overrides: Default::default(),
        }
    }

    fn lib() -> Vec<FilamentFragmentSummary> {
        vec![
            FilamentFragmentSummary {
                identity: "bambu-pla-basic-bbl-a1m".into(),
                display_name: "Bambu PLA Basic @BBL A1M".into(),
                base_type: "PLA".into(),
                vendor: "Bambu Lab".into(),
                nozzle_temp: 220,
                bed_temp: 60,
                filament_id: Some("GFA00".into()),
                edited: false,
            },
            // Specialty PLA variants placed *before* generic-pla so
            // the test catches the "first vendor+base_type match
            // wins" bug (PR-7c-2: U1 sync resolved PLA → silk).
            FilamentFragmentSummary {
                identity: "generic-pla-silk".into(),
                display_name: "Generic PLA Silk".into(),
                base_type: "PLA".into(),
                vendor: "Generic".into(),
                nozzle_temp: 220,
                bed_temp: 60,
                // Shares the PLA generic sentinel with plain generic-pla
                // — the printer can't tell silk from plain.
                filament_id: Some("GFL99".into()),
                edited: false,
            },
            FilamentFragmentSummary {
                identity: "generic-pla-cf".into(),
                display_name: "Generic PLA-CF".into(),
                base_type: "PLA".into(),
                vendor: "Generic".into(),
                nozzle_temp: 240,
                bed_temp: 65,
                filament_id: None,
                edited: false,
            },
            // Shares the generic `GFL99` sentinel with `generic-pla`
            // (in the real bundle ~19 fragments do) and is placed
            // *first*, so a naive first-match would wrongly pick it.
            // The resolver must treat the shared SKU as ambiguous.
            FilamentFragmentSummary {
                identity: "bambu-pla".into(),
                display_name: "Bambu PLA".into(),
                base_type: "PLA".into(),
                vendor: "Bambu Lab".into(),
                nozzle_temp: 220,
                bed_temp: 60,
                filament_id: Some("GFL99".into()),
                edited: false,
            },
            FilamentFragmentSummary {
                identity: "generic-pla".into(),
                display_name: "Generic PLA".into(),
                base_type: "PLA".into(),
                vendor: "Generic".into(),
                nozzle_temp: 210,
                bed_temp: 60,
                filament_id: Some("GFL99".into()),
                edited: false,
            },
            FilamentFragmentSummary {
                identity: "generic-petg".into(),
                display_name: "Generic PETG".into(),
                base_type: "PETG".into(),
                vendor: "Generic".into(),
                nozzle_temp: 240,
                bed_temp: 80,
                filament_id: None,
                edited: false,
            },
            // Specialty-only family — no plain `generic-pa`. Used
            // by the "no base variant" test below.
            FilamentFragmentSummary {
                identity: "generic-pa-cf".into(),
                display_name: "Generic PA-CF".into(),
                base_type: "PA".into(),
                vendor: "Generic".into(),
                nozzle_temp: 295,
                bed_temp: 100,
                filament_id: None,
                edited: false,
            },
        ]
    }

    fn ams_with_trays(trays: Vec<AmsTray>) -> AmsState {
        AmsState {
            units: vec![AmsUnit { id: 0, trays }],
            active_slot: None,
        }
    }

    fn empty_tray(id: u8) -> AmsTray {
        AmsTray { id, identity: None }
    }

    fn tray_with(id: u8, filament_id: Option<&str>, material: &str, color: &str) -> AmsTray {
        AmsTray {
            id,
            identity: Some(AmsFilament {
                tray_type: material.into(),
                color: color.into(),
                sub_brand: None,
                multi_colors: vec![],
                filament_id: filament_id.map(str::to_owned),
                tag_uid: None,
            }),
        }
    }

    /// Like [`tray_with`] but with an RFID tag id, for exercising the
    /// `tag_uid` sync round-trip.
    fn tray_with_tag(
        id: u8,
        filament_id: Option<&str>,
        material: &str,
        color: &str,
        tag_uid: &str,
    ) -> AmsTray {
        AmsTray {
            id,
            identity: Some(AmsFilament {
                tray_type: material.into(),
                color: color.into(),
                sub_brand: None,
                multi_colors: vec![],
                filament_id: filament_id.map(str::to_owned),
                tag_uid: Some(tag_uid.to_owned()),
            }),
        }
    }

    #[test]
    fn ams_units_reported_counts_bambu_units() {
        let extra = DriverExtra::Bambu(BambuExtra {
            ams: Some(AmsState {
                units: vec![
                    AmsUnit {
                        id: 0,
                        trays: vec![],
                    },
                    AmsUnit {
                        id: 1,
                        trays: vec![],
                    },
                ],
                active_slot: None,
            }),
            ..Default::default()
        });
        assert_eq!(ams_units_reported(&extra), Some(2));
    }

    #[test]
    fn ams_units_reported_is_none_without_an_ams_state() {
        // External spool only (AMS detached / not yet reported) must
        // NOT read as "0 units" — the sync leaves the count alone.
        let extra = DriverExtra::Bambu(BambuExtra {
            ams: None,
            external_spool: Some(AmsFilament::default()),
            ..Default::default()
        });
        assert_eq!(ams_units_reported(&extra), None);
    }

    #[test]
    fn ams_units_reported_is_none_for_u1() {
        assert_eq!(
            ams_units_reported(&DriverExtra::U1(U1Extra::default())),
            None,
        );
    }

    #[test]
    fn bambu_exact_filament_id_match_resolves_to_bundled_slug() {
        let inst = bambi();
        let ams = ams_with_trays(vec![tray_with(0, Some("GFA00"), "PLA", "FF0000FF")]);
        let updates = resolve_updates(
            &inst,
            &DriverExtra::Bambu(BambuExtra {
                ams: Some(ams),
                ..Default::default()
            }),
            &lib(),
        );
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].extruder_idx, 0);
        assert_eq!(updates[0].slot_idx, 0);
        assert_eq!(
            updates[0].filament_identity.as_deref(),
            Some("bambu-pla-basic-bbl-a1m")
        );
        assert_eq!(updates[0].color.as_deref(), Some("#ff0000"));
    }

    #[test]
    fn bambu_sync_keeps_a_still_consistent_specific_variant() {
        // The AMS can't represent generic sub-variants (silk/matte/cf
        // all report GFL99 + tray_type PLA). A user who picked "Generic
        // PLA Silk" must NOT have it downgraded to plain "Generic PLA"
        // on every sync — the bound variant stands while the report is
        // consistent (same SKU + material); only the color follows.
        let mut inst = bambi();
        inst.extruders[0].slots[0].filament_identity = Some("generic-pla-silk".into());
        let ams = ams_with_trays(vec![tray_with(0, Some("GFL99"), "PLA", "1A1B1DFF")]);
        let updates = resolve_updates(
            &inst,
            &DriverExtra::Bambu(BambuExtra {
                ams: Some(ams),
                ..Default::default()
            }),
            &lib(),
        );
        assert_eq!(
            updates[0].filament_identity.as_deref(),
            Some("generic-pla-silk"),
            "a consistent specific variant is preserved, not downgraded",
        );
        assert_eq!(updates[0].color.as_deref(), Some("#1a1b1d"), "color follows report");
    }

    #[test]
    fn bambu_sync_replaces_binding_when_material_genuinely_changes() {
        // But a real change must still win: the slot holds silk (PLA),
        // the printer now reports a PETG generic → re-resolve, don't keep
        // the stale PLA binding.
        let mut inst = bambi();
        inst.extruders[0].slots[0].filament_identity = Some("generic-pla-silk".into());
        let ams = ams_with_trays(vec![tray_with(0, Some("GFG99"), "PETG", "00AA88FF")]);
        let updates = resolve_updates(
            &inst,
            &DriverExtra::Bambu(BambuExtra {
                ams: Some(ams),
                ..Default::default()
            }),
            &lib(),
        );
        assert_eq!(
            updates[0].filament_identity.as_deref(),
            Some("generic-petg"),
            "a genuine material change re-resolves",
        );
    }

    #[test]
    fn bambu_sub_brand_label_pins_the_variant_without_a_local_binding() {
        // The variant carrier: we stamp the fragment name into
        // tray_sub_brands on write, so even a fresh instance (no local
        // binding) resolves the right variant from the printer's report
        // — `GFL99` + sub-brand "Generic PLA Silk" → generic-pla-silk,
        // not plain generic-pla.
        let inst = bambi(); // slots have no bound filament
        let ams = AmsState {
            units: vec![AmsUnit {
                id: 0,
                trays: vec![AmsTray {
                    id: 0,
                    identity: Some(AmsFilament {
                        tray_type: "PLA".into(),
                        color: "1A1B1DFF".into(),
                        sub_brand: Some("Generic PLA Silk".into()),
                        multi_colors: vec![],
                        filament_id: Some("GFL99".into()),
                        tag_uid: None,
                    }),
                }],
            }],
            active_slot: None,
        };
        let updates = resolve_updates(
            &inst,
            &DriverExtra::Bambu(BambuExtra {
                ams: Some(ams),
                ..Default::default()
            }),
            &lib(),
        );
        assert_eq!(
            updates[0].filament_identity.as_deref(),
            Some("generic-pla-silk"),
            "sub-brand label resolves the exact variant",
        );
    }

    #[test]
    fn bambu_generic_sentinel_sku_falls_back_to_material_generic() {
        // Bambu stamps the shared `GFL99` sentinel on a non-RFID generic
        // spool (verified on the wire). Resolving it must NOT pick the
        // first fragment carrying GFL99 (here `bambu-pla`, sorted first)
        // — it must fall back to the material-type generic. This is the
        // "reverts to Bambu PLA after sync" bug.
        let inst = bambi();
        let ams = ams_with_trays(vec![tray_with(0, Some("GFL99"), "PLA", "1A1B1DFF")]);
        let updates = resolve_updates(
            &inst,
            &DriverExtra::Bambu(BambuExtra {
                ams: Some(ams),
                ..Default::default()
            }),
            &lib(),
        );
        assert_eq!(
            updates[0].filament_identity.as_deref(),
            Some("generic-pla"),
            "shared GFL99 sentinel must resolve to the material generic, not bambu-pla",
        );
    }

    #[test]
    fn bambu_unknown_filament_id_falls_back_to_generic_material() {
        let inst = bambi();
        let ams = ams_with_trays(vec![tray_with(0, Some("UNKNOWN"), "PETG", "AABBCCFF")]);
        let updates = resolve_updates(
            &inst,
            &DriverExtra::Bambu(BambuExtra {
                ams: Some(ams),
                ..Default::default()
            }),
            &lib(),
        );
        assert_eq!(updates.len(), 1);
        assert_eq!(
            updates[0].filament_identity.as_deref(),
            Some("generic-petg")
        );
        assert_eq!(updates[0].color.as_deref(), Some("#aabbcc"));
    }

    #[test]
    fn bambu_no_filament_id_falls_back_to_generic_material() {
        // Untagged spool: material is reported by user via on-screen
        // tray-info edit, but no RFID tag means no filament_id.
        let inst = bambi();
        let ams = ams_with_trays(vec![tray_with(1, None, "PLA", "112233FF")]);
        let updates = resolve_updates(
            &inst,
            &DriverExtra::Bambu(BambuExtra {
                ams: Some(ams),
                ..Default::default()
            }),
            &lib(),
        );
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].slot_idx, 1);
        // Exact match miss → generic PLA (untagged or unknown SKU
        // both land in the same fallback bucket).
        assert_eq!(updates[0].filament_identity.as_deref(), Some("generic-pla"));
    }

    #[test]
    fn bambu_empty_trays_clear_their_slots() {
        // Destructive sync: a tray the printer reports as empty clears
        // the slot's binding + color, rather than leaving a stale spool.
        let inst = bambi();
        let ams = ams_with_trays(vec![empty_tray(0), empty_tray(1)]);
        let updates = resolve_updates(
            &inst,
            &DriverExtra::Bambu(BambuExtra {
                ams: Some(ams),
                ..Default::default()
            }),
            &lib(),
        );
        assert_eq!(
            updates.len(),
            2,
            "both empty trays produce a clearing update"
        );
        for (i, u) in updates.iter().enumerate() {
            assert_eq!(u.slot_idx, i);
            assert_eq!(u.filament_identity, None);
            assert_eq!(u.color, None);
        }
    }

    #[test]
    fn bambu_tag_uid_round_trips_onto_the_slot_update() {
        // An RFID-tagged tray carries its tag_uid into the update so the
        // slot persists it; a manually-set (untagged) tray syncs as
        // tag_uid = None and stays user-editable. (resolve_updates owns
        // the report→update mapping; apply_from_driver copies the field
        // verbatim onto the slot.)
        use crate::core::driver::status::rfid_detected;
        let inst = bambi();
        let ams = ams_with_trays(vec![
            tray_with_tag(0, Some("GFA00"), "PLA", "ABCDEFFF", "F1E2D3C4B5A60718"),
            tray_with(1, Some("GFA00"), "PLA", "112233FF"),
        ]);
        let updates = resolve_updates(
            &inst,
            &DriverExtra::Bambu(BambuExtra {
                ams: Some(ams),
                ..Default::default()
            }),
            &lib(),
        );
        let rfid = &updates[0];
        assert_eq!(rfid.tag_uid.as_deref(), Some("F1E2D3C4B5A60718"));
        assert!(rfid_detected(rfid.tag_uid.as_deref()));
        let manual = &updates[1];
        assert_eq!(manual.tag_uid, None);
        assert!(!rfid_detected(manual.tag_uid.as_deref()));
    }

    #[test]
    fn bambu_ams_path_never_writes_into_the_trailing_direct_slot() {
        // Tray 4 on a 4-tray AMS shouldn't exist, but synthesize one
        // anyway to verify the guard — the AMS branch must never
        // touch a Direct slot. The Direct (Ext) slot is owned
        // exclusively by `external_spool` (vt_tray).
        let inst = bambi();
        let ams = ams_with_trays(vec![tray_with(4, Some("GFA00"), "PLA", "ABCDEFFF")]);
        let updates = resolve_updates(
            &inst,
            &DriverExtra::Bambu(BambuExtra {
                ams: Some(ams),
                ..Default::default()
            }),
            &lib(),
        );
        assert!(updates.is_empty());
    }

    fn ext_spool(filament_id: Option<&str>, material: &str, color: &str) -> AmsFilament {
        AmsFilament {
            tray_type: material.into(),
            color: color.into(),
            sub_brand: None,
            multi_colors: vec![],
            filament_id: filament_id.map(str::to_owned),
            tag_uid: None,
        }
    }

    #[test]
    fn bambu_external_spool_writes_into_first_direct_slot() {
        // Bambu's external (PTFE-tube) spool has no RFID, but the
        // printer still holds the user-entered material + color and
        // pushes them via vt_tray. Sync routes that into the
        // trailing Direct slot (slot 4 on the A1 mini).
        let inst = bambi();
        let updates = resolve_updates(
            &inst,
            &DriverExtra::Bambu(BambuExtra {
                external_spool: Some(ext_spool(None, "PETG", "112233FF")),
                ..Default::default()
            }),
            &lib(),
        );
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].extruder_idx, 0);
        assert_eq!(updates[0].slot_idx, 4);
        assert_eq!(
            updates[0].filament_identity.as_deref(),
            Some("generic-petg")
        );
        assert_eq!(updates[0].color.as_deref(), Some("#112233"));
    }

    #[test]
    fn bambu_external_spool_with_filament_id_resolves_exactly() {
        // Rare case (Bambu Ext doesn't ship RFID), but if the user
        // pulls a tagged spool from the AMS and runs it externally
        // the printer can still push filament_id. Exact match wins
        // over the generic fallback.
        let inst = bambi();
        let updates = resolve_updates(
            &inst,
            &DriverExtra::Bambu(BambuExtra {
                external_spool: Some(ext_spool(Some("GFA00"), "PLA", "FF8800FF")),
                ..Default::default()
            }),
            &lib(),
        );
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].slot_idx, 4);
        assert_eq!(
            updates[0].filament_identity.as_deref(),
            Some("bambu-pla-basic-bbl-a1m")
        );
    }

    #[test]
    fn bambu_external_spool_alongside_ams_emits_both_updates() {
        let inst = bambi();
        let ams = ams_with_trays(vec![tray_with(0, Some("GFA00"), "PLA", "FF0000FF")]);
        let updates = resolve_updates(
            &inst,
            &DriverExtra::Bambu(BambuExtra {
                ams: Some(ams),
                external_spool: Some(ext_spool(None, "PETG", "AABBCCFF")),
                ..Default::default()
            }),
            &lib(),
        );
        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0].slot_idx, 0);
        assert_eq!(updates[1].slot_idx, 4);
    }

    #[test]
    fn u1_matching_material_keeps_current_identity() {
        let mut inst = snappy();
        inst.extruders[0].slots[0].filament_identity = Some("bambu-pla-basic-bbl-a1m".into());
        let extra = U1Extra {
            toolhead_filaments: vec![Some(U1Filament {
                material_type: "PLA".into(),
                color: "224466FF".into(),
            })],
            ..Default::default()
        };
        let updates = resolve_updates(&inst, &DriverExtra::U1(extra), &lib());
        assert_eq!(updates.len(), 1);
        // Same identity returned — keeps the user's pick when
        // material matches.
        assert_eq!(
            updates[0].filament_identity.as_deref(),
            Some("bambu-pla-basic-bbl-a1m")
        );
        assert_eq!(updates[0].color.as_deref(), Some("#224466"));
    }

    #[test]
    fn u1_mismatched_material_resets_to_generic() {
        let mut inst = snappy();
        inst.extruders[0].slots[0].filament_identity = Some("bambu-pla-basic-bbl-a1m".into());
        let extra = U1Extra {
            toolhead_filaments: vec![Some(U1Filament {
                material_type: "PETG".into(),
                color: "00AAFFFF".into(),
            })],
            ..Default::default()
        };
        let updates = resolve_updates(&inst, &DriverExtra::U1(extra), &lib());
        assert_eq!(updates.len(), 1);
        assert_eq!(
            updates[0].filament_identity.as_deref(),
            Some("generic-petg")
        );
    }

    #[test]
    fn u1_mismatched_pla_resolves_to_plain_generic_not_silk_or_cf() {
        // Regression for the bug where U1 sync swapped ASA → "Generic
        // PLA Silk" because the resolver picked the first
        // vendor=Generic + base_type=PLA fragment it found. The
        // fixture list deliberately puts `generic-pla-silk` and
        // `generic-pla-cf` before `generic-pla`.
        let mut inst = snappy();
        inst.extruders[0].slots[0].filament_identity = Some("generic-asa".into());
        let extra = U1Extra {
            toolhead_filaments: vec![Some(U1Filament {
                material_type: "PLA".into(),
                color: "FFFFFFFF".into(),
            })],
            ..Default::default()
        };
        let updates = resolve_updates(&inst, &DriverExtra::U1(extra), &lib());
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].filament_identity.as_deref(), Some("generic-pla"));
    }

    #[test]
    fn u1_mismatched_material_with_no_base_generic_clears_identity() {
        // PA has only specialty variants (generic-pa-cf) in the
        // catalog — no plain generic-pa. Sync shouldn't silently
        // pick the CF variant; it should leave the slot
        // identity-less so the mismatch is visible.
        let mut inst = snappy();
        inst.extruders[0].slots[0].filament_identity = Some("generic-pla".into());
        let extra = U1Extra {
            toolhead_filaments: vec![Some(U1Filament {
                material_type: "PA".into(),
                color: "112233FF".into(),
            })],
            ..Default::default()
        };
        let updates = resolve_updates(&inst, &DriverExtra::U1(extra), &lib());
        assert_eq!(updates.len(), 1);
        assert!(updates[0].filament_identity.is_none());
        // Color always updates regardless of identity resolution.
        assert_eq!(updates[0].color.as_deref(), Some("#112233"));
    }

    #[test]
    fn u1_per_toolhead_indices_map_directly_to_extruder() {
        let inst = snappy();
        let extra = U1Extra {
            toolhead_filaments: vec![
                None,
                Some(U1Filament {
                    material_type: "PLA".into(),
                    color: "010203FF".into(),
                }),
                None,
                Some(U1Filament {
                    material_type: "PETG".into(),
                    color: "040506FF".into(),
                }),
            ],
            ..Default::default()
        };
        let updates = resolve_updates(&inst, &DriverExtra::U1(extra), &lib());
        // Every reported toolhead syncs: mounted ones write, the two
        // unmounted ones clear (destructive sync). Toolhead index maps
        // directly to extruder index.
        assert_eq!(updates.len(), 4);
        assert_eq!(
            updates.iter().map(|u| u.extruder_idx).collect::<Vec<_>>(),
            vec![0, 1, 2, 3]
        );
        assert_eq!(updates[0].filament_identity, None);
        assert_eq!(updates[0].color, None);
        assert!(updates[1].filament_identity.is_some());
        assert_eq!(updates[1].color.as_deref(), Some("#010203"));
        assert_eq!(updates[2].filament_identity, None);
        assert_eq!(updates[3].color.as_deref(), Some("#040506"));
    }

    #[test]
    fn hex8_strips_alpha_and_lowercases() {
        assert_eq!(hex8_to_css("FFAA00FF"), "#ffaa00");
        assert_eq!(hex8_to_css("#aabbcc"), "#aabbcc");
    }
}
