//! Mutable in-memory registry of [`PrinterInstance`]s.
//!
//! Production loads from the on-disk user library
//! ([`instance_storage`]) at first access; first launch starts
//! empty. User mutations (slot → filament bindings, nozzle swaps,
//! connection settings, printer add / remove) land here and
//! persist back to disk through `instance_storage::persist`.
//!
//! Tests that never wire a storage root fall back to the in-memory
//! fixtures from [`super::instance_library::bundled_instances`] —
//! bambi + snappy — so the wide existing test surface doesn't need
//! per-test temp-library plumbing.
//!
//! Storage shape: `Mutex<Vec<PrinterInstance>>` — small set, linear
//! scan is fine and preserves insertion order for the picker. The
//! `OnceLock` guards first-access load; subsequent calls lock +
//! access.

use std::sync::{Mutex, OnceLock};

use uuid::Uuid;

use super::instance::{
    BedRef, ConnectionInfo, ExtruderState, FeedKind, NozzleMaterial, NozzleSku, PrinterInstance,
    SlotBinding,
};
#[cfg(any(test, feature = "test-fixtures"))]
use super::instance_library::bundled_instances;
use super::instance_storage;
use crate::core::driver::traits::DriverKind;

static REGISTRY: OnceLock<Mutex<Vec<PrinterInstance>>> = OnceLock::new();

fn registry() -> &'static Mutex<Vec<PrinterInstance>> {
    REGISTRY.get_or_init(|| {
        // First-access load. Production has a storage root registered
        // by Tauri's `setup()` and starts empty on first launch (the
        // empty-state UI fires; create_instance writes the first
        // entry), so the `None` arm never fires in a release build — it
        // returns an empty registry, never fabricated printers. Under
        // the `test-fixtures` feature (and the lib's own `cfg(test)`
        // unit tests), the wide test surface that never calls Tauri's
        // setup falls back to the in-memory fixtures.
        let initial = match instance_storage::root() {
            Some(root) => instance_storage::load_from_disk(root).unwrap_or_else(|e| {
                tracing::warn!(
                    error = %e,
                    "instance library load failed; starting with an empty registry",
                );
                Vec::new()
            }),
            #[cfg(any(test, feature = "test-fixtures"))]
            None => bundled_instances(),
            #[cfg(not(any(test, feature = "test-fixtures")))]
            None => Vec::new(),
        };
        Mutex::new(initial)
    })
}

/// Snapshot the entire registry. Returns cloned `PrinterInstance`s in
/// insertion order — safe for the frontend picker to render against
/// without holding the mutex.
pub fn list_instances() -> Vec<PrinterInstance> {
    registry()
        .lock()
        .expect("printer instance registry poisoned")
        .clone()
}

/// Look up an instance by id. Returns a clone — mutations go through
/// [`mutate_instance`].
pub fn lookup_instance(id: &str) -> Option<PrinterInstance> {
    registry()
        .lock()
        .expect("printer instance registry poisoned")
        .iter()
        .find(|i| i.id == id)
        .cloned()
}

/// Per-instance failure modes for the [`mutate_instance`] surface.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InstanceMutError {
    /// `id` doesn't match any registered instance.
    #[error("unknown printer instance `{id}`")]
    UnknownInstance { id: String },
    /// Extruder index out of range for the instance's topology.
    #[error("instance `{instance_id}` has {extruders} extruder(s); index {extruder_idx} is out of range")]
    BadExtruder {
        instance_id: String,
        extruder_idx: usize,
        extruders: usize,
    },
    /// Slot index out of range for the chosen extruder.
    #[error("instance `{instance_id}` extruder {extruder_idx} has {slots} slot(s); index {slot_idx} is out of range")]
    BadSlot {
        instance_id: String,
        extruder_idx: usize,
        slot_idx: usize,
        slots: usize,
    },
    /// Requested bed isn't in the instance's bound printer profile's
    /// `supported_build_plates`. Surfaced by [`set_instance_bed`]; the
    /// picker UI recovers by re-rendering the selector against the
    /// fresh supported list.
    #[error("instance `{instance_id}` printer `{printer_identity}` does not support build plate `{bed_identity}`")]
    UnsupportedBuildPlate {
        instance_id: String,
        printer_identity: String,
        bed_identity: String,
    },
    /// The instance's `vendor_profile_ref` doesn't resolve in the
    /// bundled printer catalog. Should be impossible for fixtures
    /// shipped in-tree, but a hand-edited on-disk instance file
    /// could trip this.
    #[error("instance `{instance_id}` references printer `{printer_identity}`, which is not in the bundled catalog")]
    PrinterProfileNotFound {
        instance_id: String,
        printer_identity: String,
    },
    /// `create_instance` was called with a vendor profile id not in
    /// the bundled catalog.
    #[error("no bundled printer with identity `{identity}`")]
    UnknownPrinterIdentity { identity: String },
    /// `create_instance`'s `ams_units` exceeds the printer profile's
    /// declared `ams_max`.
    #[error("printer `{identity}` supports at most {max} AMS unit(s); {requested} requested")]
    AmsCountExceeded {
        identity: String,
        requested: u32,
        max: u32,
    },
    /// `create_instance` was called with an empty display name.
    #[error("display name must not be empty")]
    EmptyDisplayName,
    /// Requested nozzle diameter isn't in the bound printer profile's
    /// `available_nozzle_diameters`. Surfaced by
    /// [`set_extruder_nozzle_diameter`]; the picker only offers
    /// diameters from that list so a typed error here means a hand-
    /// edited instance file or a future driver-side sync wrote
    /// something the catalog doesn't bundle.
    #[error("instance `{instance_id}` printer `{printer_identity}` does not bundle nozzle diameter `{diameter}`")]
    UnsupportedNozzleDiameter {
        instance_id: String,
        printer_identity: String,
        diameter: String,
    },
    /// A connection was written whose `ConnectionInfo` variant
    /// doesn't match the instance's printer `driver_kind` (e.g. a
    /// Bambu connection on a U1 instance). The UI gates this by
    /// driver kind, but the command boundary enforces it too so a
    /// hand-edited / secondary caller can't persist a connection the
    /// reconciler would drive with the wrong-kind driver. `expected`
    /// is the printer's declared driver kind (`"none"` when it ships
    /// no driver); `got` is the connection variant.
    #[error("instance `{instance_id}` printer `{printer_identity}` expects a `{expected}` connection; got `{got}`")]
    ConnectionDriverMismatch {
        instance_id: String,
        printer_identity: String,
        expected: &'static str,
        got: &'static str,
    },
    /// A connection's field *content* is unusable — empty host, a
    /// Bambu access code that isn't 8 hex chars, or a U1 port of 0. The
    /// settings form validates these in `connectionValidation.ts`,
    /// but the command boundary enforces them too so a hand-edited
    /// instance file or secondary caller can't persist a connection
    /// the reconciler would then drive into a doomed connect.
    #[error("instance `{instance_id}` connection is invalid: {message}")]
    InvalidConnection {
        instance_id: String,
        message: String,
    },
}

/// Apply `f` to the named instance under the registry lock. Returns
/// the cloned post-mutation state so callers can emit it on a
/// scene event without re-locking. Errors when the id is unknown.
pub fn mutate_instance<F>(id: &str, f: F) -> Result<PrinterInstance, InstanceMutError>
where
    F: FnOnce(&mut PrinterInstance) -> Result<(), InstanceMutError>,
{
    let mut guard = registry()
        .lock()
        .expect("printer instance registry poisoned");
    let inst = guard
        .iter_mut()
        .find(|i| i.id == id)
        .ok_or_else(|| InstanceMutError::UnknownInstance { id: id.to_owned() })?;
    f(inst)?;
    let cloned = inst.clone();
    // Persist while still under the registry lock so concurrent
    // mutations can't race to write stale state. A persist failure
    // is non-fatal: log and continue — in-memory state is still
    // authoritative for the rest of the session.
    if let Some(root) = instance_storage::root() {
        if let Err(e) = instance_storage::persist(root, &cloned) {
            tracing::warn!(
                instance_id = %cloned.id,
                error = %e,
                "instance mutation persist failed; in-memory state unchanged",
            );
        }
    }
    Ok(cloned)
}

