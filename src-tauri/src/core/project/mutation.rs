//! Mutation methods for [`Project`].
//!
//! Each public method takes `&mut Project` and returns the events
//! the renderer needs to apply. The convention is "pure functions
//! that return event lists; the Tauri layer emits each event via
//! `Window::emit`". Tests bypass the Tauri layer and inspect the
//! returned event list directly.
//!
//! Lives in a sibling file from [`super::model`] so the type
//! definitions stay focused; this file has the mechanics.
//!
//! Plate addressing on the public surface is by [`PlateId`] (stable
//! across reorder + remove). Internal helpers use `usize` indices
//! when they need to mutate sibling plates — the borrow checker
//! wants index-then-deref, not a borrowed `Plate`.

use std::collections::{BTreeSet, HashSet};

use glam::{Quat, Vec3};

use super::model::{Plate, PlateId, Project};
use crate::core::printer::profile::{BoundingBox, PrinterProfile};
use crate::core::scene::bed::{self, BedMesh};
use crate::core::scene::events::{
    MirrorAxis, MoveReport, RepositionReason, SceneEvent, SceneOpError, SelectMode,
};
use crate::core::scene::primitives::{self, PrimitiveKind, PrimitiveParams};
use crate::core::scene::state::{
    mesh_bb_corners, z_extent, Group, GroupId, Mesh, MeshId, MeshProvenance, NewMesh,
    NewSceneObject, ObjectId, SceneObject,
};
use crate::core::scene::transform::Transform;

/// Upper bound on `Plate.name` byte length. Holds back
/// pathological renames that would blow out the tab strip layout
/// or balloon the project `.3mf` JSON skeleton; the actual UI
/// budget is ~24 chars but we accept up to 200 to leave headroom
/// for emoji / non-ASCII users.
pub const PLATE_NAME_MAX: usize = 200;

impl Project {
    // ---- ID allocators (scene-wide) -------------------------------

    /// Allocate the next monotonic `MeshId`. IDs start at 1.
    pub fn next_mesh_id(&mut self) -> MeshId {
        self.next_mesh_id = self.next_mesh_id.wrapping_add(1);
        MeshId(self.next_mesh_id)
    }

    /// Allocate the next monotonic `ObjectId`. IDs start at 1.
    pub fn next_object_id(&mut self) -> ObjectId {
        self.next_object_id = self.next_object_id.wrapping_add(1);
        ObjectId(self.next_object_id)
    }

    /// Allocate the next monotonic `PlateId`. IDs start at 1.
    pub(crate) fn next_plate_id(&self) -> PlateId {
        let max = self.plates.iter().map(|p| p.id.0).max().unwrap_or(0);
        PlateId(max + 1)
    }

    /// Register a mesh in the scene-wide pool. Always allocates a
    /// fresh `MeshId`; the caller hands in a `NewMesh` (no id field)
    /// so there's no possibility of an ID collision or sentinel
    /// ambiguity.
    pub fn register_mesh(&mut self, new_mesh: NewMesh) -> MeshId {
        let id = self.next_mesh_id();
        self.meshes.insert(
            id,
            Mesh {
                id,
                vertices: new_mesh.vertices,
                normals: new_mesh.normals,
                indices: new_mesh.indices,
                paint_colors: new_mesh.paint_colors,
                bounding_box: new_mesh.bounding_box,
                provenance: new_mesh.provenance,
            },
        );
        id
    }

    /// Register a scene object on the active plate. Always
    /// allocates a fresh `ObjectId` (scene-wide unique).
    ///
    /// **Auto-bind side effect:** if the object's model material
    /// (its `extruder_id`, defaulted to `1` when `None`) isn't
    /// already in the active plate's `material_to_slot` map, a
    /// default mapping lands automatically — walks the bound
    /// PrinterInstance's extruder/slot grid in `(extruder, slot)`
    /// order, picks the slot at flat-index `(material - 1) MOD
    /// total_slots`. Idempotent on re-call; user edits via the
    /// panel always win.
    pub fn register_object(&mut self, new_obj: NewSceneObject) -> ObjectId {
        let id = self.next_object_id();
        let active = self.active_plate;
        let extruder_id = new_obj.extruder_id;
        self.plates[active].scene.objects.insert(
            id,
            SceneObject {
                id,
                mesh: new_obj.mesh,
                transform: new_obj.transform,
                name: new_obj.name,
                visible: new_obj.visible,
                extruder_id,
                group: new_obj.group,
            },
        );
        self.ensure_default_material_slot_on_active(extruder_id.unwrap_or(1));
        id
    }

    /// Bind `model_material` to a default slot on the active plate (public
    /// entry to the auto-binder; no-op if already bound or the plate is
    /// unbound). The Orca importer uses this to materialize a project's
    /// **painted** filaments — ones applied to faces via `paint_color`
    /// rather than a per-object `extruder`, so no object carries
    /// `extruder = N` — as bound plate materials, so `material_count`
    /// counts them and the cascade fans + routes them at slice time.
    pub fn ensure_material_bound_on_active(&mut self, model_material: u8) {
        self.ensure_default_material_slot_on_active(model_material);
    }

    /// Carry a foreign file's per-object setting overrides onto
    /// `object_id` on the active plate, keeping only object/region-scoped
    /// libslic3r keys — libslic3r ignores anything else per object (see
    /// [`crate::core::schema::is_object_overridable`]), so storing it would
    /// be an inert no-op. The 3MF / Orca-project loaders call this right
    /// after [`Project::register_object`] with the object's
    /// `ModelObject`/`ModelVolume::config` deltas. Replaces any existing
    /// overrides for the object; a no-op if nothing survives the gate.
    pub fn apply_imported_object_overrides(
        &mut self,
        object_id: ObjectId,
        raw: &std::collections::BTreeMap<String, String>,
    ) {
        // Same gate the slice path uses, so import and slice agree on which
        // keys survive. Its 3-way logging distinguishes a real mis-scoped
        // option (warn) from foreign-3MF bookkeeping metadata like `matrix`
        // / `source_*` (debug) — expected noise on a genuine Orca import.
        let gated = crate::core::schema::gate_object_overrides(raw, object_id.0);
        if gated.is_empty() {
            return;
        }
        let active = self.active_plate;
        self.plates[active]
            .scene
            .object_overrides
            .insert(object_id, gated.into_iter().collect());
    }

    /// Plant a default `material → slot` mapping on the active plate
    /// when the material has no entry yet. Helper for
    /// [`Project::register_object`]; idempotent on re-call.
    ///
    /// Slot selection: walk the bound `PrinterInstance`'s flat slot
    /// list `(extruder, slot)`-major starting at the *preferred*
    /// index `(material - 1) MOD total_slots`, then advance to the
    /// first slot not already bound to another material. Falls back
    /// to the preferred index when every slot is taken (genuine
    /// more-materials-than-slots case).
    ///
    /// Why prefer + skip rather than plain modular ring: a stale
    /// binding from a deleted object lingers in `material_to_slot`,
    /// and a naive modular pick happily doubles a *new* material
    /// onto the slot a previously-deleted material already claims —
    /// e.g. Snappy with M1 manually pinned to T2, then loading a
    /// 2-cube 2-material 3mf would auto-bind M2 to T2 as well
    /// (collision) instead of T3. First-free-from-preferred avoids
    /// the collision; wrap-around at saturation preserves the
    /// previous behavior for the 5-materials-on-4-slots case.
    ///
    /// Bambi (1 extruder × 5 slots, has AMS) → flat list excludes
    /// the external spool, so materials rotate through AMS:1..AMS:4.
    /// Snappy (4 extruders × 1 slot) → material N starts at T(N-1).
    ///
    /// No-op when the plate has no `printer_instance_id` — the slice
    /// path refuses unbound plates anyway, and we don't want to
    /// pin a mapping before the user picks a printer.
    fn ensure_default_material_slot_on_active(&mut self, model_material: u8) {
        if model_material < 1 {
            return;
        }
        let idx = self.active_plate;
        if self.plates[idx]
            .material_to_slot
            .contains_key(&model_material)
        {
            return;
        }
        let Some(instance_id) = self.plates[idx].printer_instance_id().map(str::to_owned) else {
            return;
        };
        let Some(instance) = crate::core::printer::lookup_instance(&instance_id) else {
            return;
        };
        // Flat (extruder, slot) walk in extruder-major order. If the
        // instance has any AMS-fed slots, exclude the external/direct
        // spool from the rotation — the AMS holds the user's everyday
        // filaments; the external spool is for one-offs the user
        // explicitly chose to print from. Auto-loading material 1 onto
        // the external spool means the firmware halts at print time
        // asking the user to feed the PTFE tube. Printers with no AMS
        // (Snapmaker U1, etc.) ship every slot as Direct, so the
        // filter falls back to the full list.
        use crate::core::printer::FeedKind;
        let has_ams = instance
            .extruders
            .iter()
            .any(|e| e.slots.iter().any(|s| s.feed == FeedKind::Ams));
        let flat: Vec<crate::core::printer::SlotRef> = instance
            .extruders
            .iter()
            .enumerate()
            .flat_map(|(e_idx, e)| {
                e.slots
                    .iter()
                    .enumerate()
                    .filter(move |(_, s)| !has_ams || s.feed == FeedKind::Ams)
                    .map(move |(s_idx, _)| crate::core::printer::SlotRef {
                        extruder: e_idx as u8,
                        slot: s_idx as u8,
                    })
            })
            .collect();
        if flat.is_empty() {
            return;
        }
        let taken: std::collections::HashSet<crate::core::printer::SlotRef> = self.plates[idx]
            .material_to_slot
            .values()
            .copied()
            .collect();
        let start = (model_material as usize - 1) % flat.len();
        let pick = (0..flat.len())
            .map(|offset| flat[(start + offset) % flat.len()])
            .find(|s| !taken.contains(s))
            .unwrap_or(flat[start]);
        self.plates[idx]
            .material_to_slot
            .insert(model_material, pick);
    }

    // ---- Plate list mutations -------------------------------------

    /// Append a new plate. `instance_id` is optional — newly-added
    /// plates may stay unbound until the user picks a printer via
    /// picker. Returns the new plate's id paired with the
    /// `PlateAdded` event the renderer subscribes to. Active plate
    /// is unchanged (the caller switches if desired).
    pub fn add_plate(&mut self, instance_id: Option<String>) -> (PlateId, Vec<SceneEvent>) {
        let id = self.next_plate_id();
        let position = (self.plates.len() + 1) as u32;

        // Auto-bind precedence:
        //   1. Caller-supplied `instance_id` wins outright.
        //   2. Otherwise inherit the active plate's binding (most
        //      multi-plate workflows want every plate on the same
        //      printer).
        //   3. Otherwise the new plate is unbound. The frontend
        //      empty-state UI or the picker handles the bind from
        //      here.
        let instance_id = instance_id.or_else(|| {
            self.plates
                .get(self.active_plate)
                .and_then(|p| p.printer_instance_id().map(str::to_owned))
        });

        let mut plate = Plate::new(id, position);
        // Bind + derive the bed together (one path: set_printer) so the new
        // plate renders immediately on plate switch.
        if let Some(iid) = instance_id {
            let profile = crate::core::printer::lookup_instance(&iid)
                .and_then(|inst| crate::core::printer::lookup(&inst.vendor_profile_ref));
            plate.set_printer(Some(iid), profile.as_ref());
        }
        self.plates.push(plate);

        // PlateAdded triggers the frontend's snapshot refetch which
        // pulls bed + binding back in one go. BedChanged is emitted
        // for symmetry with the other bed-setting paths so any
        // listener (e.g. the per-plate scene mirror) sees a single
        // canonical "bed for plate N is X" event.
        let mut events = vec![SceneEvent::PlateAdded { plate_id: id }];
        let new_plate = self.plates.last().expect("plate just pushed");
        if let Some(bed) = &new_plate.scene.bed {
            events.push(SceneEvent::BedChanged {
                plate_id: id,
                bed: Some(bed.clone()),
            });
        }
        (id, events)
    }

    /// Drop a plate by id. Errors when:
    ///   - The plate id isn't in the list.
    ///   - It's the only plate (FR-MP-1: a project must always have
    ///     at least one plate; there is no upper limit).
    ///
    /// On success, repacks `composition_order` so the remaining
    /// plates form a dense `[1..N]` sequence + adjusts
    /// `active_plate` when the removed plate was the active one or
    /// sat before it (emits `ActivePlateChanged` in those cases).
    pub fn remove_plate(&mut self, id: PlateId) -> Result<Vec<SceneEvent>, SceneOpError> {
        if self.plates.len() <= 1 {
            return Err(SceneOpError::LastPlate);
        }
        let idx = self.plate_index(id).ok_or(SceneOpError::UnknownPlate(id))?;
        self.plates.remove(idx);
        let mut events = vec![SceneEvent::PlateRemoved { plate_id: id }];

        // active_plate may need to shift. Two cases:
        //   1. The active plate WAS the removed one → clamp to the
        //      last valid index.
        //   2. The active plate sat AFTER the removed one →
        //      decrement so it still points at the same plate.
        // Both emit ActivePlateChanged so the frontend mirror
        // re-syncs on the new active id.
        let new_active_idx = if self.active_plate == idx {
            self.active_plate.min(self.plates.len() - 1)
        } else if self.active_plate > idx {
            self.active_plate - 1
        } else {
            self.active_plate
        };
        if new_active_idx != self.active_plate {
            self.active_plate = new_active_idx;
            events.push(SceneEvent::ActivePlateChanged {
                plate_id: self.plates[self.active_plate].id,
            });
        }

        // Renumber composition_order so the remaining plates form
        // [1..N] without gaps. Preserves relative ordering.
        let mut order_pairs: Vec<(usize, u32)> = self
            .plates
            .iter()
            .enumerate()
            .map(|(i, p)| (i, p.metadata.composition_order))
            .collect();
        order_pairs.sort_by_key(|&(_, order)| order);
        for (new_pos, (i, _)) in order_pairs.into_iter().enumerate() {
            self.plates[i].metadata.composition_order = (new_pos + 1) as u32;
        }

        Ok(events)
    }

    /// Switch the active plate. No-op (no event) when already on
    /// `id`. Errors when the id is unknown.
    pub fn set_active_plate(&mut self, id: PlateId) -> Result<Vec<SceneEvent>, SceneOpError> {
        let idx = self.plate_index(id).ok_or(SceneOpError::UnknownPlate(id))?;
        if self.active_plate == idx {
            return Ok(Vec::new());
        }
        self.active_plate = idx;
        Ok(vec![SceneEvent::ActivePlateChanged { plate_id: id }])
    }

    // ---- Material → slot routing ------------------------

    /// Upsert a `material → slot` mapping on `plate_id`. The slot
    /// reference is validated against the plate's bound
    /// PrinterInstance — out-of-range `extruder` or `slot` indices
    /// reject with `SceneOpError::InvalidPlateMetadata`.
    pub fn set_material_slot(
        &mut self,
        plate_id: PlateId,
        model_material: u8,
        slot: crate::core::printer::SlotRef,
    ) -> Result<Vec<SceneEvent>, SceneOpError> {
        if model_material < 1 {
            return Err(SceneOpError::InvalidPlateMetadata {
                plate_id,
                message: "model_material must be >= 1".into(),
            });
        }
        let plate_idx = self
            .plate_index(plate_id)
            .ok_or(SceneOpError::UnknownPlate(plate_id))?;
        // Range-check against the bound instance, if any. An unbound
        // plate still accepts the mapping (the slice path rejects
        // separately); range-check would error before the picker can
        // round-trip the user's choice.
        if let Some(instance_id) = self.plates[plate_idx]
            .printer_instance_id()
            .map(str::to_owned)
        {
            if let Some(instance) = crate::core::printer::lookup_instance(&instance_id) {
                let e_count = instance.extruders.len();
                if (slot.extruder as usize) >= e_count {
                    return Err(SceneOpError::InvalidPlateMetadata {
                        plate_id,
                        message: format!(
                            "instance `{instance_id}` has {e_count} extruder(s); index {} is out of range",
                            slot.extruder,
                        ),
                    });
                }
                let s_count = instance.extruders[slot.extruder as usize].slots.len();
                if (slot.slot as usize) >= s_count {
                    return Err(SceneOpError::InvalidPlateMetadata {
                        plate_id,
                        message: format!(
                            "instance `{instance_id}` extruder {} has {s_count} slot(s); index {} is out of range",
                            slot.extruder, slot.slot,
                        ),
                    });
                }
            }
        }
        let prev = self.plates[plate_idx]
            .material_to_slot
            .insert(model_material, slot);
        if prev == Some(slot) {
            return Ok(Vec::new());
        }
        Ok(vec![SceneEvent::MaterialSlotChanged { plate_id }])
    }

    /// Drop the mapping for `model_material`. Silent no-op when there
    /// was no entry.
    pub fn clear_material_slot(
        &mut self,
        plate_id: PlateId,
        model_material: u8,
    ) -> Result<Vec<SceneEvent>, SceneOpError> {
        let plate_idx = self
            .plate_index(plate_id)
            .ok_or(SceneOpError::UnknownPlate(plate_id))?;
        if self.plates[plate_idx]
            .material_to_slot
            .remove(&model_material)
            .is_none()
        {
            return Ok(Vec::new());
        }
        Ok(vec![SceneEvent::MaterialSlotChanged { plate_id }])
    }

    // ---- Plate metadata ----------------------------------

    /// Move a plate to position `order` in the composition queue,
    /// shifting sibling plates' `composition_order` to keep the
    /// queue a dense `[1..plates.len()]` sequence with no gaps.
    ///
    /// Validates `1 <= order <= plates.len()`. No-op (no event) when
    /// the value is unchanged.
    ///
    /// Emits one `PlateMetadataChanged` per plate whose order
    /// actually changed — the moved plate plus every plate between
    /// its old and new positions (inclusive of one endpoint). The
    /// frontend mirror re-renders the affected tab badges.
    pub fn set_plate_composition_order(
        &mut self,
        plate_id: PlateId,
        order: u32,
    ) -> Result<Vec<SceneEvent>, SceneOpError> {
        let n = self.plates.len() as u32;
        if order < 1 || order > n {
            return Err(SceneOpError::InvalidPlateMetadata {
                plate_id,
                message: format!("composition_order must be in 1..={n}, got {order}",),
            });
        }
        let idx = self
            .plate_index(plate_id)
            .ok_or(SceneOpError::UnknownPlate(plate_id))?;
        let old_order = self.plates[idx].metadata.composition_order;
        if old_order == order {
            return Ok(Vec::new());
        }

        // Standard drag-and-drop reorder: every plate strictly
        // between old and new (inclusive of the endpoint nearer
        // `new`) shifts by ±1 to make room. The moved plate
        // takes `order`.
        let (range_lo, range_hi, shift): (u32, u32, i32) = if order < old_order {
            // Moving UP (lower number = earlier in queue). Plates
            // in [order, old_order - 1] shift +1.
            (order, old_order - 1, 1)
        } else {
            // Moving DOWN. Plates in [old_order + 1, order] shift -1.
            (old_order + 1, order, -1)
        };

        let mut affected: Vec<PlateId> = Vec::new();
        for plate in self.plates.iter_mut() {
            if plate.id == plate_id {
                continue;
            }
            let o = plate.metadata.composition_order;
            if o >= range_lo && o <= range_hi {
                plate.metadata.composition_order = (o as i32 + shift) as u32;
                affected.push(plate.id);
            }
        }
        self.plates[idx].metadata.composition_order = order;

        let mut events = vec![SceneEvent::PlateMetadataChanged { plate_id }];
        for id in affected {
            events.push(SceneEvent::PlateMetadataChanged { plate_id: id });
        }
        Ok(events)
    }

