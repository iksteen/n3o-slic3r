//! Mutable in-memory registry of [`PrinterInstance`]s (PR-S-7).
//!
//! Replaces the original `bundled_instances()`-as-source-of-truth
//! pattern: the bundled fixtures now seed a writable registry at
//! first access. User mutations (slot → filament bindings, nozzle
//! swaps, connection settings) land here.
//!
//! Storage shape: `Mutex<Vec<PrinterInstance>>` — small set, linear
//! scan is fine and preserves insertion order for the picker. The
//! `OnceLock` guards seed-once; subsequent calls lock + access.
//!
//! Persistence to disk is intentionally NOT here — MVP keeps the
//! registry in-memory only. App restart resets bindings; that
//! tradeoff is documented in the design doc § "User library
//! persistence is post-MVP — superseded by `instance_storage`."

use std::sync::{Mutex, OnceLock};

use super::instance::PrinterInstance;
use super::instance_library::bundled_instances;
use super::instance_storage;

static REGISTRY: OnceLock<Mutex<Vec<PrinterInstance>>> = OnceLock::new();

fn registry() -> &'static Mutex<Vec<PrinterInstance>> {
    REGISTRY.get_or_init(|| {
        // First-access seed. If the runtime registered a user-library
        // directory, load from there (seeding bundled on first launch);
        // otherwise fall back to the in-memory bundled set. The
        // fall-back keeps unit tests that never hit Tauri's setup
        // working with no surprise filesystem writes.
        let initial = match instance_storage::root() {
            Some(root) => instance_storage::load_or_seed(root).unwrap_or_else(|e| {
                tracing::warn!(
                    error = %e,
                    "instance library load failed; falling back to bundled fixtures",
                );
                bundled_instances()
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

/// Reset the registry to bundled-fixture defaults. Test-only — drops
/// every user mutation. Useful when one test's binding changes would
/// otherwise leak into the next.
#[cfg(test)]
pub fn reset_to_bundled() {
    let mut guard = registry().lock().expect("registry poisoned");
    *guard = bundled_instances();
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::instance::{FeedKind, SlotBinding};

    #[test]
    fn list_returns_bundled_set() {
        reset_to_bundled();
        let instances = list_instances();
        assert!(instances.iter().any(|i| i.id == "bambi"));
        assert!(instances.iter().any(|i| i.id == "snappy"));
    }

    #[test]
    fn lookup_unknown_returns_none() {
        reset_to_bundled();
        assert!(lookup_instance("ghost").is_none());
    }

    #[test]
    fn set_slot_filament_mutates_in_place_and_persists_across_lookups() {
        reset_to_bundled();
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
        reset_to_bundled();
    }

    #[test]
    fn set_slot_filament_errors_on_unknown_instance() {
        reset_to_bundled();
        let err = set_slot_filament("ghost", 0, 0, Some("PLA".into())).unwrap_err();
        assert!(matches!(err, InstanceMutError::UnknownInstance { .. }));
    }

    #[test]
    fn set_slot_filament_errors_on_bad_extruder() {
        reset_to_bundled();
        // Bambi has 1 extruder; index 5 is out of range.
        let err = set_slot_filament("bambi", 5, 0, Some("PLA".into())).unwrap_err();
        assert!(matches!(
            err,
            InstanceMutError::BadExtruder { extruders: 1, extruder_idx: 5, .. },
        ));
    }

    #[test]
    fn set_slot_filament_errors_on_bad_slot() {
        reset_to_bundled();
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
        // AMS:1 ships with `#111827`). Asserts only against the
        // returned post-mutation clone — cross-test re-lookups race
        // with other tests' `reset_to_bundled` calls. Registry
        // persistence itself is covered by
        // `set_slot_filament_mutates_in_place_and_persists_across_lookups`.
        reset_to_bundled();
        let identity_before = lookup_instance("bambi")
            .expect("bambi present")
            .extruders[0]
            .slots[1]
            .filament_identity
            .clone();
        let updated = set_slot_color("bambi", 0, 1, Some("#ff8800".into()))
            .expect("bambi AMS:1 exists");
        assert_eq!(updated.extruders[0].slots[1].color.as_deref(), Some("#ff8800"));
        // Filament identity stays untouched — color is its own field.
        assert_eq!(updated.extruders[0].slots[1].filament_identity, identity_before);
        let cleared = set_slot_color("bambi", 0, 1, None).expect("clear ok");
        assert_eq!(cleared.extruders[0].slots[1].color, None);
        reset_to_bundled();
    }

    #[test]
    fn set_slot_color_errors_on_bad_slot() {
        reset_to_bundled();
        let err = set_slot_color("snappy", 0, 3, Some("#fff".into())).unwrap_err();
        assert!(matches!(
            err,
            InstanceMutError::BadSlot { slots: 1, slot_idx: 3, .. },
        ));
    }

    /// Sanity: the FeedKind + SlotBinding shape round-trips through
    /// the registry's clone/return path without losing the typed
    /// feed kind. Bambi's AMS Lite topology gives us both variants
    /// in one fixture — slot 0 is the `Ext` Direct feed, slots 1-4
    /// are the four `Ams` slots.
    #[test]
    fn feed_kind_survives_registry_round_trip() {
        reset_to_bundled();
        let bambi = lookup_instance("bambi").expect("bambi present");
        let slots = &bambi.extruders[0].slots;
        let ext: &SlotBinding = &slots[0];
        assert_eq!(ext.feed, FeedKind::Direct);
        assert_eq!(ext.label, "Ext");
        let ams: &SlotBinding = &slots[1];
        assert_eq!(ams.feed, FeedKind::Ams);
        assert_eq!(ams.label, "AMS:1");
    }
}