/// Set (or clear, with `None`) a slot's bound filament identity.
pub fn set_slot_filament(
    id: &str,
    extruder_idx: usize,
    slot_idx: usize,
    filament_identity: Option<String>,
) -> Result<PrinterInstance, InstanceMutError> {
    mutate_slot(id, extruder_idx, slot_idx, |slot| {
        slot.filament_identity = filament_identity;
    })
}

/// Set (or clear, with `None`) a `plugin.<name>.<key>` entry in the
/// instance's `config_overrides` — the printer-instance tier of the plugin
/// activation/settings cascade (a per-printer default the project/plate
/// tiers override). The caller is responsible for restricting `key` to the
/// `plugin.*` namespace (the command layer does).
pub fn set_plugin_override(
    id: &str,
    key: String,
    value: Option<String>,
) -> Result<PrinterInstance, InstanceMutError> {
    set_config_override(id, key, value)
}

/// Set (or clear, with `None`) any entry in the instance's
/// `config_overrides`. The map is the printer-instance cascade tier; the
/// command layer restricts which key namespaces it accepts (`plugin.*`
/// for plugin settings, Printer-bucket keys for machine settings).
pub fn set_config_override(
    id: &str,
    key: String,
    value: Option<String>,
) -> Result<PrinterInstance, InstanceMutError> {
    mutate_instance(id, move |inst| {
        match value {
            Some(v) => {
                inst.config_overrides.insert(key, v);
            }
            None => {
                inst.config_overrides.remove(&key);
            }
        }
        Ok(())
    })
}

/// Set (or clear, with `None`) a slot's user-assigned spool color.
/// Hex string like `"#ff8800"`; the backend does not validate the
/// shape — the picker only ever writes well-formed values.
pub fn set_slot_color(
    id: &str,
    extruder_idx: usize,
    slot_idx: usize,
    color: Option<String>,
) -> Result<PrinterInstance, InstanceMutError> {
    mutate_slot(id, extruder_idx, slot_idx, |slot| {
        slot.color = color;
    })
}

/// Change the diameter of the nozzle currently installed on the named
/// extruder. Material stays as-is — the picker MVP only surfaces
/// diameter swaps.
///
/// Validates the new diameter against the bound printer profile's
/// `available_nozzle_diameters` so a hand-edited instance file or a
/// future driver-side sync that writes a diameter the catalog doesn't
/// bundle gets a typed error instead of an opaque slice-time failure.
pub fn set_extruder_nozzle_diameter(
    id: &str,
    extruder_idx: usize,
    diameter: String,
) -> Result<PrinterInstance, InstanceMutError> {
    mutate_instance(id, |inst| {
        let extruder_count = inst.extruders.len();
        let extruder =
            inst.extruders
                .get_mut(extruder_idx)
                .ok_or(InstanceMutError::BadExtruder {
                    instance_id: id.to_owned(),
                    extruder_idx,
                    extruders: extruder_count,
                })?;
        // Resolve the bound profile up front: it's the authority on
        // which diameters this printer accepts AND it carries the
        // model.toml string the picker's `available_for` predicates
        // key off (needed below for the rule-3 fallback). Missing the
        // profile is a structural error worth surfacing, not silently
        // overwriting `quality_profile` with the empty-string-model
        // fallback that an `unwrap_or_default` would produce.
        let profile = super::lookup(&inst.vendor_profile_ref).ok_or_else(|| {
            InstanceMutError::PrinterProfileNotFound {
                instance_id: id.to_owned(),
                printer_identity: inst.vendor_profile_ref.clone(),
            }
        })?;
        if !profile
            .available_nozzle_diameters
            .iter()
            .any(|d| d == &diameter)
        {
            return Err(InstanceMutError::UnsupportedNozzleDiameter {
                instance_id: id.to_owned(),
                printer_identity: inst.vendor_profile_ref.clone(),
                diameter,
            });
        }
        extruder.installed_nozzle.diameter = diameter.clone();

        // Quality-picker rule 3: when nozzle changes, if the
        // currently selected process is no longer compatible with
        // the new installed-nozzle set, fall back to the swapped-to
        // nozzle's `default_process_profile`. Compatibility is the
        // same union rule the picker uses
        // (`list_process_fragments`): a process matches when its
        // `available_for` nozzle spec (split on `+`) shares any
        // diameter with the installed set.
        let installed: Vec<String> = inst
            .extruders
            .iter()
            .map(|e| e.installed_nozzle.diameter.clone())
            .collect();
        let printer_slug = inst.printer_fragment_slug.clone();
        let compatible = crate::core::profile_library::list_process_fragments(
            &printer_slug,
            &profile.model,
            &installed,
        );
        let still_compatible = compatible.iter().any(|s| s.slug == inst.quality_profile);
        if !still_compatible {
            let fallback =
                crate::core::profile_library::nozzle_default_process(&printer_slug, &diameter);
            if let Some(slug) = fallback {
                inst.quality_profile = slug;
            }
            // If no fallback exists (nozzle.toml lacks
            // default_process_profile), leave the slug alone — the
            // picker will surface it as "unbound" (raw slug
            // displayed) and the user can pick another.
        }
        Ok(())
    })
}