    /// Rename a plate (backs the tab-strip dblclick-rename UI).
    /// Trims surrounding whitespace, rejects an empty result and any
    /// name longer than [`PLATE_NAME_MAX`] bytes. No-op (no event)
    /// when the trimmed value matches the current name. Emits
    /// `PlateMetadataChanged` on success — same channel as cycle
    /// count / composition order so the frontend already re-fetches
    /// plate metadata on it.
    pub fn set_plate_name(
        &mut self,
        plate_id: PlateId,
        name: String,
    ) -> Result<Vec<SceneEvent>, SceneOpError> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(SceneOpError::InvalidPlateMetadata {
                plate_id,
                message: "plate name must not be empty".into(),
            });
        }
        if trimmed.len() > PLATE_NAME_MAX {
            return Err(SceneOpError::InvalidPlateMetadata {
                plate_id,
                message: format!("plate name must be at most {PLATE_NAME_MAX} bytes",),
            });
        }
        let idx = self
            .plate_index(plate_id)
            .ok_or(SceneOpError::UnknownPlate(plate_id))?;
        if self.plates[idx].name == trimmed {
            return Ok(Vec::new());
        }
        self.plates[idx].name = trimmed.to_owned();
        Ok(vec![SceneEvent::PlateMetadataChanged { plate_id }])
    }

    /// Set (or clear, with `None`) this plate's process/quality profile.
    /// `Some(slug)` is validated to be a bundled process fragment for the
    /// plate's bound printer; an unknown slug rejects with
    /// `InvalidPlateMetadata`. `None` clears the override so the plate
    /// inherits the bound instance's profile again. No-op (no event) when
    /// unchanged. Emits `PlateMetadataChanged` — the same channel the
    /// frontend already re-fetches plate metadata on.
    pub fn set_plate_quality_profile(
        &mut self,
        plate_id: PlateId,
        quality_profile: Option<String>,
    ) -> Result<Vec<SceneEvent>, SceneOpError> {
        let idx = self
            .plate_index(plate_id)
            .ok_or(SceneOpError::UnknownPlate(plate_id))?;
        // Validate a non-None slug against the plate's bound printer's
        // bundled processes. An unbound plate can't validate, so it
        // accepts the value (the slice path rejects an unbound plate
        // separately).
        if let Some(slug) = &quality_profile {
            if let Some(instance_id) = self.plates[idx].printer_instance_id().map(str::to_owned) {
                if let Some(instance) = crate::core::printer::lookup_instance(&instance_id) {
                    let known = crate::core::profile_library::bundled_process_slugs_for_printer(
                        &instance.printer_fragment_slug,
                    )
                    .iter()
                    .any(|s| *s == slug);
                    if !known {
                        return Err(SceneOpError::InvalidPlateMetadata {
                            plate_id,
                            message: format!(
                                "`{slug}` is not a bundled process for `{}`",
                                instance.printer_fragment_slug,
                            ),
                        });
                    }
                }
            }
        }
        if self.plates[idx].quality_profile == quality_profile {
            return Ok(Vec::new());
        }
        self.plates[idx].quality_profile = quality_profile;
        Ok(vec![SceneEvent::PlateMetadataChanged { plate_id }])
    }

    // ---- Per-plate printer assignment -----------------------------

    /// Install the active plate's bed by passing through a resolved
    /// `PrinterProfile`. `None` clears the bed. The plate's
    /// `printer_instance_id` binding is unchanged here — this is the
    /// bed-viz-only path used by `scene_set_active_printer` and the
    /// arrange helpers. The picker flow that also updates the binding
    /// is [`Self::rebind_plate_printer`].
    pub fn set_active_printer(&mut self, printer: Option<&PrinterProfile>) -> Vec<SceneEvent> {
        let plate_id = self.active_plate().id;
        let new_bed = printer.map(bed::bed_for_printer);
        let plate = &mut self.plates[self.active_plate];
        plate.scene.exclusion_zones = new_bed
            .as_ref()
            .map(|b| b.exclusion_zones.clone())
            .unwrap_or_default();
        plate.scene.bed = new_bed.clone();
        vec![SceneEvent::BedChanged {
            plate_id,
            bed: new_bed,
        }]
    }

    /// Rebind a plate to a different `PrinterInstance` (backs the
    /// printer picker flow). The caller is responsible for
    /// resolving the chosen instance's `PrinterProfile` via the
    /// registry; keeping the registry lookup at the Tauri-command
    /// layer keeps this mutation pure + testable without registry
    /// plumbing.
    ///
    /// Updates `printer_instance_id` (the sole carrier of binding
    /// state) and recomputes the bed visualization. The bed itself
    /// lives on the `PrinterInstance` — change it via
    /// `printer_instance_set_bed`. Emits `BedChanged` +
    /// `PlateMetadataChanged` so the tab strip's printer label
    /// updates and the cascade re-resolves against the new context.
    pub fn rebind_plate_printer(
        &mut self,
        plate_id: PlateId,
        instance_id: String,
        profile: &PrinterProfile,
    ) -> Result<
        (
            crate::core::scene::events::PrinterChangeReport,
            Vec<SceneEvent>,
        ),
        SceneOpError,
    > {
        use crate::core::scene::events::PrinterChangeReport;

        let idx = self
            .plate_index(plate_id)
            .ok_or(SceneOpError::UnknownPlate(plate_id))?;

        let previous_printer = self.plates[idx]
            .printer_instance_id()
            .and_then(crate::core::printer::lookup_instance)
            .map(|inst| inst.vendor_profile_ref);
        // Resolve identity + bed off the new instance for the
        // change-report. If the id doesn't resolve (caller passed a
        // stale uuid from a deleted instance) we still bind it —
        // the slice gate downstream will refuse the unresolved
        // reference with a meaningful error.
        let new_instance = crate::core::printer::lookup_instance(&instance_id);
        let new_printer = new_instance
            .as_ref()
            .map(|i| i.vendor_profile_ref.clone())
            .unwrap_or_else(|| instance_id.clone());
        let new_build_plate = new_instance
            .as_ref()
            .map(|i| i.bed.identity.clone())
            .unwrap_or_default();
        // Bind + derive the bed together (the one binding path).
        self.plates[idx].set_printer(Some(instance_id), Some(profile));
        // Slot refs are physical (extruder, slot) coordinates — they
        // don't survive a topology change. Wipe + re-auto-bind any
        // referenced material against the new printer so existing
        // objects keep a sensible color instead of going gray. The
        // frontend refetches material_to_slot off `PlateMetadataChanged`
        // (always emitted below), so no separate MaterialSlotChanged
        // event is needed.
        self.plates[idx].material_to_slot.clear();
        // The process/quality profile is printer-bound (process fragments
        // are keyed by `(printer, slug)`). A slug the *new* printer doesn't
        // ship is now invalid — left in place it would hard-fail
        // `compose_cascade` at slice + panel-resolve time. Clear it so the
        // plate inherits the new instance's default; a slug the new printer
        // also ships (rebind to the same model) is preserved.
        if let Some(slug) = self.plates[idx].quality_profile.clone() {
            let still_valid = new_instance.as_ref().map_or(false, |i| {
                crate::core::profile_library::bundled_process_slugs_for_printer(
                    &i.printer_fragment_slug,
                )
                .iter()
                .any(|s| *s == slug)
            });
            if !still_valid {
                self.plates[idx].quality_profile = None;
            }
        }
        // Re-bind every material the plate uses — including face-painted ones,
        // which no object's `extruder_id` names. Deriving from objects alone
        // here would drop a painted material's binding on every printer switch.
        let referenced = self.materials_on_plate(&self.plates[idx]);
        let prev_active = self.active_plate;
        self.active_plate = idx;
        for mat in referenced {
            self.ensure_default_material_slot_on_active(mat);
        }
        self.active_plate = prev_active;

        // `set_printer` above already updated the bed/exclusion zones; emit the
        // canonical BedChanged + the metadata change the tab strip listens for.
        let events = vec![
            SceneEvent::BedChanged {
                plate_id,
                bed: self.plates[idx].scene.bed.clone(),
            },
            SceneEvent::PlateMetadataChanged { plate_id },
        ];

        let report = PrinterChangeReport {
            plate_id,
            previous_printer,
            new_printer,
            new_build_plate,
            // Populated by the validation walk in a future PR; for the
            // MVP we ship the picker without proactive warnings.
            incompatible: Vec::new(),
            clamped: Vec::new(),
        };
        Ok((report, events))
    }

    /// Clear a plate's printer binding (`printer_instance_id = None`)
    /// and drop the cached bed visualization. Used when the only
    /// remaining instance is deleted — the user lands on the
    /// add-printer empty state and we don't want stale UUIDs
    /// dangling on plates the next add must rebind. Slot bindings
    /// are also cleared (no printer = no slots).
    ///
    /// No-op when the plate isn't currently bound (still emits
    /// the events so subscribers can re-render defensively).
    pub fn unbind_plate_printer(
        &mut self,
        plate_id: PlateId,
    ) -> Result<Vec<SceneEvent>, SceneOpError> {
        let idx = self
            .plate_index(plate_id)
            .ok_or(SceneOpError::UnknownPlate(plate_id))?;
        // Unbind + clear the bed/exclusion zones together.
        self.plates[idx].set_printer(None, None);
        self.plates[idx].material_to_slot.clear();
        let events = vec![
            SceneEvent::BedChanged {
                plate_id,
                bed: None,
            },
            SceneEvent::PlateMetadataChanged { plate_id },
        ];
        Ok(events)
    }

    // ---- Mesh / object load + place -------------------------------

    /// Register a mesh and place one default `SceneObject` at
    /// origin on the active plate. Returns (mesh_id, object_id,
    /// events).
    pub fn load_mesh(&mut self, new_mesh: NewMesh) -> (MeshId, ObjectId, Vec<SceneEvent>) {
        let obj_name = match &new_mesh.provenance {
            MeshProvenance::File(p) => p
                .rsplit_once('/')
                .map(|(_, leaf)| leaf.to_string())
                .unwrap_or_else(|| p.clone()),
            MeshProvenance::Primitive(name) => name.clone(),
        };
        let mesh_id = self.register_mesh(new_mesh);
        // Lift to z=0 + center on the bed. Without this, an STL/OBJ
        // whose native vertices live below Z=0 (common for symmetric
        // models centered on origin) lands underneath the build
        // plate; libslic3r clips it and the slice produces zero
        // layers. Same treatment add_from_primitive applies to
        // procedural primitives.
        let transform = self.lift_and_center_transform(mesh_id);
        let obj_id = self.register_object(NewSceneObject {
            mesh: mesh_id,
            transform,
            name: obj_name,
            visible: true,
            extruder_id: None,
            group: None,
        });

        let plate_id = self.active_plate().id;
        let mesh_header = self.meshes.get(&mesh_id).unwrap().header();
        let obj_clone = self
            .active_plate()
            .scene
            .objects
            .get(&obj_id)
            .unwrap()
            .clone();
        let events = vec![
            SceneEvent::MeshLoaded { mesh: mesh_header },
            SceneEvent::ObjectAdded {
                plate_id,
                object: obj_clone,
            },
        ];
        (mesh_id, obj_id, events)
    }

    /// Build the transform that drops the mesh's lowest-Z vertex
    /// onto the build plate (Z=0) and centers its XY bbox on the
    /// active plate's bed. Falls back to a Z-only lift when no bed
    /// is bound, and to identity when neither shift is needed.
    ///
    /// Used by both [`Self::add_from_primitive`] and
    /// [`Self::load_mesh`] so file-imported meshes get the same
    /// "land on the bed, centered" treatment as procedural ones.
    fn lift_and_center_transform(&self, mesh_id: MeshId) -> Transform {
        let mesh_bb = self.meshes.get(&mesh_id).unwrap().bounding_box;
        let lift_z = -mesh_bb.min[2] as f32;
        let (shift_x, shift_y) = match &self.active_plate().scene.bed {
            Some(bed) => {
                let bed_cx = ((bed.extents.min[0] + bed.extents.max[0]) * 0.5) as f32;
                let bed_cy = ((bed.extents.min[1] + bed.extents.max[1]) * 0.5) as f32;
                let mesh_cx = ((mesh_bb.min[0] + mesh_bb.max[0]) * 0.5) as f32;
                let mesh_cy = ((mesh_bb.min[1] + mesh_bb.max[1]) * 0.5) as f32;
                (bed_cx - mesh_cx, bed_cy - mesh_cy)
            }
            None => (0.0, 0.0),
        };
        if shift_x.abs() + shift_y.abs() + lift_z.abs() > 1e-5 {
            Transform::translation(Vec3::new(shift_x, shift_y, lift_z))
        } else {
            Transform::IDENTITY
        }
    }

    /// Add (or re-instance) a procedural primitive on the active
    /// plate. Looks up the `(kind, params)` tuple in the scene-wide
    /// cache; if missing, generates the mesh once and stashes the
    /// `MeshId`. Always creates a fresh SceneObject placed at plate
    /// origin so re-clicking "Add cube" in the library palette
    /// piles new objects on top rather than replacing the previous
    /// one.
    pub fn add_from_primitive(
        &mut self,
        kind: PrimitiveKind,
        params: PrimitiveParams,
    ) -> (MeshId, ObjectId, Vec<SceneEvent>) {
        let mut events = Vec::new();
        let mesh_id = match self
            .primitive_cache
            .iter()
            .find(|(k, p, _)| *k == kind && *p == params)
            .map(|(_, _, id)| *id)
        {
            Some(id) => id,
            None => {
                let new_mesh = primitives::generate(kind, params);
                let id = self.register_mesh(new_mesh);
                self.primitive_cache.push((kind, params, id));
                let header = self.meshes.get(&id).unwrap().header();
                events.push(SceneEvent::MeshLoaded { mesh: header });
                id
            }
        };

        let name = match kind {
            PrimitiveKind::Cube => "Cube",
            PrimitiveKind::Cylinder => "Cylinder",
            PrimitiveKind::Sphere => "Sphere",
            PrimitiveKind::Cone => "Cone",
            PrimitiveKind::Torus => "Torus",
        };
        // Lift to z=0 + center on the bed via the shared helper —
        // primitives like cube/sphere/torus are origin-centered
        // geometrically, so a bare-origin placement would sink half
        // the primitive below the plate AND straddle the back-left
        // corner.
        let transform = self.lift_and_center_transform(mesh_id);
        let obj_id = self.register_object(NewSceneObject {
            mesh: mesh_id,
            transform,
            name: name.to_string(),
            visible: true,
            extruder_id: None,
            group: None,
        });
        let plate_id = self.active_plate().id;
        let obj_clone = self
            .active_plate()
            .scene
            .objects
            .get(&obj_id)
            .unwrap()
            .clone();
        events.push(SceneEvent::ObjectAdded {
            plate_id,
            object: obj_clone,
        });
        events.extend(self.out_of_bounds_event(obj_id));
        (mesh_id, obj_id, events)
    }

    // ---- Selection -----------------------------------------------

    /// Apply a selection change on the active plate. Returns one
    /// `SelectionChanged` event (sorted for deterministic output)
    /// or empty if the selection didn't actually change.
    /// Expand `ids` to include every object sharing a group with any of
    /// them. Ungrouped ids pass through unchanged; the result is deduped.
    fn expand_to_groups(
        plate: &crate::core::scene::state::PlateSceneState,
        ids: &[ObjectId],
    ) -> Vec<ObjectId> {
        let groups: HashSet<GroupId> = ids
            .iter()
            .filter_map(|id| plate.objects.get(id).and_then(|o| o.group))
            .collect();
        if groups.is_empty() {
            return ids.to_vec();
        }
        let mut out: HashSet<ObjectId> = ids.iter().copied().collect();
        for o in plate.objects.values() {
            if o.group.is_some_and(|g| groups.contains(&g)) {
                out.insert(o.id);
            }
        }
        out.into_iter().collect()
    }

    /// Expand `ids` to whole-group membership on the active plate. The
    /// canvas uses this so clicking one part selects the whole group;
    /// the object list passes ids straight to `select` to keep parts
    /// individually selectable.
    pub fn group_expanded_ids(&self, ids: &[ObjectId]) -> Vec<ObjectId> {
        Self::expand_to_groups(&self.plates[self.active_plate].scene, ids)
    }

    pub fn select(&mut self, ids: &[ObjectId], mode: SelectMode) -> Vec<SceneEvent> {
        let active = self.active_plate;
        let plate_id = self.plates[active].id;
        let plate = &mut self.plates[active].scene;
        let before: HashSet<ObjectId> = plate.selection.iter().copied().collect();
        // `ids` is taken as-is — the object list selects individuals.
        // The canvas's select-the-whole-group behaviour is opt-in via
        // `group_expanded_ids` at the command layer.
        let present: Vec<ObjectId> = ids
            .iter()
            .copied()
            .filter(|id| plate.objects.contains_key(id))
            .collect();
        match mode {
            SelectMode::Replace => {
                plate.selection = present.into_iter().collect();
            }
            SelectMode::Add => {
                for id in present {
                    plate.selection.insert(id);
                }
            }
            SelectMode::Toggle => {
                // Toggle the (possibly group-expanded) set as a unit:
                // remove it if already fully selected, else add it all.
                let all_selected =
                    !present.is_empty() && present.iter().all(|id| plate.selection.contains(id));
                for id in present {
                    if all_selected {
                        plate.selection.remove(&id);
                    } else {
                        plate.selection.insert(id);
                    }
                }
            }
        }
        if plate.selection == before {
            return Vec::new();
        }
        let mut sorted: Vec<ObjectId> = plate.selection.iter().copied().collect();
        sorted.sort();
        vec![SceneEvent::SelectionChanged {
            plate_id,
            selected: sorted,
        }]
    }

    /// Clear the active plate's selection.
    pub fn deselect_all(&mut self) -> Vec<SceneEvent> {
        let active = self.active_plate;
        let plate_id = self.plates[active].id;
        let plate = &mut self.plates[active].scene;
        if plate.selection.is_empty() {
            return Vec::new();
        }
        plate.selection.clear();
        vec![SceneEvent::SelectionChanged {
            plate_id,
            selected: Vec::new(),
        }]
    }

    // ---- Per-object transforms (active plate) ---------------------

    /// Apply a delta translation to an object on the active plate.
    pub fn translate_object(
        &mut self,
        id: ObjectId,
        delta: Vec3,
    ) -> Result<Vec<SceneEvent>, SceneOpError> {
        let active = self.active_plate;
        let plate_id = self.plates[active].id;
        let obj = self.plates[active]
            .scene
            .objects
            .get_mut(&id)
            .ok_or(SceneOpError::UnknownObject(id))?;
        obj.transform = Transform::translation(delta).compose(obj.transform);
        let clone = obj.clone();
        let mut events = vec![SceneEvent::ObjectUpdated {
            plate_id,
            object: clone,
        }];
        events.extend(self.out_of_bounds_event(id));
        Ok(events)
    }

    /// Set an object's material — its 1-based `extruder_id` — on the
    /// active plate, ensuring that material has a slot binding (the same
    /// auto-binding the add path applies). Emits `ObjectUpdated` for the
    /// object and `MaterialSlotChanged` since the plate's material set
    /// may have gained a new entry.
    pub fn set_object_material(
        &mut self,
        id: ObjectId,
        material: u8,
    ) -> Result<Vec<SceneEvent>, SceneOpError> {
        let active = self.active_plate;
        let plate_id = self.plates[active].id;
        let obj = self.plates[active]
            .scene
            .objects
            .get_mut(&id)
            .ok_or(SceneOpError::UnknownObject(id))?;
        let old_material = obj.extruder_id.unwrap_or(1);
        obj.extruder_id = Some(material);
        let clone = obj.clone();
        // Borrow of `obj` ends above; auto-bind the (possibly new)
        // material to a slot, mirroring the object-add path.
        self.ensure_default_material_slot_on_active(material);
        // Reassigning away from a material can orphan its slot binding — drop
        // it if nothing else (object or MMU paint) on the plate still uses it,
        // the symmetric cleanup `delete_objects` does. Without this the old
        // material lingers in `material_to_slot`, so the Materials panel keeps
        // listing it (it unions object extruders with binding keys).
        if old_material != material {
            let orphan_candidates: BTreeSet<u8> = std::iter::once(old_material).collect();
            self.prune_orphan_material_bindings(active, &orphan_candidates);
        }
        Ok(vec![
            SceneEvent::ObjectUpdated {
                plate_id,
                object: clone,
            },
            SceneEvent::MaterialSlotChanged { plate_id },
        ])
    }

    // ---- Object grouping (active plate) ---------------------------
    //
    // A "group" is a set of objects sharing a `group` — the same
    // mechanism the 3MF loader uses for multi-volume objects, now also
    // driven by user grouping. The writer/slice path already emits a
    // shared `group` as one ModelObject with multiple volumes, so
    // grouping == assembling into one logical print object. Only the
    // display name (`scene.groups`) is new state.

    /// Group `ids` on the active plate under one new, globally-unique
    /// [`GroupId`] named `name`. Objects already in other groups move
    /// into this one; any group thereby left with a single member is
    /// dissolved. No-op for fewer than two ids.
    pub fn group_objects(
        &mut self,
        ids: &[ObjectId],
        name: String,
    ) -> Result<Vec<SceneEvent>, SceneOpError> {
        if ids.len() < 2 {
            return Ok(Vec::new());
        }
        let group = GroupId::fresh();
        let active = self.active_plate;
        let plate_id = self.plates[active].id;
        let mut events = Vec::new();
        for &id in ids {
            let obj = self.plates[active]
                .scene
                .objects
                .get_mut(&id)
                .ok_or(SceneOpError::UnknownObject(id))?;
            obj.group = Some(group);
            events.push(SceneEvent::ObjectUpdated {
                plate_id,
                object: obj.clone(),
            });
        }
        self.plates[active]
            .scene
            .groups
            .insert(group, Group { name });
        events.extend(self.dissolve_orphan_groups_on_active());
        events.push(SceneEvent::PlateMetadataChanged { plate_id });
        Ok(events)
    }

    /// Ungroup: clear `group` from every member of `group` on the active
    /// plate and drop its name.
    pub fn ungroup_objects(&mut self, group: GroupId) -> Vec<SceneEvent> {
        let active = self.active_plate;
        let plate_id = self.plates[active].id;
        let member_ids: Vec<ObjectId> = self.plates[active]
            .scene
            .objects
            .values()
            .filter(|o| o.group == Some(group))
            .map(|o| o.id)
            .collect();
        let mut events = Vec::new();
        for id in member_ids {
            if let Some(obj) = self.plates[active].scene.objects.get_mut(&id) {
                obj.group = None;
                events.push(SceneEvent::ObjectUpdated {
                    plate_id,
                    object: obj.clone(),
                });
            }
        }
        self.plates[active].scene.groups.remove(&group);
        events.push(SceneEvent::PlateMetadataChanged { plate_id });
        events
    }

    /// Rename a group on the active plate.
    pub fn rename_group(&mut self, group: GroupId, name: String) -> Vec<SceneEvent> {
        let active = self.active_plate;
        let plate_id = self.plates[active].id;
        self.plates[active]
            .scene
            .groups
            .insert(group, Group { name });
        vec![SceneEvent::PlateMetadataChanged { plate_id }]
    }

    /// Dissolve any group on the active plate left with fewer than two
    /// members — a group of one isn't a group. Clears those objects'
    /// `group` and drops the names. Returns the per-object events.
    fn dissolve_orphan_groups_on_active(&mut self) -> Vec<SceneEvent> {
        use std::collections::{HashMap, HashSet};
        let active = self.active_plate;
        let plate_id = self.plates[active].id;
        let mut counts: HashMap<GroupId, usize> = HashMap::new();
        for o in self.plates[active].scene.objects.values() {
            if let Some(g) = o.group {
                *counts.entry(g).or_insert(0) += 1;
            }
        }
        let orphans: HashSet<GroupId> = counts
            .into_iter()
            .filter(|&(_, c)| c < 2)
            .map(|(g, _)| g)
            .collect();
        if orphans.is_empty() {
            return Vec::new();
        }
        let ids: Vec<ObjectId> = self.plates[active]
            .scene
            .objects
            .values()
            .filter(|o| o.group.is_some_and(|g| orphans.contains(&g)))
            .map(|o| o.id)
            .collect();
        let mut events = Vec::new();
        for id in ids {
            if let Some(obj) = self.plates[active].scene.objects.get_mut(&id) {
                obj.group = None;
                events.push(SceneEvent::ObjectUpdated {
                    plate_id,
                    object: obj.clone(),
                });
            }
        }
        for g in orphans {
            self.plates[active].scene.groups.remove(&g);
        }
        events
    }

    /// Rotate an object around `axis` by `radians`. Pivot defaults
    /// to the object's current world-space center; `pivot_override`
    /// rotates around an explicit world-space point instead.
    pub fn rotate_object(
        &mut self,
        id: ObjectId,
        axis: Vec3,
        radians: f32,
        pivot_override: Option<Vec3>,
    ) -> Result<Vec<SceneEvent>, SceneOpError> {
        let active = self.active_plate;
        let mesh_bb = {
            let obj = self.plates[active]
                .scene
                .objects
                .get(&id)
                .ok_or(SceneOpError::UnknownObject(id))?;
            self.meshes
                .get(&obj.mesh)
                .ok_or(SceneOpError::UnknownMesh(obj.mesh))?
                .bounding_box
        };

        let plate_id = self.plates[active].id;
        let obj = self.plates[active].scene.objects.get_mut(&id).unwrap();
        let pivot = match pivot_override {
            Some(p) => p,
            None => {
                let local_center = Vec3::new(
                    ((mesh_bb.min[0] + mesh_bb.max[0]) * 0.5) as f32,
                    ((mesh_bb.min[1] + mesh_bb.max[1]) * 0.5) as f32,
                    ((mesh_bb.min[2] + mesh_bb.max[2]) * 0.5) as f32,
                );
                obj.transform.apply_point(local_center)
            }
        };
        let rotation = Quat::from_axis_angle(axis.normalize(), radians);
        // Rotate-around-pivot: translate(-pivot) → rotate →
        // translate(+pivot), applied as a world-space *prefix* to
        // the current transform.
        let rotate_around_pivot = Transform::translation(pivot)
            .compose(Transform::rotation(rotation))
            .compose(Transform::translation(-pivot));
        obj.transform = rotate_around_pivot.compose(obj.transform);
        let clone = obj.clone();
        let mut events = vec![SceneEvent::ObjectUpdated {
            plate_id,
            object: clone,
        }];
        events.extend(self.out_of_bounds_event(id));
        Ok(events)
    }

    /// Scale an object by per-axis factors. Non-uniform scale emits
    /// an extra `NonUniformScale` warning event so the UI can flag
    /// the object; it's not blocking. Dimensional cascade settings
    /// (line widths, top-surface thresholds) reason about physical
    /// extents, and stretching one axis silently breaks those.
    pub fn scale_object(
        &mut self,
        id: ObjectId,
        factor: Vec3,
    ) -> Result<Vec<SceneEvent>, SceneOpError> {
        let active = self.active_plate;
        let plate_id = self.plates[active].id;
        let obj = self.plates[active]
            .scene
            .objects
            .get_mut(&id)
            .ok_or(SceneOpError::UnknownObject(id))?;
        obj.transform = Transform::scale(factor).compose(obj.transform);
        let clone = obj.clone();
        let mut events = vec![SceneEvent::ObjectUpdated {
            plate_id,
            object: clone,
        }];
        if is_non_uniform(factor) {
            events.push(SceneEvent::NonUniformScale {
                plate_id,
                object_id: id,
            });
        }
        events.extend(self.out_of_bounds_event(id));
        Ok(events)
    }

    /// Mirror an object across a world-axis through the object's
    /// world-space center. Two mirrors across the same axis return
    /// the object to its original transform (modulo float error).
    pub fn mirror_object(
        &mut self,
        id: ObjectId,
        axis: MirrorAxis,
    ) -> Result<Vec<SceneEvent>, SceneOpError> {
        let center = self.world_center(id)?;
        let factor = match axis {
            MirrorAxis::X => Vec3::new(-1.0, 1.0, 1.0),
            MirrorAxis::Y => Vec3::new(1.0, -1.0, 1.0),
            MirrorAxis::Z => Vec3::new(1.0, 1.0, -1.0),
        };
        // Mirror-around-center: translate(-c) → scale(±1) →
        // translate(+c), applied as a world-space *prefix* to the
        // current transform.
        let mirror_around_center = Transform::translation(center)
            .compose(Transform::scale(factor))
            .compose(Transform::translation(-center));
        let active = self.active_plate;
        let plate_id = self.plates[active].id;
        let obj = self.plates[active].scene.objects.get_mut(&id).unwrap();
        obj.transform = mirror_around_center.compose(obj.transform);
        let clone = obj.clone();
        let mut events = vec![SceneEvent::ObjectUpdated {
            plate_id,
            object: clone,
        }];
        events.extend(self.out_of_bounds_event(id));
        Ok(events)
    }

    /// Lay-flat heuristic: re-orient the object to minimize its
    /// world-space Z extent, then drop it so the new minimum Z is
    /// exactly the active plate's surface (Z=0 for an identity-
    /// transform plate). Searches the 24 axis-aligned cube rotations
    /// and picks the one that produces the smallest Z extent —
    /// fast, deterministic, no mesh-face analysis. MVP per the
    /// ticket; library + Phase 4 UI can introduce
    /// "lay flat on selected face" later when the user can pick a
    /// face from the viewport.
    pub fn lay_flat_object(&mut self, id: ObjectId) -> Result<Vec<SceneEvent>, SceneOpError> {
        // Quick, engine-free flatten: pick the axis-aligned cube orientation
        // with the smallest Z extent, then settle it onto the plate. The
        // FFI-backed auto-orient shares place_with_rotation below.
        let (mesh_bb, current_scale, current_trans) = {
            let obj = self.plates[self.active_plate]
                .scene
                .objects
                .get(&id)
                .ok_or(SceneOpError::UnknownObject(id))?;
            let bb = self
                .meshes
                .get(&obj.mesh)
                .ok_or(SceneOpError::UnknownMesh(obj.mesh))?
                .bounding_box;
            let (scale, _rot, trans) = obj.transform.to_mat4().to_scale_rotation_translation();
            (bb, scale, trans)
        };
        let local_corners = mesh_bb_corners(&mesh_bb);
        let z_extent_of = |rot: &Quat| {
            let candidate =
                glam::Mat4::from_scale_rotation_translation(current_scale, *rot, current_trans);
            let (min_z, max_z) = z_extent(&local_corners, &candidate);
            max_z - min_z
        };
        let best_rotation = cube_rotations()
            .into_iter()
            .min_by(|a, b| {
                z_extent_of(a)
                    .partial_cmp(&z_extent_of(b))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .expect("24 rotations is non-empty");
        self.place_with_rotation(id, best_rotation)
    }

    /// Replace an object's orientation with `rotation` (object-local → world),
    /// preserving scale and XY center, then settle it onto the plate (min Z → 0).
    /// Shared by [`Self::lay_flat_object`] (best cube rotation) and the FFI-backed
    /// auto-orient (the engine-computed rotation).
    pub fn place_with_rotation(
        &mut self,
        id: ObjectId,
        rotation: Quat,
    ) -> Result<Vec<SceneEvent>, SceneOpError> {
        let active = self.active_plate;
        let mesh_bb = {
            let obj = self.plates[active]
                .scene
                .objects
                .get(&id)
                .ok_or(SceneOpError::UnknownObject(id))?;
            self.meshes
                .get(&obj.mesh)
                .ok_or(SceneOpError::UnknownMesh(obj.mesh))?
                .bounding_box
        };
        let local_corners = mesh_bb_corners(&mesh_bb);
        let local_center = Vec3::new(
            ((mesh_bb.min[0] + mesh_bb.max[0]) * 0.5) as f32,
            ((mesh_bb.min[1] + mesh_bb.max[1]) * 0.5) as f32,
            ((mesh_bb.min[2] + mesh_bb.max[2]) * 0.5) as f32,
        );

        let plate_id = self.plates[active].id;
        let obj = self.plates[active].scene.objects.get_mut(&id).unwrap();
        let current = obj.transform.to_mat4();
        // Decompose so we keep scale + position and only replace the rotation.
        // glam's decomposition is sound for affine matrices without shear; our
        // transforms are translate/rotate/scale compositions only, so this holds.
        let (current_scale, _current_rot, current_trans) = current.to_scale_rotation_translation();
        let current_world_center = obj.transform.apply_point(local_center);

        let chosen =
            glam::Mat4::from_scale_rotation_translation(current_scale, rotation, current_trans);
        let (min_z, _max_z) = z_extent(&local_corners, &chosen);
        let post_rot_center = chosen.transform_point3(local_center);
        let delta = glam::Vec3::new(
            current_world_center.x - post_rot_center.x,
            current_world_center.y - post_rot_center.y,
            -min_z,
        );
        let final_xform = glam::Mat4::from_translation(delta) * chosen;
        obj.transform = Transform::from_mat4(final_xform);

        let clone = obj.clone();
        let mut events = vec![SceneEvent::ObjectUpdated {
            plate_id,
            object: clone,
        }];
        events.extend(self.out_of_bounds_event(id));
        Ok(events)
    }

    /// Build one combined **world-space** mesh from a set of objects — each
    /// object's local mesh baked through its transform — as flattened vertices
    /// + triangle indices. Used to run the FFI auto-orient on a whole selection
    /// (single object, a group, or a multi-select) off the scene lock, so the
    /// optimizer sees the assembly as it currently sits.
    pub fn objects_world_mesh(
        &self,
        ids: &[ObjectId],
    ) -> Result<(Vec<f32>, Vec<u32>), SceneOpError> {
        let active = self.active_plate;
        let mut vertices: Vec<f32> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        for &id in ids {
            let obj = self.plates[active]
                .scene
                .objects
                .get(&id)
                .ok_or(SceneOpError::UnknownObject(id))?;
            let mesh = self
                .meshes
                .get(&obj.mesh)
                .ok_or(SceneOpError::UnknownMesh(obj.mesh))?;
            let base = (vertices.len() / 3) as u32;
            for v in mesh.vertices.chunks_exact(3) {
                let w = obj.transform.apply_point(Vec3::new(v[0], v[1], v[2]));
                vertices.extend_from_slice(&[w.x, w.y, w.z]);
            }
            indices.extend(mesh.indices.iter().map(|&i| i + base));
        }
        Ok((vertices, indices))
    }

    /// Apply an auto-orient `rotation` (world frame, from the FFI orient of the
    /// objects' combined world mesh) to a selection **as one rigid unit**: rotate
    /// every object about the combined world-AABB center, then settle the whole
    /// set onto the plate (combined min Z → 0). Rotating about a shared pivot
    /// keeps groups/assemblies intact (their relative arrangement is preserved).
    pub fn auto_orient_objects(
        &mut self,
        ids: &[ObjectId],
        rotation: Quat,
    ) -> Result<Vec<SceneEvent>, SceneOpError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let active = self.active_plate;
        let plate_id = self.plates[active].id;

        // Combined world-space AABB over the selection (min, max).
        let combined_aabb = |me: &Self| -> Result<(Vec3, Vec3), SceneOpError> {
            let mut lo = Vec3::splat(f32::INFINITY);
            let mut hi = Vec3::splat(f32::NEG_INFINITY);
            for &id in ids {
                let obj = me.plates[active]
                    .scene
                    .objects
                    .get(&id)
                    .ok_or(SceneOpError::UnknownObject(id))?;
                let bb = me
                    .meshes
                    .get(&obj.mesh)
                    .ok_or(SceneOpError::UnknownMesh(obj.mesh))?
                    .bounding_box;
                for corner in mesh_bb_corners(&bb) {
                    let w = obj.transform.apply_point(corner);
                    lo = lo.min(w);
                    hi = hi.max(w);
                }
            }
            Ok((lo, hi))
        };

        let (lo, hi) = combined_aabb(self)?;
        let pivot = (lo + hi) * 0.5;
        let rot_about_pivot = Transform::translation(pivot)
            .compose(Transform::rotation(rotation))
            .compose(Transform::translation(-pivot));
        for &id in ids {
            let obj = self.plates[active].scene.objects.get_mut(&id).unwrap();
            obj.transform = rot_about_pivot.compose(obj.transform);
        }

        // Clamp the oriented selection into the build volume (in place — orient
        // does not recenter). Z sits it on the plate; X/Y pull the footprint back
        // onto the bed only if it overhangs an edge. The small floor margin keeps
        // the settled bottom above z=0 so a sub-micron floating-point dip doesn't
        // trip the strict `min_z < 0` (below-plate) bounds check.
        const FLOOR_MARGIN: f32 = 1.0e-3;
        let (lo2, hi2) = combined_aabb(self)?;
        let mut shift = Vec3::new(0.0, 0.0, FLOOR_MARGIN - lo2.z);
        if let Some(bed) = self.plates[active].scene.bed.as_ref() {
            let clamp = |lo: f32, hi: f32, bmin: f32, bmax: f32| -> f32 {
                if lo < bmin {
                    bmin - lo
                } else if hi > bmax {
                    bmax - hi
                } else {
                    0.0
                }
            };
            shift.x = clamp(lo2.x, hi2.x, bed.extents.min[0] as f32, bed.extents.max[0] as f32);
            shift.y = clamp(lo2.y, hi2.y, bed.extents.min[1] as f32, bed.extents.max[1] as f32);
        }
        let settle = Transform::translation(shift);
        let mut events = Vec::with_capacity(ids.len());
        for &id in ids {
            let obj = self.plates[active].scene.objects.get_mut(&id).unwrap();
            obj.transform = settle.compose(obj.transform);
            events.push(SceneEvent::ObjectUpdated {
                plate_id,
                object: obj.clone(),
            });
        }
        for &id in ids {
            events.extend(self.out_of_bounds_event(id));
        }
        Ok(events)
    }

    /// Replace an object's transform wholesale. Used by
    /// auto-arrange and the gizmo's drag-finalization
    /// step.
    pub fn set_object_transform(
        &mut self,
        id: ObjectId,
        transform: Transform,
    ) -> Result<Vec<SceneEvent>, SceneOpError> {
        let active = self.active_plate;
        let plate_id = self.plates[active].id;
        let obj = self.plates[active]
            .scene
            .objects
            .get_mut(&id)
            .ok_or(SceneOpError::UnknownObject(id))?;
        obj.transform = transform;
        let clone = obj.clone();
        let mut events = vec![SceneEvent::ObjectUpdated {
            plate_id,
            object: clone,
        }];
        events.extend(self.out_of_bounds_event(id));
        Ok(events)
    }

    /// Delete one or more objects on the active plate. Removes
    /// from selection if present. Returns one `ObjectRemoved` event
    /// per id plus (if the selection changed) a `SelectionChanged`
    /// event, plus (if a material's last user was removed) a
    /// `MaterialSlotChanged` event.
    pub fn delete_objects(&mut self, ids: &[ObjectId]) -> Vec<SceneEvent> {
        let active = self.active_plate;
        let plate_id = self.plates[active].id;
        let mut events = Vec::new();
        let mut selection_changed = false;
        // Collect the materials the soon-to-be-removed objects use,
        // *before* removal — otherwise we can't tell which bindings
        // might now be orphaned.
        let mut removed_materials: BTreeSet<u8> = BTreeSet::new();
        {
            let plate = &mut self.plates[active].scene;
            for id in ids {
                if let Some(obj) = plate.objects.remove(id) {
                    removed_materials.insert(obj.extruder_id.unwrap_or(1));
                    events.push(SceneEvent::ObjectRemoved {
                        plate_id,
                        object_id: *id,
                    });
                    if plate.selection.remove(id) {
                        selection_changed = true;
                    }
                }
            }
            if selection_changed {
                let mut sorted: Vec<ObjectId> = plate.selection.iter().copied().collect();
                sorted.sort();
                events.push(SceneEvent::SelectionChanged {
                    plate_id,
                    selected: sorted,
                });
            }
        }
        if self.prune_orphan_material_bindings(active, &removed_materials) {
            events.push(SceneEvent::MaterialSlotChanged { plate_id });
        }
        events
    }

    /// Drop `material_to_slot` entries on `plate_idx` for any material
    /// in `candidates` that no remaining object on the plate uses.
    /// Returns `true` if anything was dropped — callers emit
    /// [`SceneEvent::MaterialSlotChanged`] when so.
    ///
    /// **Why:** the auto-bind in [`ensure_default_material_slot_on_active`]
    /// inserts a binding on first use; without symmetric cleanup,
    /// the binding lingers after every object that referenced the
    /// material is deleted. Loading a new model afterwards then
    /// auto-binds new materials *around* the stale entry — leading to
    /// the "second material lands on the same physical slot as the
    /// long-deleted first material" collision the user hit in the
    /// 2-cube-2-material 3mf load (a fresh M1 pin lingered from an
    /// earlier session; loading M1+M2 routed M2 onto the same slot).
    ///
    /// Object material is `extruder_id.unwrap_or(1)`, matching the
    /// default applied at register-time; plus any material referenced by MMU
    /// paint on the plate's meshes, so a face-painted material (named by no
    /// object's `extruder_id`) isn't pruned out from under the user.
    fn prune_orphan_material_bindings(
        &mut self,
        plate_idx: usize,
        candidates: &BTreeSet<u8>,
    ) -> bool {
        if candidates.is_empty() {
            return false;
        }
        let still_in_use: BTreeSet<u8> = self.materials_on_plate(&self.plates[plate_idx]);
        let mut changed = false;
        for material in candidates {
            if !still_in_use.contains(material)
                && self.plates[plate_idx]
                    .material_to_slot
                    .remove(material)
                    .is_some()
            {
                changed = true;
            }
        }
        changed
    }

    /// Duplicate an object on the active plate. The clone gets a
    /// fresh `ObjectId` and is offset by `+10 mm` in X to avoid
    /// z-fighting with the original.
    pub fn duplicate_object(
        &mut self,
        id: ObjectId,
    ) -> Result<(ObjectId, Vec<SceneEvent>), SceneOpError> {
        let original = self
            .active_plate()
            .scene
            .objects
            .get(&id)
            .ok_or(SceneOpError::UnknownObject(id))?
            .clone();
        let new_id = self.register_object(NewSceneObject {
            mesh: original.mesh,
            transform: Transform::translation(Vec3::new(10.0, 0.0, 0.0))
                .compose(original.transform),
            name: format!("{} (copy)", original.name),
            visible: original.visible,
            extruder_id: original.extruder_id,
            // Duplicate breaks group membership — copying one volume
            // of a multi-volume object yields a solo object, not a
            // 3rd member of the source's group.
            group: None,
        });
        let plate_id = self.active_plate().id;
        let cloned_obj = self
            .active_plate()
            .scene
            .objects
            .get(&new_id)
            .unwrap()
            .clone();
        Ok((
            new_id,
            vec![SceneEvent::ObjectAdded {
                plate_id,
                object: cloned_obj,
            }],
        ))
    }

    // ---- Per-object overrides ----------------------------

    /// Upsert one override on a specific (plate, object).
    pub fn object_override_set(
        &mut self,
        plate_id: PlateId,
        object_id: ObjectId,
        key: String,
        value: String,
    ) -> Result<Vec<SceneEvent>, SceneOpError> {
        let idx = self
            .plate_index(plate_id)
            .ok_or(SceneOpError::UnknownPlate(plate_id))?;
        let plate = &mut self.plates[idx].scene;
        if !plate.objects.contains_key(&object_id) {
            return Err(SceneOpError::UnknownObject(object_id));
        }
        plate
            .object_overrides
            .entry(object_id)
            .or_default()
            .insert(key, value);
        Ok(vec![SceneEvent::ObjectOverridesChanged {
            plate_id,
            object_id,
        }])
    }

    // ---- Per-plate (project-tier) overrides -------------

    /// Upsert one project-tier override on a plate. Same shape as
    /// `object_override_set` but the override applies to the whole
    /// plate (and through the cascade to every object on it) — the
    /// `Plate.project_overrides` map is the "second-most-specific"
    /// override tier the resolver sees, between object and user.
    /// Silent no-op (no event) when the value is unchanged.
    pub fn project_override_set(
        &mut self,
        plate_id: PlateId,
        key: String,
        value: String,
    ) -> Result<Vec<SceneEvent>, SceneOpError> {
        let idx = self
            .plate_index(plate_id)
            .ok_or(SceneOpError::UnknownPlate(plate_id))?;
        let plate = &mut self.plates[idx];
        if plate.project_overrides.get(&key) == Some(&value) {
            return Ok(Vec::new());
        }
        plate.project_overrides.insert(key, value);
        Ok(vec![SceneEvent::ProjectOverridesChanged { plate_id }])
    }

    /// Drop one project-tier override key from a plate. Silent no-op
    /// when the key wasn't present — safe to wire to a per-row reset
    /// button without checking first.
    pub fn project_override_clear(
        &mut self,
        plate_id: PlateId,
        key: &str,
    ) -> Result<Vec<SceneEvent>, SceneOpError> {
        let idx = self
            .plate_index(plate_id)
            .ok_or(SceneOpError::UnknownPlate(plate_id))?;
        let plate = &mut self.plates[idx];
        if plate.project_overrides.remove(key).is_none() {
            return Ok(Vec::new());
        }
        Ok(vec![SceneEvent::ProjectOverridesChanged { plate_id }])
    }

    /// Wipe every project-tier override on a plate. Silent no-op
    /// when the plate had none.
    pub fn project_override_clear_all(
        &mut self,
        plate_id: PlateId,
    ) -> Result<Vec<SceneEvent>, SceneOpError> {
        let idx = self
            .plate_index(plate_id)
            .ok_or(SceneOpError::UnknownPlate(plate_id))?;
        let plate = &mut self.plates[idx];
        if plate.project_overrides.is_empty() {
            return Ok(Vec::new());
        }
        plate.project_overrides.clear();
        Ok(vec![SceneEvent::ProjectOverridesChanged { plate_id }])
    }

    /// Upsert one **user-tier** (project-wide) override. This is
    /// `Project.user_overrides` — the least-specific override tier,
    /// above the authored cascade and below the plate/object tiers. The
    /// project-level plugin surface writes `plugin.<name>.*` keys here.
    /// Silent no-op (no event) when the value is unchanged.
    pub fn user_override_set(
        &mut self,
        key: String,
        value: String,
    ) -> Result<Vec<SceneEvent>, SceneOpError> {
        if self.user_overrides.get(&key) == Some(&value) {
            return Ok(Vec::new());
        }
        self.user_overrides.insert(key, value);
        Ok(vec![SceneEvent::UserOverridesChanged])
    }

    /// Drop one user-tier override key. Silent no-op when absent.
    pub fn user_override_clear(&mut self, key: &str) -> Result<Vec<SceneEvent>, SceneOpError> {
        if self.user_overrides.remove(key).is_none() {
            return Ok(Vec::new());
        }
        Ok(vec![SceneEvent::UserOverridesChanged])
    }

    /// Drop one override key from a specific (plate, object).
    /// Silent no-op (no event) when the override wasn't present.
    /// When the last override on an object is cleared, the
    /// per-object map entry is removed entirely.
    pub fn object_override_clear(
        &mut self,
        plate_id: PlateId,
        object_id: ObjectId,
        key: &str,
    ) -> Result<Vec<SceneEvent>, SceneOpError> {
        let idx = self
            .plate_index(plate_id)
            .ok_or(SceneOpError::UnknownPlate(plate_id))?;
        let plate = &mut self.plates[idx].scene;
        if !plate.objects.contains_key(&object_id) {
            return Err(SceneOpError::UnknownObject(object_id));
        }
        let Some(map) = plate.object_overrides.get_mut(&object_id) else {
            return Ok(Vec::new());
        };
        if map.remove(key).is_none() {
            return Ok(Vec::new());
        }
        if map.is_empty() {
            plate.object_overrides.remove(&object_id);
        }
        Ok(vec![SceneEvent::ObjectOverridesChanged {
            plate_id,
            object_id,
        }])
    }

    /// Wipe every override on a specific (plate, object). Silent
    /// no-op when the object had no overrides.
    pub fn object_override_clear_all(
        &mut self,
        plate_id: PlateId,
        object_id: ObjectId,
    ) -> Result<Vec<SceneEvent>, SceneOpError> {
        let idx = self
            .plate_index(plate_id)
            .ok_or(SceneOpError::UnknownPlate(plate_id))?;
        let plate = &mut self.plates[idx].scene;
        if !plate.objects.contains_key(&object_id) {
            return Err(SceneOpError::UnknownObject(object_id));
        }
        if plate.object_overrides.remove(&object_id).is_none() {
            return Ok(Vec::new());
        }
        Ok(vec![SceneEvent::ObjectOverridesChanged {
            plate_id,
            object_id,
        }])
    }

    // ---- Move object between plates ---------------------

    /// Move an object from `from_plate` to `to_plate`, preserving
    /// its world-space transform when the target plate's build
    /// volume can accept it. Per-object overrides travel with the
    /// object.
    ///
    /// Returns a [`MoveReport`] documenting whether the object had
    /// to be repositioned to fit on the target plate's bed.
    pub fn move_object(
        &mut self,
        from_plate: PlateId,
        to_plate: PlateId,
        object_id: ObjectId,
    ) -> Result<(MoveReport, Vec<SceneEvent>), SceneOpError> {
        if from_plate == to_plate {
            return Err(SceneOpError::SamePlate(from_plate));
        }
        let from_idx = self
            .plate_index(from_plate)
            .ok_or(SceneOpError::UnknownPlate(from_plate))?;
        let to_idx = self
            .plate_index(to_plate)
            .ok_or(SceneOpError::UnknownPlate(to_plate))?;

        let object = self.plates[from_idx]
            .scene
            .objects
            .get(&object_id)
            .ok_or(SceneOpError::UnknownObject(object_id))?
            .clone();
        let overrides = self.plates[from_idx]
            .scene
            .object_overrides
            .remove(&object_id);
        let was_selected = self.plates[from_idx].scene.selection.remove(&object_id);
        let moved_material = object.extruder_id.unwrap_or(1);
        self.plates[from_idx].scene.objects.remove(&object_id);

        let mesh_bb = self
            .meshes
            .get(&object.mesh)
            .ok_or(SceneOpError::UnknownMesh(object.mesh))?
            .bounding_box;
        let (final_obj, reposition_reason) = match (
            self.plates[from_idx].scene.bed.as_ref(),
            self.plates[to_idx].scene.bed.as_ref(),
        ) {
            // Both plates have a bed configured: check if the
            // object's current world position fits the target bed.
            (Some(_), Some(target_bed)) => {
                let reason = object_repositioning_reason(&object, &mesh_bb, target_bed);
                match reason {
                    None => (object, None),
                    Some(why) => {
                        let recentered = recenter_on_bed(&object, &mesh_bb, target_bed);
                        (recentered, Some(why))
                    }
                }
            }
            // Either plate has no bed → no bed-relative check
            // possible; keep the transform as-is.
            _ => (object, None),
        };
        let new_position = final_obj.transform.apply_point(Vec3::ZERO);
        let report = MoveReport {
            object_id,
            new_position: [new_position.x, new_position.y, new_position.z],
            repositioned: reposition_reason,
        };

        self.plates[to_idx]
            .scene
            .objects
            .insert(object_id, final_obj.clone());
        if let Some(map) = overrides {
            self.plates[to_idx]
                .scene
                .object_overrides
                .insert(object_id, map);
        }

        let mut events = vec![
            SceneEvent::ObjectRemoved {
                plate_id: from_plate,
                object_id,
            },
            SceneEvent::ObjectAdded {
                plate_id: to_plate,
                object: final_obj,
            },
        ];
        if was_selected {
            let mut sorted: Vec<ObjectId> = self.plates[from_idx]
                .scene
                .selection
                .iter()
                .copied()
                .collect();
            sorted.sort();
            events.push(SceneEvent::SelectionChanged {
                plate_id: from_plate,
                selected: sorted,
            });
        }
        // The destination plate's auto-bind for the moved object's
        // material doesn't fire here (move_object_to_plate uses
        // `objects.insert` directly, not `register_object`), so the
        // destination's material→slot map is unaffected by the
        // arrival. We *do* prune the source plate, though: a moved
        // object behaves like a deleted one from the source plate's
        // perspective.
        let mut moved_set = BTreeSet::new();
        moved_set.insert(moved_material);
        if self.prune_orphan_material_bindings(from_idx, &moved_set) {
            events.push(SceneEvent::MaterialSlotChanged {
                plate_id: from_plate,
            });
        }
        Ok((report, events))
    }

    // ---- Bounds / helpers ----------------------------------------

    /// Compute the world-space bounding box of all visible objects
    /// on the active plate. Used by `Frame All` in the renderer.
    pub fn visible_bounds(&self) -> Option<BoundingBox> {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        let mut any = false;
        for obj in self.active_plate().scene.objects.values() {
            if !obj.visible {
                continue;
            }
            let mesh = match self.meshes.get(&obj.mesh) {
                Some(m) => m,
                None => continue,
            };
            let bb = &mesh.bounding_box;
            for &x in &[bb.min[0] as f32, bb.max[0] as f32] {
                for &y in &[bb.min[1] as f32, bb.max[1] as f32] {
                    for &z in &[bb.min[2] as f32, bb.max[2] as f32] {
                        let p = obj.transform.apply_point(Vec3::new(x, y, z));
                        min = min.min(p);
                        max = max.max(p);
                        any = true;
                    }
                }
            }
        }
        if any {
            Some(BoundingBox {
                min: [min.x as f64, min.y as f64, min.z as f64],
                max: [max.x as f64, max.y as f64, max.z as f64],
            })
        } else {
            None
        }
    }

    /// World-space center of an object's mesh bounding box on the
    /// active plate. Pulled out so mirror + future bbox-anchored
    /// ops share one path.
    fn world_center(&self, id: ObjectId) -> Result<Vec3, SceneOpError> {
        let obj = self
            .active_plate()
            .scene
            .objects
            .get(&id)
            .ok_or(SceneOpError::UnknownObject(id))?;
        let mesh_bb = self
            .meshes
            .get(&obj.mesh)
            .ok_or(SceneOpError::UnknownMesh(obj.mesh))?
            .bounding_box;
        let local_center = Vec3::new(
            ((mesh_bb.min[0] + mesh_bb.max[0]) * 0.5) as f32,
            ((mesh_bb.min[1] + mesh_bb.max[1]) * 0.5) as f32,
            ((mesh_bb.min[2] + mesh_bb.max[2]) * 0.5) as f32,
        );
        Ok(obj.transform.apply_point(local_center))
    }

    /// Check `object_id` on the active plate against its bed and
    /// emit warnings for every reason it's out of bounds. No bed =
    /// no check (silently). Designed to be called by every
    /// transform op so the UI can flash a non-blocking warning the
    /// instant the user nudges an object off the plate.
    fn out_of_bounds_event(&self, object_id: ObjectId) -> Option<SceneEvent> {
        let plate_id = self.active_plate().id;
        let plate = &self.active_plate().scene;
        let bed = plate.bed.as_ref()?;
        let obj = plate.objects.get(&object_id)?;
        let mesh = self.meshes.get(&obj.mesh)?;
        let reasons = bed::object_out_of_bounds(obj, mesh, bed);
        if reasons.is_empty() {
            None
        } else {
            Some(SceneEvent::ObjectOutOfBounds {
                plate_id,
                object_id,
                reasons,
            })
        }
    }
}

