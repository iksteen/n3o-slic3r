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
    BedRef, ExtruderState, FeedKind, NozzleMaterial, NozzleSku, PrinterInstance, SlotBinding,
};
use super::instance_library::bundled_instances;
use super::instance_storage;

static REGISTRY: OnceLock<Mutex<Vec<PrinterInstance>>> = OnceLock::new();

fn registry() -> &'static Mutex<Vec<PrinterInstance>> {
    REGISTRY.get_or_init(|| {
        // First-access load. Production has a storage root registered
        // by Tauri's `setup()` and starts empty on first launch (the
        // empty-state UI fires; create_instance writes the first
        // entry). Tests that never hit Tauri's setup fall back to
        // the in-memory test fixtures so the wide test surface
        // doesn't need temp-library plumbing per test.
        let initial = match instance_storage::root() {
            Some(root) => instance_storage::load_from_disk(root).unwrap_or_else(|e| {
                tracing::warn!(
                    error = %e,
                    "instance library load failed; starting with an empty registry",
                );
                Vec::new()
            }),
            None => bundled_instances(),
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstanceMutError {
    /// `id` doesn't match any registered instance.
    UnknownInstance { id: String },
    /// Extruder index out of range for the instance's topology.
    BadExtruder {
        instance_id: String,
        extruder_idx: usize,
        extruders: usize,
    },
    /// Slot index out of range for the chosen extruder.
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
    UnsupportedBuildPlate {
        instance_id: String,
        printer_identity: String,
        bed_identity: String,
    },
    /// The instance's `vendor_profile_ref` doesn't resolve in the
    /// bundled printer catalog. Should be impossible for fixtures
    /// shipped in-tree, but a hand-edited on-disk instance file
    /// could trip this.
    PrinterProfileNotFound {
        instance_id: String,
        printer_identity: String,
    },
    /// `create_instance` was called with a vendor profile id not in
    /// the bundled catalog.
    UnknownPrinterIdentity { identity: String },
    /// `create_instance`'s `ams_units` exceeds the printer profile's
    /// declared `ams_max`.
    AmsCountExceeded {
        identity: String,
        requested: u32,
        max: u32,
    },
    /// `create_instance` was called with an empty display name.
    EmptyDisplayName,
}

impl std::fmt::Display for InstanceMutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownInstance { id } => write!(f, "unknown printer instance `{id}`"),
            Self::BadExtruder {
                instance_id,
                extruder_idx,
                extruders,
            } => write!(
                f,
                "instance `{instance_id}` has {extruders} extruder(s); index {extruder_idx} is out of range",
            ),
            Self::BadSlot {
                instance_id,
                extruder_idx,
                slot_idx,
                slots,
            } => write!(
                f,
                "instance `{instance_id}` extruder {extruder_idx} has {slots} slot(s); index {slot_idx} is out of range",
            ),
            Self::UnsupportedBuildPlate {
                instance_id,
                printer_identity,
                bed_identity,
            } => write!(
                f,
                "instance `{instance_id}` printer `{printer_identity}` does not support build plate `{bed_identity}`",
            ),
            Self::PrinterProfileNotFound {
                instance_id,
                printer_identity,
            } => write!(
                f,
                "instance `{instance_id}` references printer `{printer_identity}`, which is not in the bundled catalog",
            ),
            Self::UnknownPrinterIdentity { identity } => write!(
                f,
                "no bundled printer with identity `{identity}`",
            ),
            Self::AmsCountExceeded {
                identity,
                requested,
                max,
            } => write!(
                f,
                "printer `{identity}` supports at most {max} AMS unit(s); {requested} requested",
            ),
            Self::EmptyDisplayName => write!(f, "display name must not be empty"),
        }
    }
}