/// Construct a fresh `PrinterInstance` from a bundled printer
/// identity + a user-chosen display name + AMS unit count. Inserts
/// the new instance into the registry and persists it (when the
/// storage root is set).
///
/// Topology: one extruder with `(ams_units * 4 + 1)` slots — the
/// AMS-fed slots come first, labelled `"AMS:1".."AMS:4"` (single
/// AMS) or `"AMS A:1".."AMS B:4"..` (multiple AMS units, letter-
/// disambiguated), and the last slot is `FeedKind::Direct` with
/// label `"Ext"`. When the printer profile's `ams_max == 0`
/// `ams_units` must also be `0`; the resulting instance has just
/// the one direct-fed slot.
///
/// AMS-first ordering matters: BBS's `ams_mapping[5]` /
/// `ams_mapping2[5]` arrays put AMS slots at filament indices
/// `0..3` and the external spool at index `4` (with the `-1` /
/// `{255,255}` sentinel). The libslic3r filament index is derived
/// directly from this slot ordering, so swapping the order shifts
/// every cube's tool-number by one and the firmware routes the
/// wrong AMS slot.
///
/// Defaults derived from the bundled profile + library:
///   - **Topology** branches on `profile.toolheads.len()`:
///     - `1` (AMS-style: single toolhead, optionally AMS-fed) — one
///       extruder with `(ams_units * 4 + 1)` slots; AMS-fed slots
///       come first (`FeedKind::Ams`), the trailing slot is
///       `FeedKind::Direct` ("Ext").
///     - `N > 1` (toolchanger: U1, XL) — N extruders labelled
///       `T1..TN` (1-based for display; the in-memory extruder
///       vector and gcode tool numbers stay 0-based), each with
///       one `FeedKind::Direct` slot. The
///       `ams_units` parameter is ignored for this branch; the
///       caller-side AMS picker is hidden when `ams_max == 0`.
///   - nozzle: each toolhead's `default_nozzle_diameter`, `Stainless`.
///   - bed: `profile.default_bed` (Orca's upstream `default_bed_type`)
///     when set + supported, else the first entry in
///     `supported_build_plates`.
///   - default process: first bundled process fragment for the
///     printer (deterministic by `BTreeMap` key order).
///   - default filament: the bound nozzle's upstream
///     `default_filament_profile` (e.g. `"Bambu PLA Basic @BBL A1M"`)
///     resolved to a fragment slug; every slot is pre-bound to it.
///     Falls back to `"generic-pla"` when the nozzle doesn't declare
///     a default or the named filament isn't in our library.
///
/// Validates: the printer identity exists in the catalog, AMS
/// count is within `ams_max`, display name is non-empty (after
/// trim). Returns the new instance for the caller to surface.
pub fn create_instance(
    printer_identity: &str,
    display_name: String,
    ams_units: u32,
) -> Result<PrinterInstance, InstanceMutError> {
    let display_name = display_name.trim().to_owned();
    if display_name.is_empty() {
        return Err(InstanceMutError::EmptyDisplayName);
    }
    let profile = super::lookup(printer_identity).ok_or_else(|| {
        InstanceMutError::UnknownPrinterIdentity {
            identity: printer_identity.to_owned(),
        }
    })?;
    if ams_units > profile.ams_max {
        return Err(InstanceMutError::AmsCountExceeded {
            identity: printer_identity.to_owned(),
            requested: ams_units,
            max: profile.ams_max,
        });
    }

    // Resolve the upstream `default_filament_profile` (a
    // `filament_settings_id` like `"Bambu PLA Basic @BBL A1M"`) for
    // a given nozzle SKU to a fragment slug. Fall back to
    // `generic-pla` when the nozzle doesn't declare one or the
    // named filament isn't in our library — every vendor ships a
    // Generic PLA fragment so the slice path always has *something*
    // to resolve against.
    let resolve_default_filament = |nozzle_diameter: &str| -> String {
        crate::core::profile_library::default_filament_profile_for(
            printer_identity,
            nozzle_diameter,
        )
        .and_then(|name| crate::core::profile_library::filament_slug_by_display_name(&name))
        .unwrap_or_else(|| "generic-pla".to_owned())
    };

    // Topology branches on toolhead count. AMS-style printers
    // (single toolhead) get one extruder with N+1 slots; toolchangers
    // (U1, XL) get one extruder *per toolhead*, each with a single
    // Direct feed.
    //
    // Labels are left empty here; `PrinterInstance::populate_labels`
    // fills them in at the end of construction (same call path the
    // disk-load uses, so there's a single source of truth).
    let extruders: Vec<ExtruderState> = if profile.toolheads.len() > 1 {
        // Toolchanger: one extruder per toolhead with a single
        // Direct slot. `ams_units` is ignored — the modal hides the
        // AMS picker for `ams_max == 0` printers, so a caller that
        // somehow passes >0 here gets the same topology (we already
        // validated `ams_units <= ams_max` above).
        profile
            .toolheads
            .iter()
            .map(|toolhead| {
                let filament_slug = resolve_default_filament(&toolhead.default_nozzle_diameter);
                ExtruderState {
                    installed_nozzle: NozzleSku {
                        diameter: toolhead.default_nozzle_diameter.clone(),
                        material: NozzleMaterial::Stainless,
                    },
                    slots: vec![SlotBinding {
                        feed: FeedKind::Direct,
                        filament_identity: Some(filament_slug),
                        color: None,
                        tag_uid: None,
                    }],
                }
            })
            .collect()
    } else {
        // AMS-style: one extruder, AMS-fed slots come first, the
        // direct/Ext spool is the trailing slot. AMS-first ordering
        // matches BBS's ams_mapping convention; see the doc comment
        // on `create_instance`.
        let toolhead = profile.toolheads.first();
        let default_nozzle_diameter: String = toolhead
            .map(|t| t.default_nozzle_diameter.clone())
            .unwrap_or_else(|| "0.4".to_owned());
        let filament_slug = resolve_default_filament(&default_nozzle_diameter);
        let mut slots = Vec::new();
        for _unit in 0..ams_units {
            for _slot in 0..super::instance::AMS_SLOTS_PER_UNIT {
                slots.push(SlotBinding {
                    feed: FeedKind::Ams,
                    filament_identity: Some(filament_slug.clone()),
                    color: None,
                    tag_uid: None,
                });
            }
        }
        slots.push(SlotBinding {
            feed: FeedKind::Direct,
            filament_identity: Some(filament_slug.clone()),
            color: None,
            tag_uid: None,
        });
        vec![ExtruderState {
            installed_nozzle: NozzleSku {
                diameter: default_nozzle_diameter,
                material: NozzleMaterial::Stainless,
            },
            slots,
        }]
    };

    // For the instance-level `default_filament_fragment_slug` we
    // pick the first toolhead's default; this is the fallback the
    // slicer composer uses for slots that lack their own binding.
    let default_filament_slug = profile
        .toolheads
        .first()
        .map(|t| resolve_default_filament(&t.default_nozzle_diameter))
        .unwrap_or_else(|| "generic-pla".to_owned());

    // Bed: prefer the upstream-declared default when it's actually
    // in the supported list; otherwise first-supported. The
    // `supported_build_plates` filter guards against a stale or
    // hand-edited `default_bed` pointing at a bed we don't ship.
    let bed_identity = profile
        .default_bed
        .as_ref()
        .filter(|id| profile.supported_build_plates.iter().any(|p| p == *id))
        .cloned()
        .or_else(|| profile.supported_build_plates.first().cloned())
        .unwrap_or_default();
    // Default process: ask the first toolhead's nozzle.toml what it
    // recommends (rule 1 in the Quality-picker design). If that
    // nozzle doesn't declare a default, fall back to whatever
    // process slug is registered first for this printer (HashMap
    // iteration order — non-deterministic, but better than empty).
    let first_nozzle_sku = profile
        .toolheads
        .first()
        .map(|t| t.default_nozzle_diameter.clone());
    let quality_profile = first_nozzle_sku
        .as_deref()
        .and_then(|sku| crate::core::profile_library::nozzle_default_process(printer_identity, sku))
        .unwrap_or_else(|| {
            // Falling back to HashMap iteration order — non-
            // deterministic, depends on hash seeds. Warn so the
            // gap (nozzle.toml missing `default_process_profile`)
            // doesn't stay silent: the user sees the wrong
            // process pre-selected and the picker offers no
            // remedy except "select something else."
            let fallback =
                crate::core::profile_library::bundled_process_slugs_for_printer(printer_identity)
                    .into_iter()
                    .next()
                    .unwrap_or("")
                    .to_owned();
            tracing::warn!(
                printer = %printer_identity,
                nozzle = first_nozzle_sku.as_deref().unwrap_or("<no toolhead>"),
                fallback_slug = %fallback,
                "nozzle.toml has no `default_process_profile`; \
                 picker will start on an unstable fallback slug",
            );
            fallback
        });

    let instance = PrinterInstance {
        id: Uuid::new_v4().to_string(),
        display_name,
        vendor_profile_ref: printer_identity.to_owned(),
        printer_fragment_slug: printer_identity.to_owned(),
        default_filament_fragment_slug: default_filament_slug,
        quality_profile,
        connection: None,
        extruders,
        bed: BedRef {
            identity: bed_identity,
        },
        config_overrides: Default::default(),
    };

    {
        let mut guard = registry()
            .lock()
            .expect("printer instance registry poisoned");
        guard.push(instance.clone());
    }
    if let Some(root) = instance_storage::root() {
        if let Err(e) = instance_storage::persist(root, &instance) {
            tracing::warn!(
                instance_id = %instance.id,
                error = %e,
                "instance create persist failed; in-memory state unchanged",
            );
        }
    }
    Ok(instance)
}

/// Remove the named instance from the registry + on-disk library.
/// Errors with `UnknownInstance` when the id doesn't match anything;
/// no-op cleanly for files already missing from disk.
pub fn delete_instance(id: &str) -> Result<(), InstanceMutError> {
    {
        let mut guard = registry()
            .lock()
            .expect("printer instance registry poisoned");
        let pos = guard
            .iter()
            .position(|i| i.id == id)
            .ok_or_else(|| InstanceMutError::UnknownInstance { id: id.to_owned() })?;
        guard.remove(pos);
    }
    if let Some(root) = instance_storage::root() {
        if let Err(e) = instance_storage::delete(root, id) {
            tracing::warn!(
                instance_id = id,
                error = %e,
                "instance delete on-disk cleanup failed; in-memory state already gone",
            );
        }
    }
    Ok(())
}

/// Set the instance's currently-loaded build plate. Validates the new
/// identity against the bound printer profile's `supported_build_plates`
/// — the printer profile is the authority on what plates a given
/// printer accepts. After this, the slicer composer + every other reader
/// can trust `instance.bed.identity` blindly.
pub fn set_instance_bed(
    id: &str,
    bed_identity: String,
) -> Result<PrinterInstance, InstanceMutError> {
    mutate_instance(id, |inst| {
        let profile = super::lookup(&inst.vendor_profile_ref).ok_or_else(|| {
            InstanceMutError::PrinterProfileNotFound {
                instance_id: id.to_owned(),
                printer_identity: inst.vendor_profile_ref.clone(),
            }
        })?;
        if !profile
            .supported_build_plates
            .iter()
            .any(|p| p == &bed_identity)
        {
            return Err(InstanceMutError::UnsupportedBuildPlate {
                instance_id: id.to_owned(),
                printer_identity: inst.vendor_profile_ref.clone(),
                bed_identity,
            });
        }
        inst.bed.identity = bed_identity;
        Ok(())
    })
}