// ---- Free helpers --------------------------------------------------

fn is_non_uniform(factor: Vec3) -> bool {
    let eps = 1e-5_f32;
    (factor.x - factor.y).abs() > eps
        || (factor.x - factor.z).abs() > eps
        || (factor.y - factor.z).abs() > eps
}

/// Why an object had to be repositioned when moving to a different
/// plate. `None` = its world-space position fits the target bed
/// verbatim.
fn object_repositioning_reason(
    obj: &SceneObject,
    mesh_bb: &BoundingBox,
    target_bed: &BedMesh,
) -> Option<RepositionReason> {
    let corners = mesh_bb_corners(mesh_bb);
    let xform = obj.transform.to_mat4();
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for c in corners {
        let p = xform.transform_point3(c);
        min = min.min(p);
        max = max.max(p);
    }
    if min.z < -1e-3 {
        return Some(RepositionReason::BelowBedSurface);
    }
    let bed_min = Vec3::new(
        target_bed.extents.min[0] as f32,
        target_bed.extents.min[1] as f32,
        target_bed.extents.min[2] as f32,
    );
    let bed_max = Vec3::new(
        target_bed.extents.max[0] as f32,
        target_bed.extents.max[1] as f32,
        target_bed.extents.max[2] as f32,
    );
    if min.x < bed_min.x - 1e-3
        || min.y < bed_min.y - 1e-3
        || max.x > bed_max.x + 1e-3
        || max.y > bed_max.y + 1e-3
    {
        return Some(RepositionReason::OutOfBounds);
    }
    for zone in &target_bed.exclusion_zones {
        let z_min = [zone.bounds.min[0] as f32, zone.bounds.min[1] as f32];
        let z_max = [zone.bounds.max[0] as f32, zone.bounds.max[1] as f32];
        let overlap_x = !(max.x < z_min[0] || min.x > z_max[0]);
        let overlap_y = !(max.y < z_min[1] || min.y > z_max[1]);
        if overlap_x && overlap_y {
            return Some(RepositionReason::OnExclusionZone);
        }
    }
    None
}