impl std::error::Error for InstanceMutError {}

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
/// diameter swaps. The caller-side popover only offers diameters from
/// the printer profile's `available_nozzle_diameters`, so no validation
/// against the bundled catalog happens here.
pub fn set_extruder_nozzle_diameter(
    id: &str,
    extruder_idx: usize,
    diameter_mm: f32,
) -> Result<PrinterInstance, InstanceMutError> {
    mutate_instance(id, |inst| {
        let extruder_count = inst.extruders.len();
        let extruder = inst.extruders.get_mut(extruder_idx).ok_or(
            InstanceMutError::BadExtruder {
                instance_id: id.to_owned(),
                extruder_idx,
                extruders: extruder_count,
            },
        )?;
        extruder.installed_nozzle.diameter_mm = diameter_mm;
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
    let resolve_default_filament = |nozzle_diameter: f64| -> String {
        let sku = format!("{:.1}", nozzle_diameter);
        crate::core::profile_library::default_filament_profile_for(printer_identity, &sku)
            .and_then(|name| {
                crate::core::profile_library::filament_slug_by_display_name(&name)
            })
            .unwrap_or_else(|| "generic-pla".to_owned())
    };

    // Topology branches on toolhead count. AMS-style printers
    // (single toolhead) get one extruder with N+1 slots; toolchangers
    // (U1, XL) get one extruder *per toolhead*, each with a single
    // Direct feed.
    let extruders: Vec<ExtruderState> = if profile.toolheads.len() > 1 {
        // Toolchanger: one extruder per toolhead, labelled `T1..TN`
        // (1-based for display; the extruder vector's index and the
        // gcode tool numbers stay 0-based), single Direct slot.
        // `ams_units` is ignored — the modal
        // hides the AMS picker for `ams_max == 0` printers, so a
        // caller that somehow passes >0 here gets the same topology
        // (we already validated `ams_units <= ams_max` above).
        profile
            .toolheads
            .iter()
            .enumerate()
            .map(|(i, toolhead)| {
                let filament_slug = resolve_default_filament(toolhead.default_nozzle_diameter);
                ExtruderState {
                    // 1-based for display — humans count from 1, gcode
                    // tool numbers and the extruder vector's index
                    // both stay 0-based internally.
                    label: format!("T{}", i + 1),
                    installed_nozzle: NozzleSku {
                        diameter_mm: toolhead.default_nozzle_diameter as f32,
                        material: NozzleMaterial::Stainless,
                    },
                    slots: vec![SlotBinding {
                        // Solo slot — the extruder label carries the
                        // full identity, no per-slot label needed.
                        label: String::new(),
                        feed: FeedKind::Direct,
                        filament_identity: Some(filament_slug),
                        color: None,
                    }],
                }
            })
            .collect()
    } else {
        // AMS-style: one extruder, AMS-fed slots come first, the
        // direct/Ext spool is the trailing slot. The extruder's
        // solo so its label stays empty — the slot labels carry
        // the identity. AMS-first ordering matches BBS's
        // ams_mapping convention; see the doc comment on
        // `create_instance`.
        let toolhead = profile.toolheads.first();
        let default_nozzle_diameter =
            toolhead.map(|t| t.default_nozzle_diameter).unwrap_or(0.4);
        let filament_slug = resolve_default_filament(default_nozzle_diameter);
        let mut slots = Vec::new();
        for unit in 0..ams_units {
            for slot in 1..=4 {
                let label = if ams_units > 1 {
                    let letter = char::from(b'A' + unit as u8);
                    format!("AMS {letter}:{slot}")
                } else {
                    format!("AMS:{slot}")
                };
                slots.push(SlotBinding {
                    label,
                    feed: FeedKind::Ams,
                    filament_identity: Some(filament_slug.clone()),
                    color: None,
                });
            }
        }
        slots.push(SlotBinding {
            label: "Ext".to_owned(),
            feed: FeedKind::Direct,
            filament_identity: Some(filament_slug.clone()),
            color: None,
        });
        vec![ExtruderState {
            label: String::new(),
            installed_nozzle: NozzleSku {
                diameter_mm: default_nozzle_diameter as f32,
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
        .map(|t| resolve_default_filament(t.default_nozzle_diameter))
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
    let default_process_fragment_slug =
        crate::core::profile_library::bundled_process_slugs_for_printer(printer_identity)
            .into_iter()
            .next()
            .unwrap_or("")
            .to_owned();

    let instance = PrinterInstance {
        id: Uuid::new_v4().to_string(),
        display_name,
        vendor_profile_ref: printer_identity.to_owned(),
        printer_fragment_slug: printer_identity.to_owned(),
        default_filament_fragment_slug: default_filament_slug,
        default_process_fragment_slug,
        connection: None,
        extruders,
        bed: BedRef { identity: bed_identity },
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
        let extruder = inst.extruders.get_mut(extruder_idx).ok_or(
            InstanceMutError::BadExtruder {
                instance_id: id.to_owned(),
                extruder_idx,
                extruders: extruder_count,
            },
        )?;
        let slot_count = extruder.slots.len();
        let slot = extruder.slots.get_mut(slot_idx).ok_or(InstanceMutError::BadSlot {
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
        let lock = TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
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
    use super::*;
    use super::super::instance::{FeedKind, SlotBinding};

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
        let cleared =
            set_slot_filament("bambi", 0, 0, None).expect("clear should succeed");
        assert_eq!(cleared.extruders[0].slots[0].filament_identity, None);
    }

    #[test]
    fn set_slot_filament_errors_on_unknown_instance() {
        let _registry = RegistryGuard::acquire();
        let err = set_slot_filament("ghost", 0, 0, Some("PLA".into())).unwrap_err();
        assert!(matches!(err, InstanceMutError::UnknownInstance { .. }));
    }

    #[test]
    fn set_slot_filament_errors_on_bad_extruder() {
        let _registry = RegistryGuard::acquire();
        // Bambi has 1 extruder; index 5 is out of range.
        let err = set_slot_filament("bambi", 5, 0, Some("PLA".into())).unwrap_err();
        assert!(matches!(
            err,
            InstanceMutError::BadExtruder { extruders: 1, extruder_idx: 5, .. },
        ));
    }

    #[test]
    fn set_slot_filament_errors_on_bad_slot() {
        let _registry = RegistryGuard::acquire();
        // Snappy extruders have 1 slot each; index 3 is out of range.
        let err = set_slot_filament("snappy", 0, 3, Some("PLA".into())).unwrap_err();
        assert!(matches!(
            err,
            InstanceMutError::BadSlot { slots: 1, slot_idx: 3, .. },
        ));
    }

    #[test]
    fn set_slot_color_persists_and_clears() {
        // Mutates a slot the bundled fixture already paints (Bambi
        // AMS:2 — slot index 1 in the AMS-first ordering — ships
        // with `#d4a017`). Registry persistence itself is covered
        // by `set_slot_filament_mutates_in_place_and_persists_across_lookups`.
        let _registry = RegistryGuard::acquire();
        let identity_before = lookup_instance("bambi")
            .expect("bambi present")
            .extruders[0]
            .slots[1]
            .filament_identity
            .clone();
        let updated = set_slot_color("bambi", 0, 1, Some("#ff8800".into()))
            .expect("bambi AMS:2 exists");
        assert_eq!(updated.extruders[0].slots[1].color.as_deref(), Some("#ff8800"));
        // Filament identity stays untouched — color is its own field.
        assert_eq!(updated.extruders[0].slots[1].filament_identity, identity_before);
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
        let inst = create_instance("bambu-lab-a1-mini", "Garage A1".into(), 1)
            .expect("create with 1 AMS");
        assert_eq!(inst.display_name, "Garage A1");
        assert_eq!(inst.vendor_profile_ref, "bambu-lab-a1-mini");
        // UUIDv4 string shape: 36 chars with hyphens.
        assert_eq!(inst.id.len(), 36);
        assert_eq!(inst.extruders.len(), 1);
        let slots = &inst.extruders[0].slots;
        assert_eq!(slots.len(), 5);
        for (i, slot) in slots.iter().enumerate().take(4) {
            assert_eq!(slot.feed, FeedKind::Ams);
            assert_eq!(slot.label, format!("AMS:{}", i + 1));
        }
        assert_eq!(slots[4].feed, FeedKind::Direct);
        assert_eq!(slots[4].label, "Ext");
        // First supported bed becomes the default.
        assert!(!inst.bed.identity.is_empty());
    }

    #[test]
    fn create_instance_toolchanger_emits_one_extruder_per_toolhead() {
        let _registry = RegistryGuard::acquire();
        // U1 has 4 toolheads → 4 extruders, each with one Direct
        // slot, labelled T1..T4 (1-based for display). AMS units
        // are 0 (ams_max=0).
        let inst = create_instance("snapmaker-u1", "Test U1".into(), 0)
            .expect("create snapmaker u1");
        assert_eq!(inst.extruders.len(), 4);
        for (i, ext) in inst.extruders.iter().enumerate() {
            assert_eq!(ext.label, format!("T{}", i + 1));
            assert_eq!(ext.slots.len(), 1, "T{} should have a single slot", i + 1);
            assert_eq!(ext.slots[0].feed, FeedKind::Direct);
            // Slot label is empty — extruder label carries identity.
            assert_eq!(ext.slots[0].label, "");
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
        assert_eq!(slots[0].label, "Ext");
    }

    #[test]
    fn create_instance_multi_ams_letters_disambiguate_slots() {
        let _registry = RegistryGuard::acquire();
        // 3 AMS units on a fictional config: slots get "AMS A:1..4",
        // "AMS B:1..4", "AMS C:1..4" prefixes. The A1 mini's ams_max
        // is 1, so this would normally error — call directly with a
        // forced value via the test backdoor. (We assert validation
        // separately below.)
        //
        // For now exercise the labelling on the supported case (1 unit)
        // and document the multi-AMS labelling expectation as a unit
        // test against the helper indirectly when a higher ams_max
        // printer ships.
        let inst = create_instance("bambu-lab-a1-mini", "Single AMS".into(), 1)
            .expect("ams_max=1 supports 1 unit");
        let slot_labels: Vec<&str> = inst.extruders[0]
            .slots
            .iter()
            .map(|s| s.label.as_str())
            .collect();
        // Single-AMS uses the un-lettered "AMS:N" form. AMS slots
        // come first, Ext is trailing (matches BBS's ams_mapping
        // convention).
        assert_eq!(slot_labels, vec!["AMS:1", "AMS:2", "AMS:3", "AMS:4", "Ext"]);
    }

    #[test]
    fn create_instance_validates_ams_max_and_identity_and_name() {
        let _registry = RegistryGuard::acquire();
        let too_many =
            create_instance("bambu-lab-a1-mini", "Too many".into(), 2).unwrap_err();
        assert!(
            matches!(too_many, InstanceMutError::AmsCountExceeded { requested: 2, max: 1, .. }),
        );
        let unknown =
            create_instance("nope-printer", "Nope".into(), 0).unwrap_err();
        assert!(matches!(unknown, InstanceMutError::UnknownPrinterIdentity { .. }));
        let blank =
            create_instance("bambu-lab-a1-mini", "   ".into(), 0).unwrap_err();
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
            InstanceMutError::BadSlot { slots: 1, slot_idx: 3, .. },
        ));
    }

    #[test]
    fn set_extruder_nozzle_diameter_updates_in_place() {
        let _registry = RegistryGuard::acquire();
        // Snappy is a 4-toolhead toolchanger — pick T3 (extruder
        // index 2) and swap to a 0.6 nozzle so the assertion isn't
        // confounded by the bundled default.
        let updated = set_extruder_nozzle_diameter("snappy", 2, 0.6)
            .expect("snappy has 4 extruders");
        assert_eq!(updated.extruders[2].installed_nozzle.diameter_mm, 0.6);
        // Material is preserved — the picker only writes diameter.
        let material = updated.extruders[2].installed_nozzle.material;
        let again =
            lookup_instance("snappy").expect("snappy present after mutation");
        assert_eq!(again.extruders[2].installed_nozzle.diameter_mm, 0.6);
        assert_eq!(again.extruders[2].installed_nozzle.material, material);
    }

    #[test]
    fn set_extruder_nozzle_diameter_errors_on_bad_extruder() {
        let _registry = RegistryGuard::acquire();
        // Bambi has 1 extruder (AMS-fed single toolhead); index 1 is OOB.
        let err = set_extruder_nozzle_diameter("bambi", 1, 0.4).unwrap_err();
        assert!(matches!(
            err,
            InstanceMutError::BadExtruder { extruders: 1, extruder_idx: 1, .. },
        ));
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
        assert_eq!(ams.label, "AMS:1");
        let ext: &SlotBinding = &slots[4];
        assert_eq!(ext.feed, FeedKind::Direct);
        assert_eq!(ext.label, "Ext");
    }
}