/// Change the AMS-unit count on an AMS-style printer. Re-derives the
/// topology of `extruders[0].slots` to `(ams_units * 4 + 1)` —
/// matching `create_instance`'s layout — and preserves existing
/// bindings positionally where the new range overlaps the old one.
///
/// Bindings on slots that no longer exist (after a shrink) are
/// silently dropped after a `tracing::warn!` per drop so the user
/// can audit the loss. Slots added on a grow are seeded with the
/// instance's `default_filament_fragment_slug` (same as
/// `create_instance` does on first creation).
///
/// Validates:
///   - The bound profile resolves (else `PrinterProfileNotFound`).
///   - The printer is AMS-style (`toolheads.len() == 1` and
///     `ams_max > 0`) — toolchanger printers don't have AMS-driven
///     topology, so the call is rejected with `AmsCountExceeded`
///     citing the `ams_max == 0` constraint.
///   - `ams_units <= profile.ams_max` (else `AmsCountExceeded`).
pub fn set_instance_ams_units(
    id: &str,
    ams_units: u32,
) -> Result<PrinterInstance, InstanceMutError> {
    mutate_instance(id, |inst| apply_ams_units(inst, id, ams_units, None))
}

/// Write the instance's network connection settings (or clear them
/// with `None`). Persisted via `instance_storage` so the same
/// physical printer's connection survives across app restarts.
/// The reactive driver-registry manager
/// (`src/driver/useDriverConnections.ts`) observes the resulting
/// `printer:instance_changed` event and reconciles registered
/// drivers against the new state — register/disconnect/replace
/// happen automatically; no separate UI gesture needed.
pub fn set_instance_connection(
    id: &str,
    connection: Option<ConnectionInfo>,
) -> Result<PrinterInstance, InstanceMutError> {
    mutate_instance(id, |inst| {
        if let Some(conn) = &connection {
            validate_connection(inst, conn)?;
        }
        inst.connection = connection.clone();
        Ok(())
    })
}

/// Variant of a `ConnectionInfo` as a lowercase wire token, matching
/// the `DriverKind` serialization.
fn connection_kind_token(conn: &ConnectionInfo) -> &'static str {
    match conn {
        ConnectionInfo::Bambu { .. } => "bambu",
        ConnectionInfo::U1 { .. } => "u1",
        ConnectionInfo::Moonraker { .. } => "moonraker",
    }
}

fn driver_kind_token(kind: Option<DriverKind>) -> &'static str {
    match kind {
        Some(DriverKind::Bambu) => "bambu",
        Some(DriverKind::U1) => "u1",
        Some(DriverKind::Moonraker) => "moonraker",
        None => "none",
    }
}

/// Reject a `ConnectionInfo` whose variant doesn't match the
/// instance's printer `driver_kind` (authored in `model.toml`). This
/// is the *kind* half of the command-boundary check; field content is
/// validated separately in `validate_connection_content`, and both
/// run via `validate_connection`. The settings modal already gates
/// connection edits by driver kind, but the command boundary enforces
/// it too so a hand-edited instance file or any future secondary
/// caller can't persist a connection the reconciler would then drive
/// with the wrong-kind driver. Looks the profile up the same way
/// `set_instance_bed` / `set_extruder_nozzle_diameter` do — a missing
/// profile is the structural `PrinterProfileNotFound`, not a silent
/// pass.
fn validate_connection_kind(
    inst: &PrinterInstance,
    conn: &ConnectionInfo,
) -> Result<(), InstanceMutError> {
    let profile = super::lookup(&inst.vendor_profile_ref).ok_or_else(|| {
        InstanceMutError::PrinterProfileNotFound {
            instance_id: inst.id.clone(),
            printer_identity: inst.vendor_profile_ref.clone(),
        }
    })?;
    let ok = matches!(
        (profile.driver_kind, conn),
        (Some(DriverKind::Bambu), ConnectionInfo::Bambu { .. })
            | (Some(DriverKind::U1), ConnectionInfo::U1 { .. })
            | (Some(DriverKind::Moonraker), ConnectionInfo::Moonraker { .. })
    );
    if !ok {
        return Err(InstanceMutError::ConnectionDriverMismatch {
            instance_id: inst.id.clone(),
            printer_identity: inst.vendor_profile_ref.clone(),
            expected: driver_kind_token(profile.driver_kind),
            got: connection_kind_token(conn),
        });
    }
    Ok(())
}

/// Reject a connection whose field *content* is unusable. Mirrors the
/// frontend `connectionValidation.ts` rules (non-empty host; Bambu
/// access code = 8 hex chars; U1 port in 1..=65535 — `u16` already caps
/// the upper bound, so only 0 is out of range) so the picker dot and
/// this command boundary agree on what "valid" means.
fn validate_connection_content(
    inst: &PrinterInstance,
    conn: &ConnectionInfo,
) -> Result<(), InstanceMutError> {
    let invalid = |message: &str| InstanceMutError::InvalidConnection {
        instance_id: inst.id.clone(),
        message: message.to_owned(),
    };
    match conn {
        ConnectionInfo::Bambu {
            host, access_code, ..
        } => {
            if host.trim().is_empty() {
                return Err(invalid("host is required"));
            }
            let code = access_code.trim();
            if code.len() != 8 || !code.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(invalid("access code must be 8 characters, 0-9 or a-f"));
            }
        }
        ConnectionInfo::U1 { host, port, .. }
        | ConnectionInfo::Moonraker { host, port, .. } => {
            if host.trim().is_empty() {
                return Err(invalid("host is required"));
            }
            if *port == 0 {
                return Err(invalid("port must be between 1 and 65535"));
            }
        }
    }
    Ok(())
}

/// Full command-boundary connection validation: the variant must
/// match the printer's `driver_kind` AND the field content must be
/// usable. Both `set_instance_connection` and `update_instance` route
/// every persisted connection through this.
fn validate_connection(
    inst: &PrinterInstance,
    conn: &ConnectionInfo,
) -> Result<(), InstanceMutError> {
    validate_connection_kind(inst, conn)?;
    validate_connection_content(inst, conn)?;
    Ok(())
}

/// Patch shape for the composite `update_instance` mutator. Each
/// field is `Option`-wrapped: `None` = leave unchanged; `Some` =
/// apply. The outer `Option<ConnectionInfo>` (for `connection`)
/// has its own `Some(None)` semantics: explicitly clear the
/// saved connection. Serde `flatten` on the wire would conflate
/// these states, so the Tauri command takes a dedicated struct
/// with a `clear_connection: bool` companion flag.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct InstancePatch {
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub ams_units: Option<u32>,
    /// New connection to install (only consulted when
    /// `clear_connection` is false). Leaving both fields default
    /// leaves the connection untouched.
    #[serde(default)]
    pub connection: Option<super::instance::ConnectionInfo>,
    /// Set the persisted connection to `None` regardless of
    /// `connection`. Mutually exclusive with passing a non-null
    /// `connection`; if both are present, `clear_connection`
    /// wins.
    #[serde(default)]
    pub clear_connection: bool,
}

/// Apply a multi-field patch atomically. Single registry lock,
/// single persist, single `printer:instance_changed` emit. The
/// settings modal collapses its three sequential mutators
/// (display_name + ams_units + connection) into this one IPC.
///
/// Atomic: ALL validation (name, AMS request, connection kind +
/// content) runs against a read-only snapshot BEFORE any field is
/// mutated. Only once everything is known-good does the mutating
/// closure assign the fields. This matters because `mutate_instance`
/// isn't transactional across fields — a validation error mid-closure
/// would otherwise strand an earlier in-memory mutation (e.g. the
/// rename) that never gets persisted, diverging memory from disk.
///
/// Applied fields:
///   1. Display name (whitespace-trimmed, non-empty).
///   2. AMS units (re-derives slot topology).
///   3. Connection (replaced wholesale, or cleared if
///      `clear_connection` is set).
pub fn update_instance(
    id: &str,
    patch: InstancePatch,
) -> Result<PrinterInstance, InstanceMutError> {
    let trimmed_name = patch.display_name.as_deref().map(str::trim);
    if matches!(trimmed_name, Some(s) if s.is_empty()) {
        return Err(InstanceMutError::EmptyDisplayName);
    }
    let trimmed_name = trimmed_name.map(str::to_owned);

    // Validate the fallible fields against a read-only snapshot before
    // mutating anything. vendor_profile_ref and the bound profile are
    // immutable, so validating against the snapshot is sound even
    // though `mutate_instance` re-locks the registry afterwards.
    let validating_connection = !patch.clear_connection && patch.connection.is_some();
    if patch.ams_units.is_some() || validating_connection {
        let inst = lookup_instance(id)
            .ok_or_else(|| InstanceMutError::UnknownInstance { id: id.to_owned() })?;
        if let Some(ams_units) = patch.ams_units {
            validate_ams_request(&inst, id, ams_units, None)?;
        }
        if let Some(conn) = &patch.connection {
            if !patch.clear_connection {
                validate_connection(&inst, conn)?;
            }
        }
    }

    mutate_instance(id, |inst| {
        if let Some(name) = trimmed_name {
            inst.display_name = name;
        }
        if let Some(ams_units) = patch.ams_units {
            rebuild_ams_slots(inst, id, ams_units);
        }
        if patch.clear_connection {
            inst.connection = None;
        } else if let Some(conn) = patch.connection {
            inst.connection = Some(conn);
        }
        Ok(())
    })
}