/// Drop `obj` onto the target plate's XY center at bed Z. Preserves
/// the rotation + scale of the original transform; only the
/// translation part changes.
fn recenter_on_bed(obj: &SceneObject, mesh_bb: &BoundingBox, target_bed: &BedMesh) -> SceneObject {
    let current = obj.transform.to_mat4();
    let (scale, rotation, _trans) = current.to_scale_rotation_translation();
    let local_center = Vec3::new(
        ((mesh_bb.min[0] + mesh_bb.max[0]) * 0.5) as f32,
        ((mesh_bb.min[1] + mesh_bb.max[1]) * 0.5) as f32,
        ((mesh_bb.min[2] + mesh_bb.max[2]) * 0.5) as f32,
    );
    let no_translation = glam::Mat4::from_scale_rotation_translation(scale, rotation, Vec3::ZERO);
    let post_rs_center = no_translation.transform_point3(local_center);
    let corners = mesh_bb_corners(mesh_bb);
    let mut min_z_after_rs = f32::INFINITY;
    for c in corners {
        let p = no_translation.transform_point3(c);
        if p.z < min_z_after_rs {
            min_z_after_rs = p.z;
        }
    }
    let bed_cx = ((target_bed.extents.min[0] + target_bed.extents.max[0]) * 0.5) as f32;
    let bed_cy = ((target_bed.extents.min[1] + target_bed.extents.max[1]) * 0.5) as f32;
    let delta = Vec3::new(
        bed_cx - post_rs_center.x,
        bed_cy - post_rs_center.y,
        -min_z_after_rs,
    );
    let final_xform = glam::Mat4::from_translation(delta) * no_translation;
    SceneObject {
        transform: Transform::from_mat4(final_xform),
        ..obj.clone()
    }
}