/// Validate an AMS-units request against the bound profile — AMS-style
/// printer (`toolheads.len() == 1` and `ams_max > 0`) and the count
/// within `ams_max`. No mutation; pairs with [`rebuild_ams_slots`] so
/// `update_instance` can run all validation before mutating anything.
/// `pub(crate)` so the driver-sync reconcile (`sync::apply_from_driver`)
/// can gate an AMS-count change on the same eligibility/bound rule
/// without re-deriving the predicate.
pub(crate) fn validate_ams_request(
    inst: &PrinterInstance,
    id: &str,
    ams_units: u32,
    profile: Option<&crate::core::printer::profile::PrinterProfile>,
) -> Result<(), InstanceMutError> {
    let profile = match profile {
        Some(p) => p,
        None => &super::lookup(&inst.vendor_profile_ref).ok_or_else(|| {
            InstanceMutError::PrinterProfileNotFound {
                instance_id: id.to_owned(),
                printer_identity: inst.vendor_profile_ref.clone(),
            }
        })?,
    };
    if profile.toolheads.len() != 1 || profile.ams_max == 0 || ams_units > profile.ams_max {
        return Err(InstanceMutError::AmsCountExceeded {
            identity: inst.vendor_profile_ref.clone(),
            requested: ams_units,
            max: profile.ams_max,
        });
    }
    Ok(())
}

/// Rebuild `inst.extruders[0].slots` for `ams_units` AMS units (each
/// `AMS_SLOTS_PER_UNIT` slots) plus one trailing `Direct` slot.
/// Infallible — the caller MUST have already passed
/// [`validate_ams_request`]. Preserves overlapping bindings by feed
/// kind; seeds new slots from the instance's default filament.
/// `pub(crate)` so `sync::apply_from_driver` can reconcile the AMS
/// topology inside its own `mutate_instance` closure.
pub(crate) fn rebuild_ams_slots(inst: &mut PrinterInstance, id: &str, ams_units: u32) {
    let target_slot_count = (ams_units as usize) * super::instance::AMS_SLOTS_PER_UNIT + 1;
    // An AMS-style instance is expected to have at least one extruder,
    // but a hand-edited / corrupt instance file might not — no-op
    // rather than panic, since there's no extruder to rebuild slots on.
    let Some(extruder) = inst.extruders.get_mut(0) else {
        return;
    };
    let current_slot_count = extruder.slots.len();
    if current_slot_count == target_slot_count {
        return;
    }
    let default_slug = Some(inst.default_filament_fragment_slug.clone());
    let mut new_slots: Vec<SlotBinding> = Vec::with_capacity(target_slot_count);
    for new_idx in 0..target_slot_count {
        let last = new_idx + 1 == target_slot_count;
        let target_feed = if last {
            FeedKind::Direct
        } else {
            FeedKind::Ams
        };
        let source_idx = if last {
            current_slot_count.checked_sub(1)
        } else if new_idx < current_slot_count.saturating_sub(1) {
            Some(new_idx)
        } else {
            None
        };
        let slot = match source_idx.and_then(|i| extruder.slots.get(i)) {
            Some(prior) if prior.feed == target_feed => prior.clone(),
            _ => SlotBinding {
                feed: target_feed,
                filament_identity: default_slug.clone(),
                color: None,
                tag_uid: None,
            },
        };
        new_slots.push(slot);
    }
    if current_slot_count > target_slot_count {
        let dropped = current_slot_count - target_slot_count;
        tracing::warn!(
            instance_id = %id,
            dropped_slots = dropped,
            from = current_slot_count,
            to = target_slot_count,
            "ams_units shrink dropped slot bindings",
        );
    }
    extruder.slots = new_slots;
}

/// Validate + rebuild in one step. Used by `set_instance_ams_units`,
/// which mutates a single field so there's no cross-field atomicity to
/// preserve. `update_instance` instead calls the two halves separately
/// so all validation runs before any field is mutated.
fn apply_ams_units(
    inst: &mut PrinterInstance,
    id: &str,
    ams_units: u32,
    profile: Option<&crate::core::printer::profile::PrinterProfile>,
) -> Result<(), InstanceMutError> {
    validate_ams_request(inst, id, ams_units, profile)?;
    rebuild_ams_slots(inst, id, ams_units);
    Ok(())
}

/// Rename the instance. Mirrors `create_instance`'s validation:
/// trims whitespace and rejects empty.
pub fn set_instance_display_name(
    id: &str,
    display_name: String,
) -> Result<PrinterInstance, InstanceMutError> {
    let trimmed = display_name.trim().to_owned();
    if trimmed.is_empty() {
        return Err(InstanceMutError::EmptyDisplayName);
    }
    mutate_instance(id, |inst| {
        inst.display_name = trimmed.clone();
        Ok(())
    })
}

/// Range-check the `(extruder_idx, slot_idx)` pair and hand the
/// mutable slot to `f`. Shared backbone for `set_slot_filament` /
/// `set_slot_color` so the OOB error mapping lives in one place.
fn mutate_slot<F>(
    id: &str,
    extruder_idx: usize,
    slot_idx: usize,
    f: F,
) -> Result<PrinterInstance, InstanceMutError>
where
    F: FnOnce(&mut super::instance::SlotBinding),
{
    mutate_instance(id, |inst| {
        let extruder_count = inst.extruders.len();
        let extruder =
            inst.extruders
                .get_mut(extruder_idx)
                .ok_or(InstanceMutError::BadExtruder {
                    instance_id: id.to_owned(),
                    extruder_idx,
                    extruders: extruder_count,
                })?;
        let slot_count = extruder.slots.len();
        let slot = extruder
            .slots
            .get_mut(slot_idx)
            .ok_or(InstanceMutError::BadSlot {
                instance_id: id.to_owned(),
                extruder_idx,
                slot_idx,
                slots: slot_count,
            })?;
        f(slot);
        Ok(())
    })
}

// ─────────────────────────────────────────────────────────────────────
// Test-only serialization scaffolding.
//
// The registry is a process-global `Mutex<Vec<PrinterInstance>>`. The
// per-call Mutex serializes individual operations cleanly, but it
// CAN'T enforce sequence-level invariants like "reset, then mutate,
// then read what we wrote" because parallel tests can reset between
// my mutate and my read.
//
// [`RegistryGuard`] solves this by acquiring a separate process-wide
// `TEST_LOCK` for the test's entire body, then resetting on both
// acquire and drop so the next test starts from the same baseline.
// Every test that touches the registry MUST take this guard — the
// only way to reset the registry is through the guard, so any test
// that skips it can't influence (or be influenced by) tests that
// do.
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// RAII guard that serializes a test's registry access. Acquire at
/// the top of any test body that calls into this module — reads or
/// writes alike. The lock is released on drop along with another
/// `reset_to_bundled_inner()` so the next test sees a clean baseline
/// regardless of failure path.
///
/// Idiom:
/// ```ignore
/// #[test]
/// fn my_test() {
///     let _registry = RegistryGuard::acquire();
///     // … test body …
/// }
/// ```
#[cfg(test)]
pub struct RegistryGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
}

#[cfg(test)]
impl RegistryGuard {
    /// Take the test-wide lock + reset the registry to bundled
    /// fixtures. Blocks until any concurrent guarded test releases
    /// its lock. Poison-tolerant: a previous test that panicked
    /// while holding the lock doesn't wedge subsequent tests.
    pub fn acquire() -> Self {
        let lock = TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        reset_to_bundled_inner();
        Self { _lock: lock }
    }
}

#[cfg(test)]
impl Drop for RegistryGuard {
    fn drop(&mut self) {
        // Reset so the next test starts clean even if this one left
        // the registry mid-mutation.
        reset_to_bundled_inner();
    }
}

/// Inner reset — does NOT take the test lock. Only called by
/// `RegistryGuard::acquire` and `RegistryGuard::drop` (each already
/// holds `TEST_LOCK`).
#[cfg(test)]
fn reset_to_bundled_inner() {
    let mut guard = registry().lock().expect("registry poisoned");
    *guard = bundled_instances();
}

#[cfg(test)]
mod tests {
    use super::super::instance::{FeedKind, SlotBinding};
    use super::*;

    #[test]
    fn list_returns_bundled_set() {
        let _registry = RegistryGuard::acquire();
        let instances = list_instances();
        assert!(instances.iter().any(|i| i.id == "bambi"));
        assert!(instances.iter().any(|i| i.id == "snappy"));
    }

    #[test]
    fn lookup_unknown_returns_none() {
        let _registry = RegistryGuard::acquire();
        assert!(lookup_instance("ghost").is_none());
    }

    #[test]
    fn set_slot_filament_mutates_in_place_and_persists_across_lookups() {
        let _registry = RegistryGuard::acquire();
        let updated = set_slot_filament("bambi", 0, 0, Some("Generic PLA".into()))
            .expect("bambi extruder 0 slot 0 exists");
        assert_eq!(
            updated.extruders[0].slots[0].filament_identity,
            Some("Generic PLA".into()),
        );
        // A second lookup must see the same value — confirms the
        // mutation actually landed in the registry, not just on the
        // returned clone.
        let again = lookup_instance("bambi").expect("bambi present");
        assert_eq!(
            again.extruders[0].slots[0].filament_identity,
            Some("Generic PLA".into()),
        );
        // Clear path.
        let cleared = set_slot_filament("bambi", 0, 0, None).expect("clear should succeed");
        assert_eq!(cleared.extruders[0].slots[0].filament_identity, None);
    }

    #[test]
    fn set_slot_filament_errors_on_unknown_instance() {
        let _registry = RegistryGuard::acquire();
        let err = set_slot_filament("ghost", 0, 0, Some("PLA".into())).unwrap_err();
        assert!(matches!(err, InstanceMutError::UnknownInstance { .. }));
    }

    #[test]
    fn set_plugin_override_sets_and_clears_in_config_overrides() {
        let _registry = RegistryGuard::acquire();
        let updated = set_plugin_override(
            "bambi",
            "plugin.platecycler.enabled".into(),
            Some("false".into()),
        )
        .expect("bambi exists");
        assert_eq!(
            updated.config_overrides.get("plugin.platecycler.enabled"),
            Some(&"false".to_string()),
        );
        // Persisted into the registry, not just the returned clone.
        let again = lookup_instance("bambi").expect("bambi present");
        assert_eq!(
            again.config_overrides.get("plugin.platecycler.enabled"),
            Some(&"false".to_string()),
        );
        // Clear removes the key entirely (back to inherit).
        let cleared =
            set_plugin_override("bambi", "plugin.platecycler.enabled".into(), None).unwrap();
        assert!(!cleared
            .config_overrides
            .contains_key("plugin.platecycler.enabled"));
    }

    #[test]
    fn set_slot_filament_errors_on_bad_extruder() {
        let _registry = RegistryGuard::acquire();
        // Bambi has 1 extruder; index 5 is out of range.
        let err = set_slot_filament("bambi", 5, 0, Some("PLA".into())).unwrap_err();
        assert!(matches!(
            err,
            InstanceMutError::BadExtruder {
                extruders: 1,
                extruder_idx: 5,
                ..
            },
        ));
    }

    #[test]
    fn set_slot_filament_errors_on_bad_slot() {
        let _registry = RegistryGuard::acquire();
        // Snappy extruders have 1 slot each; index 3 is out of range.
        let err = set_slot_filament("snappy", 0, 3, Some("PLA".into())).unwrap_err();
        assert!(matches!(
            err,
            InstanceMutError::BadSlot {
                slots: 1,
                slot_idx: 3,
                ..
            },
        ));
    }

    #[test]
    fn set_slot_color_persists_and_clears() {
        // Mutates a slot the bundled fixture already paints (Bambi
        // AMS:2 — slot index 1 in the AMS-first ordering — ships
        // with `#d4a017`). Registry persistence itself is covered
        // by `set_slot_filament_mutates_in_place_and_persists_across_lookups`.
        let _registry = RegistryGuard::acquire();
        let identity_before = lookup_instance("bambi").expect("bambi present").extruders[0].slots
            [1]
        .filament_identity
        .clone();
        let updated =
            set_slot_color("bambi", 0, 1, Some("#ff8800".into())).expect("bambi AMS:2 exists");
        assert_eq!(
            updated.extruders[0].slots[1].color.as_deref(),
            Some("#ff8800")
        );
        // Filament identity stays untouched — color is its own field.
        assert_eq!(
            updated.extruders[0].slots[1].filament_identity,
            identity_before
        );
        let cleared = set_slot_color("bambi", 0, 1, None).expect("clear ok");
        assert_eq!(cleared.extruders[0].slots[1].color, None);
    }

    #[test]
    fn set_instance_bed_validates_against_supported_plates() {
        let _registry = RegistryGuard::acquire();
        let updated = set_instance_bed("bambi", "Cool Plate".into())
            .expect("Cool Plate is supported by bambi");
        assert_eq!(updated.bed.identity, "Cool Plate");

        // Garbage identity is refused with the typed error.
        let err = set_instance_bed("bambi", "Nonsense Plate".into()).unwrap_err();
        assert!(
            matches!(err, InstanceMutError::UnsupportedBuildPlate { .. }),
            "expected UnsupportedBuildPlate, got {err:?}",
        );
    }

    #[test]
    fn set_instance_bed_errors_on_unknown_instance() {
        let _registry = RegistryGuard::acquire();
        let err = set_instance_bed("ghost", "Cool Plate".into()).unwrap_err();
        assert!(matches!(err, InstanceMutError::UnknownInstance { .. }));
    }

    #[test]
    fn create_instance_builds_topology_from_ams_units() {
        let _registry = RegistryGuard::acquire();
        // 1 AMS = 5 slots: AMS:1..4 (Ams) + Ext (Direct).
        let inst =
            create_instance("bambu-lab-a1-mini", "Garage A1".into(), 1).expect("create with 1 AMS");
        assert_eq!(inst.display_name, "Garage A1");
        assert_eq!(inst.vendor_profile_ref, "bambu-lab-a1-mini");
        // UUIDv4 string shape: 36 chars with hyphens.
        assert_eq!(inst.id.len(), 36);
        assert_eq!(inst.extruders.len(), 1);
        let slots = &inst.extruders[0].slots;
        assert_eq!(slots.len(), 5);
        for slot in slots.iter().take(4) {
            assert_eq!(slot.feed, FeedKind::Ams);
        }
        assert_eq!(slots[4].feed, FeedKind::Direct);
        // First supported bed becomes the default.
        assert!(!inst.bed.identity.is_empty());
    }

    #[test]
    fn create_instance_toolchanger_emits_one_extruder_per_toolhead() {
        let _registry = RegistryGuard::acquire();
        // U1 has 4 toolheads → 4 extruders, each with one Direct
        // slot. AMS units are 0 (ams_max=0). Display labels
        // (`T1..T4`) live in the frontend.
        let inst =
            create_instance("snapmaker-u1", "Test U1".into(), 0).expect("create snapmaker u1");
        assert_eq!(inst.extruders.len(), 4);
        for ext in inst.extruders.iter() {
            assert_eq!(ext.slots.len(), 1);
            assert_eq!(ext.slots[0].feed, FeedKind::Direct);
            // Pre-bound to the nozzle's default filament (Snapmaker PLA).
            assert!(ext.slots[0].filament_identity.is_some());
        }
    }