/// The 24 proper rotations of a cube. Generated as compositions of
/// identity + (90/180/270)° around each of the three principal axes
/// — that yields 24 distinct rotations (the full chiral octahedral
/// group). Used by [`Project::lay_flat_object`] to pick the
/// orientation that minimizes the world-space Z extent.
fn cube_rotations() -> Vec<Quat> {
    use std::f32::consts::FRAC_PI_2;
    let face_rots = [
        Quat::IDENTITY,
        Quat::from_rotation_y(FRAC_PI_2),
        Quat::from_rotation_y(std::f32::consts::PI),
        Quat::from_rotation_y(-FRAC_PI_2),
        Quat::from_rotation_x(FRAC_PI_2),
        Quat::from_rotation_x(-FRAC_PI_2),
    ];
    let z_spins = [
        Quat::IDENTITY,
        Quat::from_rotation_z(FRAC_PI_2),
        Quat::from_rotation_z(std::f32::consts::PI),
        Quat::from_rotation_z(-FRAC_PI_2),
    ];
    let mut out = Vec::with_capacity(24);
    for f in face_rots {
        for s in z_spins {
            out.push(f * s);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::scene::state::MeshProvenance;

    // ---- Fixtures --------------------------------------------------

    fn unit_cube_mesh() -> NewMesh {
        // 8-corner cube — enough geometry for tests that don't care
        // about visual quality. Normals left zeroed since the
        // mutation-method tests don't shade.
        NewMesh {
            vertices: vec![
                0.0, 0.0, 0.0, //
                1.0, 0.0, 0.0, //
                0.0, 1.0, 0.0, //
                1.0, 1.0, 0.0, //
                0.0, 0.0, 1.0, //
                1.0, 0.0, 1.0, //
                0.0, 1.0, 1.0, //
                1.0, 1.0, 1.0, //
            ],
            normals: vec![0.0; 24],
            indices: vec![
                0, 1, 2, 1, 3, 2, // bottom
                4, 6, 5, 5, 6, 7, // top
                0, 4, 1, 1, 4, 5, // front
                2, 3, 6, 3, 7, 6, // back
                0, 2, 4, 2, 6, 4, // left
                1, 5, 3, 3, 5, 7, // right
            ],
            paint_colors: None,
            bounding_box: BoundingBox {
                min: [0.0, 0.0, 0.0],
                max: [1.0, 1.0, 1.0],
            },
            provenance: MeshProvenance::Primitive("unit-cube".into()),
        }
    }

    /// Test-only helper that lands a unit cube at the world origin
    /// (transform identity). Doesn't use `load_mesh` because that
    /// path now lifts + centers on the bed — fine for the live
    /// importer (file-loaded meshes need to land on the plate) but
    /// noisy for the transform-math tests below, which want exact
    /// corner positions to assert against.
    fn add_cube(p: &mut Project) -> (MeshId, ObjectId) {
        let mesh_id = p.register_mesh(unit_cube_mesh());
        let obj_id = p.register_object(NewSceneObject::at_origin(mesh_id, "cube"));
        (mesh_id, obj_id)
    }

    #[test]
    fn apply_imported_object_overrides_scope_gates_then_stores() {
        let _ = slic3r_ffi::init(None, 3);
        let mut p = Project::default();
        let (_mesh, obj) = add_cube(&mut p);

        p.apply_imported_object_overrides(
            obj,
            &std::collections::BTreeMap::from([
                ("layer_height".to_string(), "0.3".to_string()), // object scope → kept
                ("skirt_loops".to_string(), "2".to_string()),    // print scope → dropped
                ("n3o_not_a_real_key".to_string(), "x".to_string()), // unknown → dropped
            ]),
        );
        let stored = p
            .active_plate()
            .scene
            .object_overrides
            .get(&obj)
            .cloned()
            .unwrap_or_default();
        assert_eq!(
            stored.len(),
            1,
            "only the object-scoped key survives: {stored:?}"
        );
        assert_eq!(stored.get("layer_height").map(String::as_str), Some("0.3"));

        // Input with no object/region keys leaves no override entry at all.
        let (_m2, obj2) = add_cube(&mut p);
        p.apply_imported_object_overrides(
            obj2,
            &std::collections::BTreeMap::from([("skirt_loops".to_string(), "2".to_string())]),
        );
        assert!(
            !p.active_plate().scene.object_overrides.contains_key(&obj2),
            "no object/region keys → no override entry",
        );
    }

    #[test]
    fn group_objects_shares_id_and_name_then_ungroups() {
        let mut p = Project::default();
        let (_, a) = add_cube(&mut p);
        let (_, b) = add_cube(&mut p);

        let events = p.group_objects(&[a, b], "Bracket".into()).unwrap();
        assert!(!events.is_empty());
        let plate = p.active_plate();
        let ga = plate.scene.objects[&a].group.expect("a grouped");
        assert_eq!(plate.scene.objects[&b].group, Some(ga), "shared group id");
        assert_eq!(
            plate.scene.groups.get(&ga).map(|g| g.name.as_str()),
            Some("Bracket"),
        );

        p.ungroup_objects(ga);
        let plate = p.active_plate();
        assert_eq!(plate.scene.objects[&a].group, None);
        assert_eq!(plate.scene.objects[&b].group, None);
        assert!(plate.scene.groups.is_empty(), "name dropped on ungroup");
    }

    #[test]
    fn grouping_fewer_than_two_objects_is_a_noop() {
        let mut p = Project::default();
        let (_, a) = add_cube(&mut p);
        assert!(p.group_objects(&[a], "G".into()).unwrap().is_empty());
        assert_eq!(p.active_plate().scene.objects[&a].group, None);
    }

    #[test]
    fn regrouping_dissolves_a_group_left_with_one_member() {
        let mut p = Project::default();
        let (_, a) = add_cube(&mut p);
        let (_, b) = add_cube(&mut p);
        let (_, c) = add_cube(&mut p);
        p.group_objects(&[a, b], "G1".into()).unwrap();
        let g1 = p.active_plate().scene.objects[&a].group.unwrap();
        // Move b into a new group with c — G1 is left with only a, so it
        // dissolves (a group of one isn't a group).
        p.group_objects(&[b, c], "G2".into()).unwrap();
        let plate = p.active_plate();
        assert_eq!(plate.scene.objects[&a].group, None, "a's group dissolved");
        assert!(!plate.scene.groups.contains_key(&g1));
        let gbc = plate.scene.objects[&b].group.expect("b grouped");
        assert_eq!(plate.scene.objects[&c].group, Some(gbc));
    }

    #[test]
    fn group_expansion_is_opt_in_select_stays_individual() {
        let mut p = Project::default();
        let (_, a) = add_cube(&mut p);
        let (_, b) = add_cube(&mut p);
        let (_, c) = add_cube(&mut p);
        p.group_objects(&[a, b], "G".into()).unwrap();

        // The canvas's expansion helper pulls in the whole group...
        let mut exp = p.group_expanded_ids(&[a]);
        exp.sort();
        let mut want = vec![a, b];
        want.sort();
        assert_eq!(exp, want, "grouped object expands to its group");
        assert_eq!(
            p.group_expanded_ids(&[c]),
            vec![c],
            "ungrouped passes through"
        );

        // ...but `select` itself (the object-list path) stays individual.
        p.select(&[a], SelectMode::Replace);
        let sel = &p.active_plate().scene.selection;
        assert_eq!(sel.len(), 1, "select does not auto-expand");
        assert!(sel.contains(&a));

        // Toggle a multi-id set as a unit (the canvas group-toggle path):
        // a is selected, b is not → not all selected → add both.
        p.select(&[a, b], SelectMode::Toggle);
        assert!(p.active_plate().scene.selection.contains(&b));
        // Both selected now → toggle removes both.
        p.select(&[a, b], SelectMode::Toggle);
        assert!(p.active_plate().scene.selection.is_empty());
    }

    fn a1_mini_for_test() -> PrinterProfile {
        use crate::core::printer::profile::{BoundingBox, Toolhead};
        PrinterProfile {
            model: "Bambu Lab A1 mini".into(),
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

    fn small_printer() -> PrinterProfile {
        use crate::core::printer::profile::{BoundingBox, Toolhead};
        PrinterProfile {
            model: "Small".into(),
            supported_build_plates: vec!["Plain".into()],
            toolheads: vec![Toolhead {
                default_nozzle_diameter: "0.4".to_string(),
                hotend_type: "stainless_steel".into(),
                max_temp: 300.0,
            }],
            build_volume: BoundingBox {
                min: [0.0, 0.0, 0.0],
                max: [100.0, 100.0, 100.0],
            },
            exclusion_zones: vec![],
            ..Default::default()
        }
    }

    // ---- Basic registry behavior ----------------------------------

    #[test]
    fn empty_project_has_no_visible_bounds() {
        let p = Project::default();
        assert!(p.visible_bounds().is_none());
    }

    #[test]
    fn monotonic_ids_dont_reuse() {
        let mut p = Project::default();
        assert_eq!(p.next_mesh_id(), MeshId(1));
        assert_eq!(p.next_mesh_id(), MeshId(2));
        assert_eq!(p.next_object_id(), ObjectId(1));
        assert_eq!(p.next_object_id(), ObjectId(2));
    }

    #[test]
    fn register_mesh_allocates_monotonically() {
        let mut p = Project::default();
        let id = p.register_mesh(unit_cube_mesh());
        assert_eq!(id, MeshId(1));
        let id2 = p.register_mesh(unit_cube_mesh());
        assert_eq!(id2, MeshId(2));
    }

    #[test]
    fn visible_bounds_unions_objects() {
        let mut p = Project::default();
        let mesh_id = p.register_mesh(unit_cube_mesh());
        let _ = p.register_object(NewSceneObject::at_origin(mesh_id, "cube0"));
        let _ = p.register_object(NewSceneObject {
            transform: Transform::translation(Vec3::new(10.0, 0.0, 0.0)),
            name: "cube1".into(),
            ..NewSceneObject::at_origin(mesh_id, "cube1")
        });

        let bb = p.visible_bounds().expect("two visible objects");
        assert_eq!(bb.min, [0.0, 0.0, 0.0]);
        assert!((bb.max[0] - 11.0).abs() < 1e-3, "got {bb:?}");
    }

    #[test]
    fn invisible_objects_skipped_in_bounds() {
        let mut p = Project::default();
        let mesh_id = p.register_mesh(unit_cube_mesh());
        let _ = p.register_object(NewSceneObject {
            visible: false,
            transform: Transform::translation(Vec3::new(100.0, 0.0, 0.0)),
            ..NewSceneObject::at_origin(mesh_id, "hidden")
        });
        assert!(p.visible_bounds().is_none());
    }

    // ---- Selection + basic transforms -----------------------------

    #[test]
    fn load_then_select_then_translate_emits_expected_event_stream() {
        let mut p = Project::default();
        let (_mesh, obj) = add_cube(&mut p);

        let events = p.select(&[obj], SelectMode::Replace);
        assert_eq!(events.len(), 1);
        match &events[0] {
            SceneEvent::SelectionChanged { selected, .. } => {
                assert_eq!(selected, &vec![obj]);
            }
            other => panic!("expected SelectionChanged, got {other:?}"),
        }

        let events = p.translate_object(obj, Vec3::new(5.0, 0.0, 0.0)).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            SceneEvent::ObjectUpdated { object: o, .. } => {
                let center = o.transform.apply_point(Vec3::new(0.5, 0.5, 0.5));
                assert!((center - Vec3::new(5.5, 0.5, 0.5)).length() < 1e-5);
            }
            other => panic!("expected ObjectUpdated, got {other:?}"),
        }
    }

    #[test]
    fn select_no_op_emits_no_event() {
        let mut p = Project::default();
        let (_mesh, obj) = add_cube(&mut p);
        let _ = p.select(&[obj], SelectMode::Replace);
        let events = p.select(&[obj], SelectMode::Replace);
        assert!(events.is_empty(), "re-selecting same set is a no-op");
    }

    #[test]
    fn select_unknown_object_is_skipped() {
        let mut p = Project::default();
        let (_mesh, obj) = add_cube(&mut p);
        let events = p.select(&[obj, ObjectId(9999)], SelectMode::Replace);
        match &events[0] {
            SceneEvent::SelectionChanged { selected, .. } => {
                assert_eq!(selected, &vec![obj], "unknown id filtered out");
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn deselect_all_emits_event_when_selection_was_nonempty() {
        let mut p = Project::default();
        let (_mesh, obj) = add_cube(&mut p);
        let _ = p.select(&[obj], SelectMode::Replace);
        let events = p.deselect_all();
        match &events[0] {
            SceneEvent::SelectionChanged { selected, .. } => assert!(selected.is_empty()),
            _ => unreachable!(),
        }
        // Second deselect_all is a no-op.
        let events = p.deselect_all();
        assert!(events.is_empty());
    }

    #[test]
    fn rotate_around_object_center() {
        let mut p = Project::default();
        let (_mesh, obj) = add_cube(&mut p);
        // Cube center is at (0.5, 0.5, 0.5); rotate 180° around Z.
        let _ = p
            .rotate_object(obj, Vec3::Z, std::f32::consts::PI, None)
            .unwrap();
        let o = p.active_plate().scene.objects.get(&obj).unwrap();
        let corner = o.transform.apply_point(Vec3::ZERO);
        assert!((corner - Vec3::new(1.0, 1.0, 0.0)).length() < 1e-4);
    }

    #[test]
    fn rotate_with_explicit_pivot_preserves_relative_position() {
        let mut p = Project::default();
        let (_mesh, obj) = add_cube(&mut p);
        let _ = p
            .rotate_object(obj, Vec3::Z, std::f32::consts::FRAC_PI_2, Some(Vec3::ZERO))
            .unwrap();
        let o = p.active_plate().scene.objects.get(&obj).unwrap();
        let corner = o.transform.apply_point(Vec3::X);
        assert!((corner - Vec3::Y).length() < 1e-4);
    }

    #[test]
    fn delete_objects_clears_selection_for_deleted_ids() {
        let mut p = Project::default();
        let (_mesh, obj1) = add_cube(&mut p);
        let (_mesh2, obj2) = add_cube(&mut p);
        let _ = p.select(&[obj1, obj2], SelectMode::Replace);
        let events = p.delete_objects(&[obj1]);
        assert_eq!(events.len(), 2);
        assert!(matches!(
            events[0],
            SceneEvent::ObjectRemoved { object_id, .. } if object_id == obj1,
        ));
        match &events[1] {
            SceneEvent::SelectionChanged { selected, .. } => {
                assert_eq!(selected, &vec![obj2]);
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn duplicate_object_offsets_by_10mm_x() {
        let mut p = Project::default();
        let (_mesh, obj) = add_cube(&mut p);
        let (new_id, events) = p.duplicate_object(obj).unwrap();
        assert_ne!(new_id, obj);
        match &events[0] {
            SceneEvent::ObjectAdded { object: o, .. } => {
                let new_corner = o.transform.apply_point(Vec3::ZERO);
                assert!((new_corner - Vec3::new(10.0, 0.0, 0.0)).length() < 1e-5);
                assert!(o.name.contains("(copy)"));
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn unknown_object_ops_return_error() {
        let mut p = Project::default();
        let bad = ObjectId(42);
        assert!(matches!(
            p.translate_object(bad, Vec3::ZERO),
            Err(SceneOpError::UnknownObject(_))
        ));
        assert!(matches!(
            p.rotate_object(bad, Vec3::Z, 0.0, None),
            Err(SceneOpError::UnknownObject(_))
        ));
        assert!(p.delete_objects(&[bad]).is_empty());
    }

    #[test]
    fn project_round_trips_via_json() {
        let mut p = Project::default();
        let mesh_id = p.register_mesh(unit_cube_mesh());
        let _ = p.register_object(NewSceneObject {
            mesh: mesh_id,
            transform: Transform::translation(Vec3::new(5.0, 5.0, 0.0)),
            name: "test-cube".into(),
            visible: true,
            extruder_id: Some(2),
            group: None,
        });

        let json = serde_json::to_string(&p).unwrap();
        let parsed: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.meshes.len(), 1);
        assert_eq!(parsed.active_plate().scene.objects.len(), 1);
        let obj = parsed.active_plate().scene.objects.values().next().unwrap();
        assert_eq!(obj.name, "test-cube");
        assert_eq!(obj.extruder_id, Some(2));
    }

    // ---- Mirror + scale + lay_flat --------------------------------

    #[test]
    fn double_mirror_across_x_returns_to_original() {
        let mut p = Project::default();
        let (_, obj) = add_cube(&mut p);
        let probe = Vec3::new(0.25, 0.5, 0.5);
        let before = p
            .active_plate()
            .scene
            .objects
            .get(&obj)
            .unwrap()
            .transform
            .apply_point(probe);

        p.mirror_object(obj, MirrorAxis::X).unwrap();
        p.mirror_object(obj, MirrorAxis::X).unwrap();

        let after = p
            .active_plate()
            .scene
            .objects
            .get(&obj)
            .unwrap()
            .transform
            .apply_point(probe);
        assert!((after - before).length() < 1e-4);
    }

    #[test]
    fn mirror_x_flips_x_through_world_center() {
        let mut p = Project::default();
        let (_, obj) = add_cube(&mut p);
        p.mirror_object(obj, MirrorAxis::X).unwrap();
        let probe = Vec3::new(1.0, 0.5, 0.5);
        let mirrored = p
            .active_plate()
            .scene
            .objects
            .get(&obj)
            .unwrap()
            .transform
            .apply_point(probe);
        assert!((mirrored - Vec3::new(0.0, 0.5, 0.5)).length() < 1e-5);
    }

    #[test]
    fn non_uniform_scale_emits_warning_event() {
        let mut p = Project::default();
        let (_, obj) = add_cube(&mut p);
        let events = p.scale_object(obj, Vec3::new(2.0, 1.0, 1.0)).unwrap();
        assert!(matches!(events[0], SceneEvent::ObjectUpdated { .. }));
        assert!(matches!(
            events.get(1),
            Some(SceneEvent::NonUniformScale { object_id, .. }) if *object_id == obj,
        ));
    }

    #[test]
    fn uniform_scale_does_not_emit_warning() {
        let mut p = Project::default();
        let (_, obj) = add_cube(&mut p);
        let events = p.scale_object(obj, Vec3::new(1.5, 1.5, 1.5)).unwrap();
        assert_eq!(events.len(), 1, "uniform scale: only ObjectUpdated");
    }

    #[test]
    fn lay_flat_settles_rotated_cube_to_z_zero_min() {
        let mut p = Project::default();
        let (_, obj) = add_cube(&mut p);
        p.translate_object(obj, Vec3::new(0.0, 0.0, 5.0)).unwrap();
        p.rotate_object(obj, Vec3::X, 0.5, None).unwrap();
        p.lay_flat_object(obj).unwrap();

        let xform = p.active_plate().scene.objects.get(&obj).unwrap().transform;
        let mesh_bb = &p.meshes.values().next().unwrap().bounding_box;
        let mut min_z = f32::INFINITY;
        let mut max_z = f32::NEG_INFINITY;
        for &x in &[mesh_bb.min[0] as f32, mesh_bb.max[0] as f32] {
            for &y in &[mesh_bb.min[1] as f32, mesh_bb.max[1] as f32] {
                for &z in &[mesh_bb.min[2] as f32, mesh_bb.max[2] as f32] {
                    let pt = xform.apply_point(Vec3::new(x, y, z));
                    min_z = min_z.min(pt.z);
                    max_z = max_z.max(pt.z);
                }
            }
        }
        assert!(min_z.abs() < 1e-4, "expected min_z ≈ 0, got {min_z}");
        assert!((max_z - 1.0).abs() < 1e-4, "got max_z={max_z}");
    }

    #[test]
    fn auto_orient_identity_preserves_xy_and_settles_on_bed() {
        let mut p = Project::default();
        let (_, obj) = add_cube(&mut p);
        // Place it off-origin and floating above the bed.
        p.translate_object(obj, Vec3::new(50.0, 30.0, 7.0)).unwrap();
        let mesh_bb = p.meshes.values().next().unwrap().bounding_box;
        let center_local = Vec3::new(
            ((mesh_bb.min[0] + mesh_bb.max[0]) * 0.5) as f32,
            ((mesh_bb.min[1] + mesh_bb.max[1]) * 0.5) as f32,
            ((mesh_bb.min[2] + mesh_bb.max[2]) * 0.5) as f32,
        );
        let before = p
            .active_plate()
            .scene
            .objects
            .get(&obj)
            .unwrap()
            .transform
            .apply_point(center_local);
        // Identity rotation (engine "already optimal" / best = z-up): orient in
        // place — keep the XY center, just settle onto the bed.
        p.auto_orient_objects(&[obj], Quat::IDENTITY).unwrap();

        let xform = p.active_plate().scene.objects.get(&obj).unwrap().transform;
        let wc = xform.apply_point(center_local);
        assert!((wc.x - before.x).abs() < 1e-3, "x moved {} -> {}", before.x, wc.x);
        assert!((wc.y - before.y).abs() < 1e-3, "y moved {} -> {}", before.y, wc.y);
        // Settled on the bed: min Z ≈ 0.
        let mut min_z = f32::INFINITY;
        for &x in &[mesh_bb.min[0] as f32, mesh_bb.max[0] as f32] {
            for &y in &[mesh_bb.min[1] as f32, mesh_bb.max[1] as f32] {
                for &z in &[mesh_bb.min[2] as f32, mesh_bb.max[2] as f32] {
                    min_z = min_z.min(xform.apply_point(Vec3::new(x, y, z)).z);
                }
            }
        }
        assert!((0.0..1e-2).contains(&min_z), "min_z={min_z} (expected on the plate, not below)");
    }

    #[test]
    fn auto_orient_clamps_overhanging_footprint_onto_bed() {
        let mut p = Project::default();
        let (_, obj) = add_cube(&mut p);
        let bed_max_x = {
            let bed = p.active_plate().scene.bed.as_ref().unwrap();
            bed.extents.max[0] as f32
        };
        // Push the unit cube partly off the +X edge of the bed.
        p.translate_object(obj, Vec3::new(bed_max_x - 0.5, 30.0, 0.0))
            .unwrap();
        // Even with no reorientation, the clamp must pull the footprint back in.
        p.auto_orient_objects(&[obj], Quat::IDENTITY).unwrap();

        let xform = p.active_plate().scene.objects.get(&obj).unwrap().transform;
        let mesh_bb = p.meshes.values().next().unwrap().bounding_box;
        let mut max_x = f32::NEG_INFINITY;
        for &x in &[mesh_bb.min[0] as f32, mesh_bb.max[0] as f32] {
            for &y in &[mesh_bb.min[1] as f32, mesh_bb.max[1] as f32] {
                for &z in &[mesh_bb.min[2] as f32, mesh_bb.max[2] as f32] {
                    max_x = max_x.max(xform.apply_point(Vec3::new(x, y, z)).x);
                }
            }
        }
        assert!(
            max_x <= bed_max_x + 1e-3,
            "footprint not clamped onto bed: max_x={max_x} > {bed_max_x}"
        );
    }

    // ---- add_from_primitive dedup --------------------------------

    #[test]
    fn add_from_primitive_dedups_same_params_to_one_mesh() {
        let mut p = Project::default();
        let params = PrimitiveParams {
            width: 20.0,
            depth: 20.0,
            height: 20.0,
            radius: 0.0,
            radial_segments: 0,
        };
        let (m1, o1, _) = p.add_from_primitive(PrimitiveKind::Cube, params);
        let (m2, o2, _) = p.add_from_primitive(PrimitiveKind::Cube, params);
        assert_eq!(m1, m2);
        assert_ne!(o1, o2);
        assert_eq!(p.meshes.len(), 1);
    }

    #[test]
    fn add_from_primitive_with_different_params_creates_new_mesh() {
        let mut p = Project::default();
        let p1 = PrimitiveParams {
            width: 20.0,
            depth: 20.0,
            height: 20.0,
            radius: 0.0,
            radial_segments: 0,
        };
        let p2 = PrimitiveParams { width: 30.0, ..p1 };
        let (m1, _, _) = p.add_from_primitive(PrimitiveKind::Cube, p1);
        let (m2, _, _) = p.add_from_primitive(PrimitiveKind::Cube, p2);
        assert_ne!(m1, m2);
        assert_eq!(p.meshes.len(), 2);
    }

    #[test]
    fn add_from_primitive_emits_mesh_loaded_only_first_time() {
        let mut p = Project::default();
        let params = PrimitiveParams::defaults_for(PrimitiveKind::Cube);
        let (_, _, events1) = p.add_from_primitive(PrimitiveKind::Cube, params);
        assert!(events1
            .iter()
            .any(|e| matches!(e, SceneEvent::MeshLoaded { .. })));
        let (_, _, events2) = p.add_from_primitive(PrimitiveKind::Cube, params);
        assert!(!events2
            .iter()
            .any(|e| matches!(e, SceneEvent::MeshLoaded { .. })));
        assert!(events2
            .iter()
            .any(|e| matches!(e, SceneEvent::ObjectAdded { .. })));
    }

    // ---- load_mesh lift + center ---------------------------------

    #[test]
    fn load_mesh_lifts_mesh_with_negative_z_onto_bed() {
        // Regression for "complex models slice to 0 layers": STL
        // files often have vertices below Z=0 (model centered on
        // origin geometrically). load_mesh must lift so the
        // lowest mesh point lands on Z=0, otherwise libslic3r
        // clips the geometry as below-bed and the slice produces
        // no layers.
        let mut p = Project::default();
        let mesh = NewMesh {
            vertices: vec![-5.0, -5.0, -3.0, 5.0, -5.0, -3.0, 0.0, 5.0, 3.0],
            normals: vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
            indices: vec![0, 1, 2],
            paint_colors: None,
            bounding_box: BoundingBox {
                min: [-5.0, -5.0, -3.0],
                max: [5.0, 5.0, 3.0],
            },
            provenance: MeshProvenance::File("/tmp/test.stl".into()),
        };
        let (_, obj_id, _) = p.load_mesh(mesh);
        let obj = p.plates[0].scene.objects.get(&obj_id).unwrap();
        // The mesh's lowest Z (-3) should now land on Z=0 after
        // the transform applies.
        let lifted_min_z = obj.transform.apply_point(Vec3::new(0.0, 0.0, -3.0)).z;
        assert!(
            lifted_min_z.abs() < 1e-4,
            "lowest mesh vertex must land on Z=0, got {lifted_min_z}",
        );
    }

    #[test]
    fn load_mesh_at_origin_with_z_above_zero_does_not_sink_below() {
        // Mesh that's already sitting at Z=0+: lift should be 0
        // (or near-zero), but XY center applies to position on bed.
        let mut p = Project::default();
        let mesh = NewMesh {
            vertices: vec![0.0, 0.0, 0.0, 10.0, 0.0, 0.0, 0.0, 10.0, 5.0],
            normals: vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
            indices: vec![0, 1, 2],
            paint_colors: None,
            bounding_box: BoundingBox {
                min: [0.0, 0.0, 0.0],
                max: [10.0, 10.0, 5.0],
            },
            provenance: MeshProvenance::File("/tmp/test.stl".into()),
        };
        let (_, obj_id, _) = p.load_mesh(mesh);
        let obj = p.plates[0].scene.objects.get(&obj_id).unwrap();
        // Mesh point (0, 0, 0) should still be on the bed (Z=0)
        // after the lift-and-center transform.
        let z = obj.transform.apply_point(Vec3::new(0.0, 0.0, 0.0)).z;
        assert!(z.abs() < 1e-4, "Z=0 vertex stays on bed, got {z}");
    }

    // ---- OOB checks against active plate's bed --------------------

    #[test]
    fn transform_op_with_no_bed_emits_no_oob_event() {
        let mut p = Project::default();
        // Project::default now auto-binds the bundled printer + its
        // bed. This test pins the no-bed code path explicitly, so
        // clear it before the move.
        p.plates[0].scene.bed = None;
        let (_, obj) = add_cube(&mut p);
        let events = p.translate_object(obj, Vec3::new(500.0, 0.0, 0.0)).unwrap();
        assert!(events
            .iter()
            .all(|e| !matches!(e, SceneEvent::ObjectOutOfBounds { .. })));
    }

    #[test]
    fn translate_off_plate_emits_oob_warning_after_active_printer_set() {
        let mut p = Project::default();
        let (_, obj) = add_cube(&mut p);
        let bed_events = p.set_active_printer(Some(&a1_mini_for_test()));
        assert!(matches!(
            bed_events[0],
            SceneEvent::BedChanged { bed: Some(_), .. },
        ));

        let events = p.translate_object(obj, Vec3::new(50.0, 0.0, 0.0)).unwrap();
        assert!(events
            .iter()
            .all(|e| !matches!(e, SceneEvent::ObjectOutOfBounds { .. })));

        let events = p.translate_object(obj, Vec3::new(200.0, 0.0, 0.0)).unwrap();
        assert!(events.iter().any(|e| matches!(
            e,
            SceneEvent::ObjectOutOfBounds { object_id, reasons, .. }
                if *object_id == obj && !reasons.is_empty()
        )));
    }

    #[test]
    fn rotate_around_explicit_pivot_below_plate_emits_below_plate_reason() {
        use crate::core::scene::bed::OutOfBoundsReason;
        let mut p = Project::default();
        let (_, obj) = add_cube(&mut p);
        p.set_active_printer(Some(&a1_mini_for_test()));
        let events = p
            .rotate_object(
                obj,
                Vec3::X,
                std::f32::consts::PI,
                Some(Vec3::new(0.0, 0.0, 0.0)),
            )
            .unwrap();
        let oob = events
            .iter()
            .find_map(|e| match e {
                SceneEvent::ObjectOutOfBounds { reasons, .. } => Some(reasons),
                _ => None,
            })
            .expect("expected OOB event");
        assert!(oob
            .iter()
            .any(|r| matches!(r, OutOfBoundsReason::BelowBuildPlate)));
    }

    // ---- Plate list mutations ----------------------------

    #[test]
    fn default_project_has_one_plate_active_zero() {
        let p = Project::default();
        assert_eq!(p.plates.len(), 1);
        assert_eq!(p.active_plate, 0);
        assert_eq!(p.plates[0].id, PlateId(1));
    }

    #[test]
    fn add_plate_appends_new_id_and_keeps_active() {
        let mut p = Project::default();
        let (new_id, events) = p.add_plate(None);
        assert_eq!(new_id, PlateId(2));
        assert_eq!(p.plates.len(), 2);
        assert_eq!(p.active_plate, 0, "active plate unchanged");
        // PlateAdded + BedChanged: the auto-bind branch (default
        // binding inherits from active plate) populates the new
        // plate's bed in the same mutation, so the renderer sees
        // both events.
        assert!(
            matches!(events.first(), Some(SceneEvent::PlateAdded { plate_id }) if *plate_id == new_id)
        );
        assert!(events
            .iter()
            .any(|e| matches!(e, SceneEvent::BedChanged { plate_id, .. } if *plate_id == new_id)));
    }

    #[test]
    fn add_plate_inherits_printer_from_active_plate() {
        let mut p = Project::default();
        // Override the bootstrap plate's instance so we can tell the
        // inheritance apart from the bundled-default fallback.
        p.plates[0].set_printer(Some("snappy".into()), None);
        let (new_id, _) = p.add_plate(None);
        let new_plate = p.plate(new_id).unwrap();
        assert_eq!(
            new_plate.printer_instance_id(),
            Some("snappy"),
            "inherits from active plate",
        );
    }

    #[test]
    fn add_plate_unbound_when_active_is_unbound() {
        let mut p = Project::default();
        p.plates[0].set_printer(None, None);
        p.plates[0].scene.bed = None;
        let (new_id, _) = p.add_plate(None);
        let new_plate = p.plate(new_id).unwrap();
        assert!(
            new_plate.printer_instance_id().is_none(),
            "no inheritance source + no caller-supplied id → unbound",
        );
    }

    #[test]
    fn add_plate_respects_explicit_instance_id() {
        let mut p = Project::default();
        let (new_id, _) = p.add_plate(Some("snappy".into()));
        let new_plate = p.plate(new_id).unwrap();
        assert_eq!(
            new_plate.printer_instance_id(),
            Some("snappy"),
            "explicit instance id wins over inheritance",
        );
    }

    #[test]
    fn project_default_bootstraps_with_bundled_printer() {
        let p = Project::default();
        assert!(
            p.plates[0].printer_instance_id().is_some(),
            "default project's first plate is auto-bound",
        );
        assert!(
            p.plates[0].scene.bed.is_some(),
            "auto-bind also populates the bed visualization",
        );
    }

    #[test]
    fn set_active_plate_switches_and_emits() {
        let mut p = Project::default();
        let (id, _) = p.add_plate(None);
        let events = p.set_active_plate(id).unwrap();
        assert_eq!(p.active_plate, 1);
        assert!(matches!(
            events.as_slice(),
            [SceneEvent::ActivePlateChanged { plate_id }] if *plate_id == id,
        ));
    }

    #[test]
    fn set_active_plate_to_current_is_silent_noop() {
        let mut p = Project::default();
        p.add_plate(None);
        let events = p.set_active_plate(PlateId(1)).unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn set_active_plate_unknown_errors() {
        let mut p = Project::default();
        let err = p.set_active_plate(PlateId(99)).unwrap_err();
        assert_eq!(err, SceneOpError::UnknownPlate(PlateId(99)));
    }

    #[test]
    fn remove_last_plate_errors() {
        let mut p = Project::default();
        let err = p.remove_plate(PlateId(1)).unwrap_err();
        assert_eq!(err, SceneOpError::LastPlate);
    }

    #[test]
    fn remove_active_plate_clamps_active_to_last() {
        let mut p = Project::default();
        let (id_b, _) = p.add_plate(None);
        let (id_c, _) = p.add_plate(None);
        p.set_active_plate(id_c).unwrap();
        let events = p.remove_plate(id_c).unwrap();
        assert_eq!(p.plates.len(), 2);
        assert_eq!(p.active_plate, 1);
        assert!(events.iter().any(|e| matches!(
            e,
            SceneEvent::ActivePlateChanged { plate_id } if *plate_id == id_b,
        )));
    }

    #[test]
    fn remove_plate_before_active_shifts_active_down() {
        let mut p = Project::default();
        let (id_b, _) = p.add_plate(None);
        let (id_c, _) = p.add_plate(None);
        p.set_active_plate(id_c).unwrap();
        // Plate 0 is the default PlateId(1). Removing it shifts
        // active from index 2 → 1. The active plate's id is
        // unchanged (still id_c) but its index shifted.
        let events = p.remove_plate(PlateId(1)).unwrap();
        assert_eq!(p.active_plate, 1);
        assert_eq!(p.plates[p.active_plate].id, id_c);
        // active_plate_changed fires with the new id (which is the
        // same id_c — but the frontend mirror still needs to know
        // because the index changed).
        assert!(events.iter().any(|e| matches!(
            e,
            SceneEvent::ActivePlateChanged { plate_id } if *plate_id == id_c,
        )));
        // id_b stays at the front of the list.
        assert_eq!(p.plates[0].id, id_b);
    }

    #[test]
    fn remove_plate_renumbers_composition_order() {
        let mut p = Project::default();
        p.add_plate(None);
        p.add_plate(None);
        p.remove_plate(PlateId(2)).unwrap();
        assert_eq!(p.plates[0].metadata.composition_order, 1);
        assert_eq!(p.plates[1].metadata.composition_order, 2);
    }

    #[test]
    fn objects_added_to_different_plates_are_independent() {
        let mut p = Project::default();
        let (id_b, _) = p.add_plate(None);
        let (id_c, _) = p.add_plate(None);
        let (_, obj_a) = add_cube(&mut p);
        p.set_active_plate(id_b).unwrap();
        let (_, obj_b) = add_cube(&mut p);
        p.set_active_plate(id_c).unwrap();
        let (_, obj_c) = add_cube(&mut p);

        assert_eq!(p.plates[0].scene.objects.len(), 1);
        assert_eq!(p.plates[1].scene.objects.len(), 1);
        assert_eq!(p.plates[2].scene.objects.len(), 1);
        assert!(p.plates[0].scene.objects.contains_key(&obj_a));
        assert!(p.plates[1].scene.objects.contains_key(&obj_b));
        assert!(p.plates[2].scene.objects.contains_key(&obj_c));
        // load_mesh allocates per call, so 3 meshes scene-wide.
        assert_eq!(p.meshes.len(), 3);
        assert_ne!(obj_a, obj_b);
        assert_ne!(obj_b, obj_c);
    }

    #[test]
    fn selection_is_per_plate() {
        let mut p = Project::default();
        let (id_b, _) = p.add_plate(None);
        let (_, obj_a) = add_cube(&mut p);
        p.set_active_plate(id_b).unwrap();
        let (_, obj_b) = add_cube(&mut p);

        p.set_active_plate(PlateId(1)).unwrap();
        p.select(&[obj_a], SelectMode::Replace);
        p.set_active_plate(id_b).unwrap();
        p.select(&[obj_b], SelectMode::Replace);

        assert!(p.plates[0].scene.selection.contains(&obj_a));
        assert!(!p.plates[0].scene.selection.contains(&obj_b));
        assert!(p.plates[1].scene.selection.contains(&obj_b));
        assert!(!p.plates[1].scene.selection.contains(&obj_a));
    }

    #[test]
    fn primitive_cache_dedups_across_plates() {
        let mut p = Project::default();
        let (id_b, _) = p.add_plate(None);
        let params = PrimitiveParams::defaults_for(PrimitiveKind::Cube);
        let (mesh_a, _, _) = p.add_from_primitive(PrimitiveKind::Cube, params);
        p.set_active_plate(id_b).unwrap();
        let (mesh_b, _, _) = p.add_from_primitive(PrimitiveKind::Cube, params);
        assert_eq!(mesh_a, mesh_b);
        assert_eq!(p.meshes.len(), 1);
    }

    // ---- Per-object overrides ----------------------------

    #[test]
    fn object_override_set_then_get_round_trips() {
        let mut p = Project::default();
        let (_, obj) = add_cube(&mut p);
        let active_id = p.active_plate().id;
        let events = p
            .object_override_set(active_id, obj, "layer_height".into(), "0.12".into())
            .unwrap();
        assert!(matches!(
            events.as_slice(),
            [SceneEvent::ObjectOverridesChanged {
                plate_id,
                object_id,
            }] if *plate_id == active_id && *object_id == obj,
        ));
        let stored = p.plates[0]
            .scene
            .object_overrides
            .get(&obj)
            .and_then(|m| m.get("layer_height"));
        assert_eq!(stored.map(|s| s.as_str()), Some("0.12"));
    }

    #[test]
    fn object_override_clear_removes_key_and_drops_empty_map() {
        let mut p = Project::default();
        let (_, obj) = add_cube(&mut p);
        let active_id = p.active_plate().id;
        p.object_override_set(active_id, obj, "layer_height".into(), "0.12".into())
            .unwrap();
        p.object_override_set(active_id, obj, "infill_density".into(), "25%".into())
            .unwrap();

        p.object_override_clear(active_id, obj, "layer_height")
            .unwrap();
        let map = p.plates[0].scene.object_overrides.get(&obj).unwrap();
        assert_eq!(map.len(), 1);

        p.object_override_clear(active_id, obj, "infill_density")
            .unwrap();
        assert!(!p.plates[0].scene.object_overrides.contains_key(&obj));
    }

    #[test]
    fn object_override_clear_missing_key_is_silent_noop() {
        let mut p = Project::default();
        let (_, obj) = add_cube(&mut p);
        let active_id = p.active_plate().id;
        let events = p
            .object_override_clear(active_id, obj, "never_set")
            .unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn object_override_clear_all_drops_every_override() {
        let mut p = Project::default();
        let (_, obj) = add_cube(&mut p);
        let active_id = p.active_plate().id;
        p.object_override_set(active_id, obj, "k1".into(), "v1".into())
            .unwrap();
        p.object_override_set(active_id, obj, "k2".into(), "v2".into())
            .unwrap();
        let events = p.object_override_clear_all(active_id, obj).unwrap();
        assert_eq!(events.len(), 1);
        assert!(!p.plates[0].scene.object_overrides.contains_key(&obj));
    }

    #[test]
    fn object_overrides_are_per_plate() {
        let mut p = Project::default();
        let (id_b, _) = p.add_plate(None);
        let (_, obj_a) = add_cube(&mut p);
        p.set_active_plate(id_b).unwrap();
        let (_, obj_b) = add_cube(&mut p);

        p.object_override_set(PlateId(1), obj_a, "layer_height".into(), "0.10".into())
            .unwrap();
        p.object_override_set(id_b, obj_b, "layer_height".into(), "0.28".into())
            .unwrap();

        assert_eq!(
            p.plates[0].scene.object_overrides.get(&obj_a).unwrap()["layer_height"],
            "0.10",
        );
        assert_eq!(
            p.plates[1].scene.object_overrides.get(&obj_b).unwrap()["layer_height"],
            "0.28",
        );
    }

    #[test]
    fn object_override_errors_on_unknown_plate_or_object() {
        let mut p = Project::default();
        let (_, obj) = add_cube(&mut p);
        assert_eq!(
            p.object_override_set(PlateId(99), obj, "k".into(), "v".into())
                .unwrap_err(),
            SceneOpError::UnknownPlate(PlateId(99)),
        );
        assert_eq!(
            p.object_override_set(PlateId(1), ObjectId(9999), "k".into(), "v".into(),)
                .unwrap_err(),
            SceneOpError::UnknownObject(ObjectId(9999)),
        );
    }

    // ---- User-tier (project-wide) overrides -------------

    #[test]
    fn user_override_set_then_clear_round_trips() {
        let mut p = Project::default();
        let events = p
            .user_override_set("plugin.platecycler.enabled".into(), "true".into())
            .unwrap();
        assert!(matches!(
            events.as_slice(),
            [SceneEvent::UserOverridesChanged]
        ));
        assert_eq!(
            p.user_overrides
                .get("plugin.platecycler.enabled")
                .map(|s| s.as_str()),
            Some("true"),
        );
        // Unchanged value → silent no-op.
        assert!(p
            .user_override_set("plugin.platecycler.enabled".into(), "true".into())
            .unwrap()
            .is_empty());
        // Clear removes it and emits.
        let cleared = p.user_override_clear("plugin.platecycler.enabled").unwrap();
        assert!(matches!(
            cleared.as_slice(),
            [SceneEvent::UserOverridesChanged]
        ));
        assert!(p.user_overrides.is_empty());
    }

    // ---- Project-tier (per-plate) overrides -------------

    #[test]
    fn project_override_set_then_get_round_trips() {
        let mut p = Project::default();
        let events = p
            .project_override_set(PlateId(1), "layer_height".into(), "0.12".into())
            .unwrap();
        assert!(matches!(
            events.as_slice(),
            [SceneEvent::ProjectOverridesChanged {
                plate_id: PlateId(1)
            }],
        ));
        assert_eq!(
            p.plates[0]
                .project_overrides
                .get("layer_height")
                .map(|s| s.as_str()),
            Some("0.12"),
        );
    }

    #[test]
    fn project_override_set_to_current_value_is_silent_noop() {
        let mut p = Project::default();
        p.project_override_set(PlateId(1), "k".into(), "v".into())
            .unwrap();
        let again = p
            .project_override_set(PlateId(1), "k".into(), "v".into())
            .unwrap();
        assert!(again.is_empty());
    }

    #[test]
    fn project_override_clear_removes_key() {
        let mut p = Project::default();
        p.project_override_set(PlateId(1), "k".into(), "v".into())
            .unwrap();
        let events = p.project_override_clear(PlateId(1), "k").unwrap();
        assert_eq!(events.len(), 1);
        assert!(!p.plates[0].project_overrides.contains_key("k"));
    }

    #[test]
    fn project_override_clear_missing_key_is_silent_noop() {
        let mut p = Project::default();
        let events = p.project_override_clear(PlateId(1), "never_set").unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn project_override_clear_all_wipes_every_key() {
        let mut p = Project::default();
        p.project_override_set(PlateId(1), "k1".into(), "v1".into())
            .unwrap();
        p.project_override_set(PlateId(1), "k2".into(), "v2".into())
            .unwrap();
        let events = p.project_override_clear_all(PlateId(1)).unwrap();
        assert_eq!(events.len(), 1);
        assert!(p.plates[0].project_overrides.is_empty());
    }

    #[test]
    fn project_override_clear_all_on_empty_is_silent_noop() {
        let mut p = Project::default();
        let events = p.project_override_clear_all(PlateId(1)).unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn project_overrides_are_per_plate() {
        let mut p = Project::default();
        let (id_b, _) = p.add_plate(None);
        p.project_override_set(PlateId(1), "layer_height".into(), "0.10".into())
            .unwrap();
        p.project_override_set(id_b, "layer_height".into(), "0.28".into())
            .unwrap();
        assert_eq!(p.plates[0].project_overrides["layer_height"], "0.10");
        assert_eq!(p.plates[1].project_overrides["layer_height"], "0.28");
    }

    #[test]
    fn project_override_errors_on_unknown_plate() {
        let mut p = Project::default();
        assert_eq!(
            p.project_override_set(PlateId(99), "k".into(), "v".into())
                .unwrap_err(),
            SceneOpError::UnknownPlate(PlateId(99)),
        );
    }

    // ---- move_object ------------------------------------

    #[test]
    fn move_object_errors_on_same_plate() {
        let mut p = Project::default();
        let (_, obj) = add_cube(&mut p);
        let err = p.move_object(PlateId(1), PlateId(1), obj).unwrap_err();
        assert_eq!(err, SceneOpError::SamePlate(PlateId(1)));
    }

    #[test]
    fn move_object_errors_on_unknown_plate() {
        let mut p = Project::default();
        p.add_plate(None);
        let (_, obj) = add_cube(&mut p);
        assert_eq!(
            p.move_object(PlateId(1), PlateId(99), obj).unwrap_err(),
            SceneOpError::UnknownPlate(PlateId(99)),
        );
    }

    #[test]
    fn move_object_relocates_and_emits_remove_add() {
        let mut p = Project::default();
        let (id_b, _) = p.add_plate(None);
        let (_, obj) = add_cube(&mut p);
        let (report, events) = p.move_object(PlateId(1), id_b, obj).unwrap();
        assert_eq!(report.object_id, obj);
        assert!(report.repositioned.is_none());
        assert!(matches!(
            events[0],
            SceneEvent::ObjectRemoved { object_id, .. } if object_id == obj,
        ));
        assert!(matches!(events[1], SceneEvent::ObjectAdded { .. }));
        assert!(!p.plates[0].scene.objects.contains_key(&obj));
        assert!(p.plates[1].scene.objects.contains_key(&obj));
    }

    #[test]
    fn move_object_carries_per_object_overrides() {
        let mut p = Project::default();
        let (id_b, _) = p.add_plate(None);
        let (_, obj) = add_cube(&mut p);
        p.object_override_set(PlateId(1), obj, "layer_height".into(), "0.12".into())
            .unwrap();
        p.move_object(PlateId(1), id_b, obj).unwrap();
        assert!(!p.plates[0].scene.object_overrides.contains_key(&obj));
        let landed = p.plates[1].scene.object_overrides.get(&obj).unwrap();
        assert_eq!(landed["layer_height"], "0.12");
    }

    #[test]
    fn move_object_recenters_when_target_bed_smaller() {
        let mut p = Project::default();
        let (id_b, _) = p.add_plate(None);
        p.set_active_printer(Some(&a1_mini_for_test()));
        let (_, obj) = add_cube(&mut p);
        p.translate_object(obj, Vec3::new(160.0, 160.0, 0.0))
            .unwrap();
        p.set_active_plate(id_b).unwrap();
        p.set_active_printer(Some(&small_printer()));

        let (report, _) = p.move_object(PlateId(1), id_b, obj).unwrap();
        assert_eq!(report.repositioned, Some(RepositionReason::OutOfBounds));
        let landed = p.plates[1].scene.objects.get(&obj).unwrap();
        let center = landed.transform.apply_point(Vec3::new(0.5, 0.5, 0.5));
        assert!((center.x - 50.0).abs() < 1.0);
        assert!((center.y - 50.0).abs() < 1.0);
        assert!(center.z > 0.0 && center.z < 1.0);
    }

    // ---- Per-plate printer --------------------------------------

    #[test]
    fn set_active_printer_delegates_to_active_plate() {
        let mut p = Project::default();
        p.set_active_printer(Some(&a1_mini_for_test()));
        assert!(p.active_plate().scene.bed.is_some());
    }

    // ---- rebind_plate_printer (picker flow) ---------------------

    #[test]
    fn rebind_plate_printer_updates_binding_and_emits_events() {
        use crate::core::scene::events::SceneEvent;
        let mut p = Project::default();
        // Clear the bootstrap auto-bind so this test pins the
        // rebinding-from-unbound case explicitly (previous_printer
        // = None). The rebind-from-bound case is covered by
        // `rebind_plate_printer_records_previous_printer`.
        p.plates[0].set_printer(None, None);
        p.plates[0].scene.bed = None;
        let profile = a1_mini_for_test();
        let (report, events) = p
            .rebind_plate_printer(PlateId(1), "bambi".into(), &profile)
            .unwrap();
        // printer_instance_id assigned to the chosen instance.
        assert_eq!(p.plates[0].printer_instance_id(), Some("bambi"));
        // Report shape — previous was None for a fresh project.
        assert_eq!(report.plate_id, PlateId(1));
        assert_eq!(report.previous_printer, None);
        assert_eq!(report.new_printer, "bambu-lab-a1-mini");
        // new_build_plate reflects whatever the now-bound PrinterInstance
        // carries on its `bed.identity` — the bambi fixture ships with
        // Supertack Plate as the default.
        assert_eq!(report.new_build_plate, "Supertack Plate");
        assert!(report.incompatible.is_empty());
        assert!(report.clamped.is_empty());
        // Events emitted: BedChanged + PlateMetadataChanged.
        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[0],
            SceneEvent::BedChanged {
                plate_id: PlateId(1),
                ..
            }
        ));
        assert!(matches!(
            &events[1],
            SceneEvent::PlateMetadataChanged {
                plate_id: PlateId(1)
            }
        ));
    }

    #[test]
    fn rebind_plate_printer_clears_quality_profile_the_new_printer_lacks() {
        // The picker writes a per-plate process slug; rebinding to a
        // printer that doesn't ship it must clear it (else compose hard-
        // fails at slice/resolve), while a slug the new printer also
        // ships is preserved. `profile` is only used for the bed viz, so
        // a1_mini_for_test() suffices for both legs.
        let profile = a1_mini_for_test();
        let mut p = Project::default();
        p.plates[0].set_printer(Some("bambi".into()), None);
        p.plates[0].quality_profile = Some("0.20mm-strength".into());

        // Same-model rebind: the A1 mini ships `0.20mm-strength` → kept.
        p.rebind_plate_printer(PlateId(1), "bambi".into(), &profile)
            .unwrap();
        assert_eq!(
            p.plates[0].quality_profile.as_deref(),
            Some("0.20mm-strength"),
        );

        // Cross-printer rebind to the U1 (its process slugs omit the
        // "mm", so `0.20mm-strength` isn't one of them) → cleared.
        p.rebind_plate_printer(PlateId(1), "snappy".into(), &profile)
            .unwrap();
        assert_eq!(p.plates[0].quality_profile, None);
    }

    #[test]
    fn rebind_preserves_a_face_painted_material_binding() {
        let mut p = Project::default();
        p.plates[0].set_printer(Some("bambi".into()), None);
        // A painted object: base material 1, faces painted filament 2 ("8" →
        // EnforcerBlockerType state 2). No object carries extruder_id 2.
        let mut mesh = unit_cube_mesh();
        mesh.paint_colors = Some(vec!["8".to_string(); 12]); // 12 triangles
        let mesh_id = p.register_mesh(mesh);
        let _ = p.register_object(NewSceneObject::at_origin(mesh_id, "painted"));
        // The importer binds the painted material as a plate material.
        p.ensure_material_bound_on_active(2);
        assert!(
            p.plates[0].material_to_slot.contains_key(&2),
            "painted material 2 bound before the switch",
        );

        // Switch printers. The rebind clears + re-binds; before the fix it
        // re-bound only object materials ({1}) and dropped the painted 2.
        let profile = a1_mini_for_test();
        p.rebind_plate_printer(PlateId(1), "snappy".into(), &profile)
            .unwrap();
        assert!(
            p.plates[0].material_to_slot.contains_key(&2),
            "painted material 2 must survive a printer switch",
        );
    }

    #[test]
    fn rebind_plate_printer_records_previous_identity() {
        let mut p = Project::default();
        p.plates[0].set_printer(Some("snappy".into()), None);
        let profile = a1_mini_for_test();
        let (report, _) = p
            .rebind_plate_printer(PlateId(1), "bambi".into(), &profile)
            .unwrap();
        assert_eq!(report.previous_printer.as_deref(), Some("snapmaker-u1"));
        assert_eq!(report.new_printer, "bambu-lab-a1-mini");
    }

    #[test]
    fn rebind_plate_printer_unknown_plate_errors() {
        let mut p = Project::default();
        let profile = a1_mini_for_test();
        let err = p
            .rebind_plate_printer(PlateId(99), "bambi".into(), &profile)
            .unwrap_err();
        assert_eq!(err, SceneOpError::UnknownPlate(PlateId(99)));
    }

    // ---- Plate rename -----------------------------------

    #[test]
    fn set_plate_name_writes_and_emits() {
        let mut p = Project::default();
        let events = p.set_plate_name(PlateId(1), "Bench".into()).unwrap();
        assert_eq!(p.plates[0].name, "Bench");
        assert!(matches!(
            events.as_slice(),
            [SceneEvent::PlateMetadataChanged {
                plate_id: PlateId(1)
            }],
        ));
    }

    #[test]
    fn set_plate_name_trims_surrounding_whitespace() {
        let mut p = Project::default();
        p.set_plate_name(PlateId(1), "  Calibration tower  ".into())
            .unwrap();
        assert_eq!(p.plates[0].name, "Calibration tower");
    }

    #[test]
    fn set_plate_name_to_current_is_silent_noop() {
        let mut p = Project::default();
        // Set once, then re-set to the same trimmed value.
        p.set_plate_name(PlateId(1), "Bench".into()).unwrap();
        let again = p.set_plate_name(PlateId(1), "  Bench  ".into()).unwrap();
        assert!(again.is_empty());
    }

    #[test]
    fn set_plate_name_empty_after_trim_errors() {
        let mut p = Project::default();
        let err = p.set_plate_name(PlateId(1), "   ".into()).unwrap_err();
        assert!(matches!(
            err,
            SceneOpError::InvalidPlateMetadata {
                plate_id: PlateId(1),
                ..
            },
        ));
    }

    #[test]
    fn set_plate_name_over_max_errors() {
        let mut p = Project::default();
        let too_long = "x".repeat(PLATE_NAME_MAX + 1);
        let err = p.set_plate_name(PlateId(1), too_long).unwrap_err();
        assert!(matches!(
            err,
            SceneOpError::InvalidPlateMetadata {
                plate_id: PlateId(1),
                ..
            },
        ));
    }

    #[test]
    fn set_plate_name_at_max_succeeds() {
        let mut p = Project::default();
        let on_boundary = "x".repeat(PLATE_NAME_MAX);
        assert!(p.set_plate_name(PlateId(1), on_boundary.clone()).is_ok());
        assert_eq!(p.plates[0].name.len(), PLATE_NAME_MAX);
    }

    #[test]
    fn set_plate_name_unknown_plate_errors() {
        let mut p = Project::default();
        let err = p.set_plate_name(PlateId(99), "Bench".into()).unwrap_err();
        assert_eq!(err, SceneOpError::UnknownPlate(PlateId(99)));
    }

    #[test]
    fn plate_name_round_trips_via_json() {
        let mut p = Project::default();
        p.set_plate_name(PlateId(1), "Calibration".into()).unwrap();
        let json = serde_json::to_string(&p).unwrap();
        let parsed: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.plates[0].name, "Calibration");
    }

    // ---- Composition order -------------------------------

    fn plate_orders(p: &Project) -> Vec<(PlateId, u32)> {
        p.plates
            .iter()
            .map(|pl| (pl.id, pl.metadata.composition_order))
            .collect()
    }

    #[test]
    fn set_composition_order_moves_plate_down_shifts_others_up() {
        let mut p = Project::default();
        let (id_b, _) = p.add_plate(None); // composition_order=2
        let (id_c, _) = p.add_plate(None); // composition_order=3
        let (id_d, _) = p.add_plate(None); // composition_order=4
                                           // Initial: [A=1, B=2, C=3, D=4].
                                           // Move A (composition_order=1) → 3.
                                           // Expected: A=3, B=1, C=2, D=4.
        let events = p.set_plate_composition_order(PlateId(1), 3).unwrap();
        let orders = plate_orders(&p);
        assert_eq!(
            orders,
            vec![(PlateId(1), 3), (id_b, 1), (id_c, 2), (id_d, 4),]
        );
        // 3 plates affected: the moved one + 2 shifted siblings.
        assert_eq!(events.len(), 3);
    }

    #[test]
    fn set_composition_order_moves_plate_up_shifts_others_down() {
        let mut p = Project::default();
        let (id_b, _) = p.add_plate(None);
        let (id_c, _) = p.add_plate(None);
        let (id_d, _) = p.add_plate(None);
        // Initial: [A=1, B=2, C=3, D=4].
        // Move D (composition_order=4) → 2.
        // Expected: A=1, B=3, C=4, D=2.
        let events = p.set_plate_composition_order(id_d, 2).unwrap();
        let orders = plate_orders(&p);
        assert_eq!(
            orders,
            vec![(PlateId(1), 1), (id_b, 3), (id_c, 4), (id_d, 2),]
        );
        assert_eq!(events.len(), 3);
    }

    #[test]
    fn set_composition_order_preserves_dense_sequence() {
        let mut p = Project::default();
        p.add_plate(None);
        p.add_plate(None);
        p.add_plate(None);
        // Perform several reorders and verify the orders always
        // form a dense [1..N] set.
        p.set_plate_composition_order(PlateId(1), 4).unwrap();
        p.set_plate_composition_order(PlateId(3), 1).unwrap();
        p.set_plate_composition_order(PlateId(2), 2).unwrap();
        let mut orders: Vec<u32> = p
            .plates
            .iter()
            .map(|pl| pl.metadata.composition_order)
            .collect();
        orders.sort();
        assert_eq!(orders, vec![1, 2, 3, 4]);
    }

    #[test]
    fn set_composition_order_to_current_is_silent_noop() {
        let mut p = Project::default();
        p.add_plate(None);
        let events = p.set_plate_composition_order(PlateId(1), 1).unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn set_composition_order_zero_errors() {
        let mut p = Project::default();
        let err = p.set_plate_composition_order(PlateId(1), 0).unwrap_err();
        assert!(matches!(
            err,
            SceneOpError::InvalidPlateMetadata {
                plate_id: PlateId(1),
                ..
            },
        ));
    }

    #[test]
    fn set_composition_order_above_plate_count_errors() {
        let mut p = Project::default();
        p.add_plate(None);
        // 2 plates → valid range is 1..=2. 3 is too big.
        let err = p.set_plate_composition_order(PlateId(1), 3).unwrap_err();
        assert!(matches!(
            err,
            SceneOpError::InvalidPlateMetadata {
                plate_id: PlateId(1),
                ..
            },
        ));
    }

    #[test]
    fn set_composition_order_unknown_plate_errors() {
        let mut p = Project::default();
        let err = p.set_plate_composition_order(PlateId(99), 1).unwrap_err();
        assert_eq!(err, SceneOpError::UnknownPlate(PlateId(99)));
    }

    // ---- Material → slot routing ------------------------

    use crate::core::printer::SlotRef;

    fn cube_mesh() -> NewMesh {
        NewMesh {
            vertices: vec![0.0; 24],
            normals: vec![0.0; 24],
            indices: vec![0, 1, 2],
            paint_colors: None,
            bounding_box: BoundingBox {
                min: [0.0, 0.0, 0.0],
                max: [1.0, 1.0, 1.0],
            },
            provenance: MeshProvenance::Primitive("cube".into()),
        }
    }

    fn add_cube_with_material(p: &mut Project, mat: u8) -> ObjectId {
        let mesh_id = p.register_mesh(cube_mesh());
        p.register_object(NewSceneObject {
            mesh: mesh_id,
            transform: Transform::IDENTITY,
            name: format!("cube-m{mat}"),
            visible: true,
            extruder_id: Some(mat),
            group: None,
        })
    }

    #[test]
    fn register_object_auto_binds_material_to_slot_on_bambi() {
        // Default project boots into Bambi (1 extruder × 5 slots:
        // AMS:1..AMS:4 + Ext, AMS-first cosmetic ordering). Because
        // the instance carries AMS slots, auto-bind skips the
        // external spool — assigning material 1 to Ext would make
        // the firmware halt at print time asking the user to feed
        // the PTFE tube.
        //
        // Materials get distinct slots while any remain free: M1 →
        // AMS:1 (slot 0), M2 → AMS:2 (slot 1). M5's preferred slot
        // (modular over the 4 AMS slots) is AMS:1 (taken); the
        // first-free-from-preferred policy walks forward and lands
        // on AMS:3 (slot 2) instead of colliding with M1.
        let mut p = Project::default();
        add_cube_with_material(&mut p, 1);
        add_cube_with_material(&mut p, 2);
        add_cube_with_material(&mut p, 5);
        assert_eq!(
            p.plates[0].material_to_slot.get(&1),
            Some(&SlotRef {
                extruder: 0,
                slot: 0
            }),
        );
        assert_eq!(
            p.plates[0].material_to_slot.get(&2),
            Some(&SlotRef {
                extruder: 0,
                slot: 1
            }),
        );
        assert_eq!(
            p.plates[0].material_to_slot.get(&5),
            Some(&SlotRef {
                extruder: 0,
                slot: 2
            }),
            "preferred slot (AMS:1, modular) is taken by M1; walk forward to first free → AMS:3",
        );
    }

    #[test]
    fn auto_bind_wraps_when_every_slot_taken() {
        // Genuine more-materials-than-slots case: with M1..M4 already
        // bound on Bambi (all 4 AMS slots taken), M5 must still land
        // *somewhere* — the fallback wraps back to the preferred
        // modular index (AMS:1). Users sharing one physical slot
        // across two materials is the expected outcome when the
        // model has more materials than the printer has slots.
        let mut p = Project::default();
        for m in 1..=4 {
            add_cube_with_material(&mut p, m);
        }
        add_cube_with_material(&mut p, 5);
        assert_eq!(
            p.plates[0].material_to_slot.get(&5),
            Some(&SlotRef {
                extruder: 0,
                slot: 0
            }),
            "all 4 AMS slots taken → wrap to preferred (AMS:1 = slot 0)",
        );
    }

    #[test]
    fn deleting_last_user_of_a_material_drops_its_slot_binding() {
        // The user's reproduction (rebuilt): on Snappy, add a cube
        // for material 1, pin it to T2 manually, then delete the
        // cube. The binding for M1 must NOT linger after its last
        // user vanishes — otherwise a subsequent multi-material load
        // collides materials onto T2 (auto-bind's first-free-from-
        // preferred still has T2 as "taken" and steers the new
        // material around it, but the panel would also still show
        // M1 → T2 with no object to justify it: confusing UX, and
        // the slice-time cascade keeps emitting toolchanges to T2
        // for a material nothing references).
        let mut p = Project::default();
        let _guard = crate::core::printer::instance_registry::RegistryGuard::acquire();
        p.plates[0].set_printer(Some("snappy".into()), None);
        let cube = add_cube_with_material(&mut p, 1);
        // User manually pins M1 → T2 (instead of the auto-bind's T1).
        p.plates[0].material_to_slot.insert(
            1,
            SlotRef {
                extruder: 1,
                slot: 0,
            },
        );
        let events = p.delete_objects(&[cube]);
        assert!(!p.plates[0].material_to_slot.contains_key(&1));
        assert!(
            events
                .iter()
                .any(|e| matches!(e, SceneEvent::MaterialSlotChanged { .. })),
            "delete that orphans a material must emit MaterialSlotChanged so the panel refreshes",
        );
    }

    #[test]
    fn deleting_one_cube_keeps_binding_when_another_still_uses_the_material() {
        // Two cubes share material 1. Deleting one leaves M1 still in
        // use → binding survives, no MaterialSlotChanged event.
        let mut p = Project::default();
        let _guard = crate::core::printer::instance_registry::RegistryGuard::acquire();
        p.plates[0].set_printer(Some("snappy".into()), None);
        let cube_a = add_cube_with_material(&mut p, 1);
        let _cube_b = add_cube_with_material(&mut p, 1);
        let before = p.plates[0].material_to_slot.get(&1).copied();
        assert!(before.is_some(), "auto-bind populated M1");
        let events = p.delete_objects(&[cube_a]);
        assert_eq!(p.plates[0].material_to_slot.get(&1).copied(), before);
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, SceneEvent::MaterialSlotChanged { .. })),
            "no MaterialSlotChanged when the material still has a user",
        );
    }

    #[test]
    fn reassigning_an_objects_material_drops_the_old_orphaned_binding() {
        // The user's reproduction: assign a material via the Objects-panel
        // picker, then reassign that object to a different material. The old
        // material now has no user, so its slot binding must be pruned —
        // otherwise the Materials panel (which unions object extruders with
        // material_to_slot keys) keeps listing the abandoned material.
        let mut p = Project::default();
        let _guard = crate::core::printer::instance_registry::RegistryGuard::acquire();
        p.plates[0].set_printer(Some("snappy".into()), None);
        let cube = add_cube_with_material(&mut p, 2);
        assert!(
            p.plates[0].material_to_slot.contains_key(&2),
            "auto-bind populated M2",
        );
        let events = p.set_object_material(cube, 3).expect("object exists");
        assert!(
            !p.plates[0].material_to_slot.contains_key(&2),
            "M2 has no user after reassignment → its binding must be pruned",
        );
        assert!(
            p.plates[0].material_to_slot.contains_key(&3),
            "the new material M3 is auto-bound",
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, SceneEvent::MaterialSlotChanged { .. })),
            "reassignment emits MaterialSlotChanged so the panel refreshes",
        );
    }

    #[test]
    fn reassigning_keeps_old_binding_when_another_object_still_uses_it() {
        // Two cubes on M2; reassign one to M3. M2 is still used by the other
        // cube → its binding survives.
        let mut p = Project::default();
        let _guard = crate::core::printer::instance_registry::RegistryGuard::acquire();
        p.plates[0].set_printer(Some("snappy".into()), None);
        let cube_a = add_cube_with_material(&mut p, 2);
        let _cube_b = add_cube_with_material(&mut p, 2);
        let before = p.plates[0].material_to_slot.get(&2).copied();
        assert!(before.is_some(), "auto-bind populated M2");
        p.set_object_material(cube_a, 3).expect("object exists");
        assert_eq!(
            p.plates[0].material_to_slot.get(&2).copied(),
            before,
            "M2 still has a user → binding survives",
        );
    }

    #[test]
    fn auto_bind_skips_slot_already_pinned_by_user() {
        // Regression for the bug the user hit by hand: on Snappy
        // (4 extruders × 1 slot), pin M1 → T2 manually, then add a
        // cube with material 2. The auto-bind's preferred slot for
        // M2 is flat[(2-1) % 4] = T2 — same as M1's pin. Without
        // the first-free-from-preferred policy, M2 would collide
        // onto T2; with it, M2 advances to the next free slot (T3).
        let mut p = Project::default();
        let _guard = crate::core::printer::instance_registry::RegistryGuard::acquire();
        p.plates[0].set_printer(Some("snappy".into()), None);
        // User pins M1 → T2 before adding any objects.
        p.plates[0].material_to_slot.insert(
            1,
            SlotRef {
                extruder: 1,
                slot: 0,
            },
        );
        add_cube_with_material(&mut p, 2);
        assert_eq!(
            p.plates[0].material_to_slot.get(&1),
            Some(&SlotRef {
                extruder: 1,
                slot: 0
            }),
            "user pin survives",
        );
        assert_eq!(
            p.plates[0].material_to_slot.get(&2),
            Some(&SlotRef {
                extruder: 2,
                slot: 0
            }),
            "M2's preferred T2 is taken by user pin; walks forward to T3",
        );
    }

    #[test]
    fn set_material_slot_overrides_auto_bind_and_idempotent_on_repeat() {
        let mut p = Project::default();
        add_cube_with_material(&mut p, 1);
        // Auto-bind on Bambi puts material 1 on AMS:1 (slot 0 in
        // the AMS-first layout); setting the same value should be
        // a silent no-op.
        let target = SlotRef {
            extruder: 0,
            slot: 0,
        };
        let events = p.set_material_slot(PlateId(1), 1, target).unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn set_material_slot_out_of_range_extruder_errors() {
        let mut p = Project::default();
        add_cube_with_material(&mut p, 1);
        let err = p
            .set_material_slot(
                PlateId(1),
                1,
                SlotRef {
                    extruder: 5,
                    slot: 0,
                },
            )
            .unwrap_err();
        assert!(matches!(err, SceneOpError::InvalidPlateMetadata { .. }));
    }

    #[test]
    fn clear_material_slot_drops_entry_and_is_idempotent() {
        let mut p = Project::default();
        add_cube_with_material(&mut p, 1);
        let events = p.clear_material_slot(PlateId(1), 1).unwrap();
        assert_eq!(events.len(), 1);
        assert!(!p.plates[0].material_to_slot.contains_key(&1));
        // Second call has nothing to drop.
        let again = p.clear_material_slot(PlateId(1), 1).unwrap();
        assert!(again.is_empty());
    }

    #[test]
    fn material_to_slot_round_trips_through_project_serde() {
        // material_to_slot survives the JSON round-trip the project
        // save/load path uses.
        let mut p = Project::default();
        add_cube_with_material(&mut p, 1);
        let json = serde_json::to_string(&p).unwrap();
        let parsed: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.plates[0].material_to_slot.len(), 1);
        assert_eq!(
            parsed.plates[0].material_to_slot.get(&1),
            Some(&SlotRef {
                extruder: 0,
                slot: 0
            }),
        );
    }
}