    #[test]
    fn create_instance_zero_ams_produces_single_direct_slot() {
        let _registry = RegistryGuard::acquire();
        let inst = create_instance("bambu-lab-a1-mini", "Direct-feed bambi".into(), 0)
            .expect("create with 0 AMS units");
        let slots = &inst.extruders[0].slots;
        assert_eq!(slots.len(), 1, "0 AMS units → 1 direct slot");
        assert_eq!(slots[0].feed, FeedKind::Direct);
    }

    #[test]
    fn create_instance_single_ams_emits_four_ams_plus_ext_in_order() {
        let _registry = RegistryGuard::acquire();
        // ams_units=1: 4 Ams-feed slots followed by the trailing
        // Direct-feed external spool. AMS-first ordering matches
        // BBS's ams_mapping convention. Display labels (AMS:1..4 /
        // Ext) live in the frontend, derived from this structure.
        let inst = create_instance("bambu-lab-a1-mini", "Single AMS".into(), 1)
            .expect("ams_max=1 supports 1 unit");
        let feeds: Vec<FeedKind> = inst.extruders[0].slots.iter().map(|s| s.feed).collect();
        assert_eq!(
            feeds,
            vec![
                FeedKind::Ams,
                FeedKind::Ams,
                FeedKind::Ams,
                FeedKind::Ams,
                FeedKind::Direct,
            ],
        );
    }

    #[test]
    fn create_instance_validates_ams_max_and_identity_and_name() {
        let _registry = RegistryGuard::acquire();
        let too_many = create_instance("bambu-lab-a1-mini", "Too many".into(), 2).unwrap_err();
        assert!(matches!(
            too_many,
            InstanceMutError::AmsCountExceeded {
                requested: 2,
                max: 1,
                ..
            }
        ),);
        let unknown = create_instance("nope-printer", "Nope".into(), 0).unwrap_err();
        assert!(matches!(
            unknown,
            InstanceMutError::UnknownPrinterIdentity { .. }
        ));
        let blank = create_instance("bambu-lab-a1-mini", "   ".into(), 0).unwrap_err();
        assert!(matches!(blank, InstanceMutError::EmptyDisplayName));
    }

    #[test]
    fn delete_instance_errors_on_unknown_id() {
        let _registry = RegistryGuard::acquire();
        let err = delete_instance("definitely-not-a-real-uuid").unwrap_err();
        assert!(matches!(err, InstanceMutError::UnknownInstance { .. }));
    }

    #[test]
    fn set_slot_color_errors_on_bad_slot() {
        let _registry = RegistryGuard::acquire();
        let err = set_slot_color("snappy", 0, 3, Some("#fff".into())).unwrap_err();
        assert!(matches!(
            err,
            InstanceMutError::BadSlot {
                slots: 1,
                slot_idx: 3,
                ..
            },
        ));
    }

    #[test]
    fn set_extruder_nozzle_diameter_updates_in_place() {
        let _registry = RegistryGuard::acquire();
        // Snappy is a 4-toolhead toolchanger — pick T3 (extruder
        // index 2) and swap to a 0.6 nozzle so the assertion isn't
        // confounded by the bundled default.
        let updated = set_extruder_nozzle_diameter("snappy", 2, "0.6".to_string())
            .expect("snappy has 4 extruders");
        assert_eq!(updated.extruders[2].installed_nozzle.diameter, "0.6");
        // Material is preserved — the picker only writes diameter.
        let material = updated.extruders[2].installed_nozzle.material;
        let again = lookup_instance("snappy").expect("snappy present after mutation");
        assert_eq!(again.extruders[2].installed_nozzle.diameter, "0.6");
        assert_eq!(again.extruders[2].installed_nozzle.material, material);
    }

    #[test]
    fn set_extruder_nozzle_diameter_resets_incompatible_quality_profile() {
        // Quality-picker rule 3: a nozzle swap that invalidates the
        // currently selected process auto-falls-back to the new
        // nozzle's `default_process_profile`.
        //
        // Bambi (A1 mini, single extruder) starts with a 0.4 nozzle
        // and `0.20mm-standard` (a 0.4-only process). Swap to 0.6
        // — the only nozzle now installed is 0.6, the previous
        // process targets only 0.4, so the fallback kicks in and
        // `quality_profile` becomes the 0.6 nozzle's default
        // (`0.30mm-standard`).
        let _registry = RegistryGuard::acquire();
        // Seed the state we want.
        let _ = mutate_instance("bambi", |inst| {
            inst.extruders[0].installed_nozzle.diameter = "0.4".to_string();
            inst.quality_profile = "0.20mm-standard".to_string();
            Ok(())
        })
        .expect("bambi present");
        let updated = set_extruder_nozzle_diameter("bambi", 0, "0.6".to_string())
            .expect("bambi has 1 extruder");
        assert_eq!(updated.extruders[0].installed_nozzle.diameter, "0.6");
        assert_eq!(
            updated.quality_profile, "0.30mm-standard",
            "0.6 nozzle's default (`0.30mm-standard`) should replace \
             the incompatible 0.4-only `0.20mm-standard`",
        );
    }

    #[test]
    fn set_extruder_nozzle_diameter_keeps_compatible_quality_profile() {
        // Counterpart to the fallback test: if the current process
        // is still valid on the post-swap installed-nozzle set, the
        // quality_profile is preserved.
        //
        // Discriminating setup: snappy (U1, 4 toolheads, all 0.4)
        // seeded with `0.20-quality` — a 0.4-only U1 process.
        // Swap T1's nozzle 0.4 → 0.6: the installed set becomes
        // [0.6, 0.4, 0.4, 0.4]; the union rule keeps `0.20-quality`
        // compatible because 0.4 is still installed. The
        // counterfactual (reset fires) would replace the slug with
        // 0.6's `default_process_profile` (`0.20-standard`), which
        // differs from the seeded `0.20-quality`, so the assertion
        // genuinely discriminates between the two code paths.
        let _registry = RegistryGuard::acquire();
        let _ = mutate_instance("snappy", |inst| {
            for ext in inst.extruders.iter_mut() {
                ext.installed_nozzle.diameter = "0.4".to_string();
            }
            inst.quality_profile = "0.20-quality".to_string();
            Ok(())
        })
        .expect("snappy present");
        let updated = set_extruder_nozzle_diameter("snappy", 0, "0.6".to_string())
            .expect("snappy has 4 extruders");
        assert_eq!(updated.extruders[0].installed_nozzle.diameter, "0.6");
        assert_eq!(
            updated.quality_profile, "0.20-quality",
            "0.20-quality is still compatible with the mixed [0.6, 0.4, …] \
             installed set (union rule via the remaining 0.4 toolheads); \
             quality_profile must not be reset to the destination nozzle's default",
        );
    }

    #[test]
    fn set_extruder_nozzle_diameter_rejects_unbundled_diameter() {
        // The picker only ever offers diameters from
        // `available_nozzle_diameters`; a typed error here surfaces
        // hand-edited instance state or a driver sync that writes a
        // diameter the catalog doesn't bundle.
        let _registry = RegistryGuard::acquire();
        let err = set_extruder_nozzle_diameter("bambi", 0, "1.0".to_string()).unwrap_err();
        assert!(
            matches!(err, InstanceMutError::UnsupportedNozzleDiameter { .. }),
            "expected UnsupportedNozzleDiameter, got {err:?}",
        );
    }

    #[test]
    fn set_extruder_nozzle_diameter_errors_on_bad_extruder() {
        let _registry = RegistryGuard::acquire();
        // Bambi has 1 extruder (AMS-fed single toolhead); index 1 is OOB.
        let err = set_extruder_nozzle_diameter("bambi", 1, "0.4".to_string()).unwrap_err();
        assert!(matches!(
            err,
            InstanceMutError::BadExtruder {
                extruders: 1,
                extruder_idx: 1,
                ..
            },
        ));
    }

    #[test]
    fn set_instance_display_name_trims_and_rejects_empty() {
        let _registry = RegistryGuard::acquire();
        let updated =
            set_instance_display_name("bambi", "  Lab Mini  ".to_string()).expect("rename ok");
        assert_eq!(updated.display_name, "Lab Mini");
        // Persistence round-trip via lookup.
        let again = lookup_instance("bambi").expect("bambi present");
        assert_eq!(again.display_name, "Lab Mini");

        let err = set_instance_display_name("bambi", "   ".to_string()).unwrap_err();
        assert!(matches!(err, InstanceMutError::EmptyDisplayName));
    }

    #[test]
    fn set_instance_ams_units_grow_seeds_new_slots_from_default_filament() {
        // Bambi (A1 mini, ams_max=1) starts at ams_units=1 — 5 slots
        // (4 AMS + Ext). Bumping to 1 is a no-op, but a fresh test
        // starts with whatever the bundled fixture seeded. Shrink to
        // 0 first, then grow to 1 — the new AMS slots inherit
        // `default_filament_fragment_slug`.
        let _registry = RegistryGuard::acquire();
        let shrunk = set_instance_ams_units("bambi", 0).expect("shrink ok");
        assert_eq!(shrunk.extruders[0].slots.len(), 1);
        assert_eq!(shrunk.extruders[0].slots[0].feed, FeedKind::Direct);

        let grown = set_instance_ams_units("bambi", 1).expect("grow ok");
        let slots = &grown.extruders[0].slots;
        assert_eq!(slots.len(), 5);
        for ams_slot in slots.iter().take(4) {
            assert_eq!(ams_slot.feed, FeedKind::Ams);
            assert_eq!(
                ams_slot.filament_identity.as_deref(),
                Some(grown.default_filament_fragment_slug.as_str()),
                "newly-seeded AMS slot inherits the instance's default filament",
            );
        }
        assert_eq!(slots[4].feed, FeedKind::Direct);
    }

    #[test]
    fn set_instance_ams_units_shrink_preserves_overlapping_bindings() {
        let _registry = RegistryGuard::acquire();
        // Bambi ships at ams_units=1 (5 slots). Stamp a custom
        // binding into AMS:1 and the Ext slot, then shrink to 0
        // (1 slot remains — the trailing Direct). The Direct slot
        // pairs with the old last slot, so its binding survives.
        let _ = mutate_instance("bambi", |inst| {
            inst.extruders[0].slots[0].filament_identity = Some("ams-one".into());
            inst.extruders[0].slots[4].filament_identity = Some("ext-trail".into());
            Ok(())
        })
        .expect("bambi present");
        let shrunk = set_instance_ams_units("bambi", 0).expect("shrink ok");
        let slot = &shrunk.extruders[0].slots[0];
        assert_eq!(slot.feed, FeedKind::Direct);
        assert_eq!(
            slot.filament_identity.as_deref(),
            Some("ext-trail"),
            "trailing Direct slot survives the AMS-drop",
        );
    }

    #[test]
    fn set_instance_ams_units_rejects_above_ams_max() {
        let _registry = RegistryGuard::acquire();
        // Bambi's ams_max is 1; requesting 2 hits the typed error.
        let err = set_instance_ams_units("bambi", 2).unwrap_err();
        assert!(
            matches!(
                err,
                InstanceMutError::AmsCountExceeded {
                    max: 1,
                    requested: 2,
                    ..
                }
            ),
            "got {err:?}",
        );
    }

    #[test]
    fn set_instance_ams_units_rejects_for_toolchanger() {
        let _registry = RegistryGuard::acquire();
        // Snappy (U1) has 4 toolheads and ams_max=0 — AMS-units
        // editing isn't meaningful. Any non-zero count is rejected;
        // even 0 is rejected since ams_max==0 collapses the valid
        // range to an empty set.
        let err = set_instance_ams_units("snappy", 0).unwrap_err();
        assert!(
            matches!(err, InstanceMutError::AmsCountExceeded { max: 0, .. }),
            "got {err:?}",
        );
    }

    #[test]
    fn set_instance_connection_round_trip_and_clear() {
        use super::super::instance::ConnectionInfo;
        let _registry = RegistryGuard::acquire();
        let bambu = ConnectionInfo::Bambu {
            host: "192.168.1.42".to_string(),
            access_code: "12345678".to_string(),
        };
        let updated = set_instance_connection("bambi", Some(bambu)).expect("set bambu conn");
        assert!(matches!(
            updated.connection,
            Some(ConnectionInfo::Bambu { ref host, .. }) if host == "192.168.1.42",
        ));
        let cleared = set_instance_connection("bambi", None).expect("clear");
        assert!(cleared.connection.is_none());

        let u1 = ConnectionInfo::U1 {
            host: "snappy.local".to_string(),
            port: 8080,
        };
        let updated = set_instance_connection("snappy", Some(u1)).expect("set u1 conn");
        match updated.connection {
            Some(ConnectionInfo::U1 { host, port }) => {
                assert_eq!(host, "snappy.local");
                assert_eq!(port, 8080);
            }
            other => panic!("expected U1 connection, got {other:?}"),
        }
    }

    #[test]
    fn set_instance_connection_rejects_mismatched_driver_kind() {
        use super::super::instance::ConnectionInfo;
        let _registry = RegistryGuard::acquire();
        // bambi is an A1 mini (driver_kind = bambu); a U1 connection
        // must be refused at the command boundary even though the UI
        // would never offer it.
        let wrong = ConnectionInfo::U1 {
            host: "snappy.local".to_string(),
            port: 80,
        };
        let err = set_instance_connection("bambi", Some(wrong)).unwrap_err();
        assert!(
            matches!(
                err,
                InstanceMutError::ConnectionDriverMismatch {
                    expected: "bambu",
                    got: "u1",
                    ..
                }
            ),
            "got {err:?}",
        );
        // The mismatch must not have been persisted.
        assert!(lookup_instance("bambi")
            .expect("bambi present")
            .connection
            .is_none());

        // The symmetric case: a Bambu connection on the U1 instance.
        let wrong = ConnectionInfo::Bambu {
            host: "10.0.0.5".to_string(),
            access_code: "12345678".to_string(),
        };
        let err = update_instance(
            "snappy",
            InstancePatch {
                connection: Some(wrong),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(
            matches!(
                err,
                InstanceMutError::ConnectionDriverMismatch {
                    expected: "u1",
                    got: "bambu",
                    ..
                }
            ),
            "got {err:?}",
        );
    }

    #[test]
    fn set_instance_connection_rejects_invalid_content() {
        use super::super::instance::ConnectionInfo;
        let _registry = RegistryGuard::acquire();
        // Right kind, but empty host — must be refused on content.
        let empty_host = ConnectionInfo::Bambu {
            host: "   ".to_string(),
            access_code: "12345678".to_string(),
        };
        let err = set_instance_connection("bambi", Some(empty_host)).unwrap_err();
        assert!(
            matches!(err, InstanceMutError::InvalidConnection { .. }),
            "got {err:?}",
        );
        // Right kind, but a non-8-hex-char access code.
        let bad_code = ConnectionInfo::Bambu {
            host: "192.168.1.42".to_string(),
            access_code: "abc12xyz".to_string(),
        };
        let err = set_instance_connection("bambi", Some(bad_code)).unwrap_err();
        assert!(
            matches!(err, InstanceMutError::InvalidConnection { .. }),
            "got {err:?}",
        );
        // U1 port 0 is out of range.
        let bad_port = ConnectionInfo::U1 {
            host: "snappy.local".to_string(),
            port: 0,
        };
        let err = update_instance(
            "snappy",
            InstancePatch {
                connection: Some(bad_port),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(
            matches!(err, InstanceMutError::InvalidConnection { .. }),
            "got {err:?}",
        );
        // None of the rejects persisted anything.
        assert!(lookup_instance("bambi")
            .expect("bambi")
            .connection
            .is_none());
        assert!(lookup_instance("snappy")
            .expect("snappy")
            .connection
            .is_none());
    }

    /// Sanity: the FeedKind + SlotBinding shape round-trips through
    /// the registry's clone/return path without losing the typed
    /// feed kind. Bambi's AMS Lite topology gives us both variants
    /// in one fixture — slots 0-3 are the four `Ams` slots, slot 4
    /// is the `Ext` Direct feed (AMS-first ordering matches BBS's
    /// ams_mapping convention).
    #[test]
    fn feed_kind_survives_registry_round_trip() {
        let _registry = RegistryGuard::acquire();
        let bambi = lookup_instance("bambi").expect("bambi present");
        let slots = &bambi.extruders[0].slots;
        let ams: &SlotBinding = &slots[0];
        assert_eq!(ams.feed, FeedKind::Ams);
        let ext: &SlotBinding = &slots[4];
        assert_eq!(ext.feed, FeedKind::Direct);
    }
}
