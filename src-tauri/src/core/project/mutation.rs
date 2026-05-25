//! Mutation methods for [`Project`].
//!
//! Each public method takes `&mut Project` and returns the events
//! the renderer needs to apply (per the PR-2-2 contract: pure
//! functions that return event lists; the Tauri layer emits each
//! event via `Window::emit`). Tests bypass the Tauri layer and
//! inspect the returned event list directly.
//!
//! Lives in a sibling file from [`super::model`] so the type
//! definitions stay focused; this file has the mechanics.
//!
//! Plate addressing on the public surface is by [`PlateId`] (stable
//! across reorder + remove). Internal helpers use `usize` indices
//! when they need to mutate sibling plates — the borrow checker
//! wants index-then-deref, not a borrowed `Plate`.

use std::collections::HashSet;

use glam::{Quat, Vec3};

use super::model::{Plate, PlateId, Project};
use crate::core::printer::profile::{BoundingBox, PrinterProfile};
use crate::core::scene::bed::{self, BedMesh};
use crate::core::scene::events::{
    MirrorAxis, MoveReport, RepositionReason, SceneEvent, SceneOpError, SelectMode,
};
use crate::core::scene::primitives::{self, PrimitiveKind, PrimitiveParams};
use crate::core::scene::state::{
    mesh_bb_corners, z_extent, CameraState, GizmoState, Mesh, MeshId, MeshProvenance,
    NewMesh, NewSceneObject, ObjectId, SceneObject,
};
use crate::core::scene::transform::Transform;

/// Upper bound on `Plate.name` byte length (PR-5-3). Holds back
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
                parent: new_obj.parent,
            },
        );
        self.ensure_default_material_slot_on_active(extruder_id.unwrap_or(1));
        id
    }

    /// Plant a default `material → slot` mapping on the active plate
    /// when the material has no entry yet. Helper for
    /// [`Project::register_object`]; idempotent on re-call.
    ///
    /// Slot selection: walk the bound `PrinterInstance`'s flat slot
    /// list `(extruder, slot)`-major and pick index
    /// `(material - 1) MOD total_slots`. For Bambi (1 slot total)
    /// every material lands on `(0, 0)`. For Snappy (4 extruders ×
    /// 1 slot) material N maps to extruder `N-1` mod 4. For a
    /// future Bambi+AMS (1 extruder × 5 slots) material N rotates
    /// through the 5 slots, starting at the `Direct` slot.
    ///
    /// No-op when the plate has no `printer_instance_id` — the slice
    /// path refuses unbound plates anyway, and we don't want to
    /// pin a mapping before the user picks a printer.
    fn ensure_default_material_slot_on_active(&mut self, model_material: u8) {
        if model_material < 1 {
            return;
        }
        let idx = self.active_plate;
        if self.plates[idx].material_to_slot.contains_key(&model_material) {
            return;
        }
        let Some(instance_id) = self.plates[idx].printer_instance_id.clone()
        else {
            return;
        };
        let Some(instance) = crate::core::printer::lookup_instance(&instance_id)
        else {
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
        let pick = flat[(model_material as usize - 1) % flat.len()];
        self.plates[idx].material_to_slot.insert(model_material, pick);
    }

    // ---- Plate list mutations -------------------------------------

    /// Append a new plate. `printer` is optional — newly-added
    /// plates may stay unbound until the user picks a printer via
    /// PR-5-4's picker. Returns the new plate's id paired with the
    /// `PlateAdded` event the renderer subscribes to. Active plate
    /// is unchanged (the caller switches if desired).
    pub fn add_plate(
        &mut self,
        printer: Option<crate::core::project::binding::PrinterBinding>,
    ) -> (PlateId, Vec<SceneEvent>) {
        let id = self.next_plate_id();
        let position = (self.plates.len() + 1) as u32;

        // Auto-bind precedence (mirrors the auto-bind-materials
        // pattern from PR-5-6):
        //   1. Caller-supplied `printer` wins outright.
        //   2. Otherwise inherit from the currently-active plate
        //      (the tab the user clicked "+ New plate" from — most
        //      multi-plate workflows want every plate on the same
        //      printer).
        //   3. Otherwise fall back to the bundled-catalog default
        //      (fresh project case — first launch).
        let binding = printer
            .or_else(|| {
                self.plates
                    .get(self.active_plate)
                    .and_then(|p| p.printer.clone())
            })
            .or_else(crate::core::printer::default_binding);

        let mut plate = match &binding {
            Some(b) => Plate::with_printer(id, b.clone(), position),
            None => Plate::new(id, position),
        };
        // Populate the bed visualization so the viewport renders
        // immediately on plate switch — set_plate_printer would
        // otherwise be the only path setting plate.scene.bed, but
        // it's only called by the explicit picker flow.
        if let Some(b) = &binding {
            if let Some(profile) = crate::core::printer::lookup(&b.printer_identity) {
                let bed = crate::core::scene::bed::bed_for_printer(&profile);
                plate.scene.exclusion_zones = bed.exclusion_zones.clone();
                plate.scene.bed = Some(bed);
            }
            // PR-S-5c: route this plate through the composer path
            // (see model.rs::bind_default_printer_in_place for rationale).
            plate.printer_instance_id =
                crate::core::printer::instance_id_for_vendor_profile(&b.printer_identity)
                    .map(str::to_owned);
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
    ///   - It's the only plate (FR-MP-1: 1-4 plates; a project
    ///     must always have at least one).
    ///
    /// On success, repacks `composition_order` so the remaining
    /// plates form a dense `[1..N]` sequence + adjusts
    /// `active_plate` when the removed plate was the active one or
    /// sat before it (emits `ActivePlateChanged` in those cases).
    pub fn remove_plate(
        &mut self,
        id: PlateId,
    ) -> Result<Vec<SceneEvent>, SceneOpError> {
        if self.plates.len() <= 1 {
            return Err(SceneOpError::LastPlate);
        }
        let idx = self
            .plate_index(id)
            .ok_or(SceneOpError::UnknownPlate(id))?;
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
    pub fn set_active_plate(
        &mut self,
        id: PlateId,
    ) -> Result<Vec<SceneEvent>, SceneOpError> {
        let idx = self
            .plate_index(id)
            .ok_or(SceneOpError::UnknownPlate(id))?;
        if self.active_plate == idx {
            return Ok(Vec::new());
        }
        self.active_plate = idx;
        Ok(vec![SceneEvent::ActivePlateChanged { plate_id: id }])
    }

    // ---- Material → slot routing (PR-S-7) ------------------------

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
        if let Some(instance_id) = self.plates[plate_idx].printer_instance_id.clone() {
            if let Some(instance) =
                crate::core::printer::lookup_instance(&instance_id)
            {
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
        let prev = self.plates[plate_idx].material_to_slot.insert(model_material, slot);
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

    // ---- Plate metadata (PR-5-5) ----------------------------------

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
                message: format!(
                    "composition_order must be in 1..={n}, got {order}",
                ),
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

    /// Rename a plate (PR-5-3 tab strip dblclick-rename target).
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
                message: format!(
                    "plate name must be at most {PLATE_NAME_MAX} bytes",
                ),
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

    // ---- Per-plate printer assignment -----------------------------

    /// Install the active plate's bed by passing through a resolved
    /// `PrinterProfile`. `None` clears the bed. The plate's
    /// `printer` binding is unchanged here — bindings carry
    /// printer/build-plate *identity*, not the resolved profile;
    /// see [`Self::set_plate_printer`] for the binding update
    /// surface.
    pub fn set_active_printer(
        &mut self,
        printer: Option<&PrinterProfile>,
    ) -> Vec<SceneEvent> {
        let active_id = self.active_plate().id;
        // Indexed mutation can't fail for the active plate.
        self.set_plate_printer(active_id, printer)
            .expect("active plate id is always valid")
    }

    /// Install a printer profile on the specified plate.
    /// Recomputes the bed visualization, caches it on the plate,
    /// and emits a `BedChanged` event the renderer subscribes to.
    /// Pass `None` to clear the plate's bed.
    ///
    /// This is the bed-viz-only path — it does NOT touch the plate's
    /// `printer` binding. Use [`Self::rebind_plate_printer`] for the
    /// picker flow that also updates the binding + emits the
    /// metadata-changed signal the tab strip subscribes to.
    pub fn set_plate_printer(
        &mut self,
        plate_id: PlateId,
        printer: Option<&PrinterProfile>,
    ) -> Result<Vec<SceneEvent>, SceneOpError> {
        let idx = self
            .plate_index(plate_id)
            .ok_or(SceneOpError::UnknownPlate(plate_id))?;
        let new_bed = printer.map(bed::bed_for_printer);
        let plate = &mut self.plates[idx];
        plate.scene.exclusion_zones = new_bed
            .as_ref()
            .map(|b| b.exclusion_zones.clone())
            .unwrap_or_default();
        plate.scene.bed = new_bed.clone();
        Ok(vec![SceneEvent::BedChanged {
            plate_id,
            bed: new_bed,
        }])
    }

    /// Rebind a plate to a different printer by identity (PR-5-4 —
    /// the picker flow). The caller is responsible for resolving
    /// the identity to a `PrinterProfile` via the printer registry;
    /// keeping the registry lookup at the Tauri-command layer keeps
    /// this mutation pure + testable without registry plumbing.
    ///
    /// Validates that `binding.build_plate_identity` is in the
    /// chosen profile's `supported_build_plates`. Updates the
    /// plate's `printer` binding, recomputes the bed visualization,
    /// and returns a `PrinterChangeReport` documenting the swap.
    /// Emits `BedChanged` + `PlateMetadataChanged` so the tab
    /// strip's printer label updates and the cascade re-resolves
    /// against the new context.
    pub fn rebind_plate_printer(
        &mut self,
        plate_id: PlateId,
        binding: crate::core::project::PrinterBinding,
        profile: &PrinterProfile,
    ) -> Result<(crate::core::scene::events::PrinterChangeReport, Vec<SceneEvent>), SceneOpError> {
        use crate::core::scene::events::PrinterChangeReport;

        let idx = self
            .plate_index(plate_id)
            .ok_or(SceneOpError::UnknownPlate(plate_id))?;
        if !profile
            .supported_build_plates
            .iter()
            .any(|p| p == &binding.build_plate_identity)
        {
            return Err(SceneOpError::UnsupportedBuildPlate {
                plate_id,
                printer_identity: binding.printer_identity.clone(),
                build_plate_identity: binding.build_plate_identity.clone(),
            });
        }

        let previous_printer = self.plates[idx]
            .printer
            .as_ref()
            .map(|p| p.printer_identity.clone());
        let new_printer = binding.printer_identity.clone();
        let new_build_plate = binding.build_plate_identity.clone();

        // Update the binding first so events emitted by the bed
        // recompute see the new state. PR-S-5c: keep the new
        // `printer_instance_id` in sync so the composer path picks
        // the right PrinterInstance on the next slice.
        self.plates[idx].printer_instance_id =
            crate::core::printer::instance_id_for_vendor_profile(&binding.printer_identity)
                .map(str::to_owned);
        // Mirror the picker's bed pick onto the bound PrinterInstance.
        // The slicer composer reads bed.identity off the instance, not
        // off `plate.printer.build_plate_identity`, so without this
        // mirror the picker change is purely cosmetic — the slice
        // would silently use whatever bed the instance shipped with
        // (and on next launch the plate would re-bind to the
        // instance's bed anyway, losing the user's pick).
        if let Some(instance_id) = self.plates[idx].printer_instance_id.clone() {
            let bed_id = binding.build_plate_identity.clone();
            if let Err(e) =
                crate::core::printer::mutate_instance(&instance_id, move |inst| {
                    inst.bed.identity = bed_id;
                    Ok(())
                })
            {
                tracing::warn!(
                    instance_id = %instance_id,
                    error = %e,
                    "couldn't mirror bed pick onto printer instance",
                );
            }
        }
        self.plates[idx].printer = Some(binding);
        // Slot refs are physical (extruder, slot) coordinates — they
        // don't survive a topology change. Wipe + re-auto-bind any
        // referenced material against the new printer so existing
        // objects keep a sensible color instead of going gray. The
        // frontend refetches material_to_slot off `PlateMetadataChanged`
        // (always emitted below), so no separate MaterialSlotChanged
        // event is needed.
        self.plates[idx].material_to_slot.clear();
        let referenced: std::collections::BTreeSet<u8> = self.plates[idx]
            .scene
            .objects
            .values()
            .map(|o| o.extruder_id.unwrap_or(1))
            .collect();
        let prev_active = self.active_plate;
        self.active_plate = idx;
        for mat in referenced {
            self.ensure_default_material_slot_on_active(mat);
        }
        self.active_plate = prev_active;

        // Reuse the bed-viz path for the bed/exclusion-zone update.
        // Can't fail (we already validated the plate id above).
        let mut events = self
            .set_plate_printer(plate_id, Some(profile))
            .expect("plate id was validated above");
        events.push(SceneEvent::PlateMetadataChanged { plate_id });

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

    // ---- Mesh / object load + place -------------------------------

    /// Register a mesh and place one default `SceneObject` at
    /// origin on the active plate. Returns (mesh_id, object_id,
    /// events).
    pub fn load_mesh(
        &mut self,
        new_mesh: NewMesh,
    ) -> (MeshId, ObjectId, Vec<SceneEvent>) {
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
            parent: None,
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
        let mesh_bb = self.meshes.get(&mesh_id).unwrap().bounding_box.clone();
        let lift_z = -mesh_bb.min[2] as f32;
        let (shift_x, shift_y) = match &self.active_plate().scene.bed {
            Some(bed) => {
                let bed_cx =
                    ((bed.extents.min[0] + bed.extents.max[0]) * 0.5) as f32;
                let bed_cy =
                    ((bed.extents.min[1] + bed.extents.max[1]) * 0.5) as f32;
                let mesh_cx =
                    ((mesh_bb.min[0] + mesh_bb.max[0]) * 0.5) as f32;
                let mesh_cy =
                    ((mesh_bb.min[1] + mesh_bb.max[1]) * 0.5) as f32;
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
            parent: None,
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
    pub fn select(&mut self, ids: &[ObjectId], mode: SelectMode) -> Vec<SceneEvent> {
        let active = self.active_plate;
        let plate_id = self.plates[active].id;
        let plate = &mut self.plates[active].scene;
        let before: HashSet<ObjectId> = plate.selection.iter().copied().collect();
        match mode {
            SelectMode::Replace => {
                plate.selection = ids
                    .iter()
                    .copied()
                    .filter(|id| plate.objects.contains_key(id))
                    .collect();
            }
            SelectMode::Add => {
                for id in ids {
                    if plate.objects.contains_key(id) {
                        plate.selection.insert(*id);
                    }
                }
            }
            SelectMode::Toggle => {
                for id in ids {
                    if !plate.objects.contains_key(id) {
                        continue;
                    }
                    if !plate.selection.insert(*id) {
                        plate.selection.remove(id);
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

    /// Rotate an object around `axis` by `radians`. Pivot defaults
    /// to the object's current world-space center; explicit pivot
    /// via `pivot_override` is for the gizmo's "rotate around
    /// custom point" mode (PR-2-10).
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
                .clone()
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
    /// ticket; PR-2-7's library + Phase 4 UI can introduce
    /// "lay flat on selected face" later when the user can pick a
    /// face from the viewport.
    pub fn lay_flat_object(
        &mut self,
        id: ObjectId,
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
                .clone()
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

        // Decompose into scale/rotation/translation so we can
        // preserve scale + center position while replacing the
        // rotation with a candidate one. glam's decomposition is
        // sound for affine matrices without shear; our transforms
        // are built from translate/rotate/scale composition only,
        // so this holds.
        let (current_scale, _current_rot, current_trans) =
            current.to_scale_rotation_translation();
        let current_world_center = obj.transform.apply_point(local_center);

        let (best_rotation, best_min_z) = cube_rotations()
            .into_iter()
            .map(|rot| {
                let candidate = glam::Mat4::from_scale_rotation_translation(
                    current_scale,
                    rot,
                    current_trans,
                );
                let (min_z, max_z) = z_extent(&local_corners, &candidate);
                (rot, min_z, max_z)
            })
            .min_by(|a, b| {
                let extent_a = a.2 - a.1;
                let extent_b = b.2 - b.1;
                extent_a
                    .partial_cmp(&extent_b)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(rot, min_z, _)| (rot, min_z))
            .expect("24 rotations is non-empty");

        let chosen = glam::Mat4::from_scale_rotation_translation(
            current_scale,
            best_rotation,
            current_trans,
        );
        let post_rot_center = chosen.transform_point3(local_center);
        let delta = glam::Vec3::new(
            current_world_center.x - post_rot_center.x,
            current_world_center.y - post_rot_center.y,
            -best_min_z,
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

    /// Replace an object's transform wholesale. Used by
    /// auto-arrange (PR-2-8) and the gizmo's drag-finalization
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
    /// event.
    pub fn delete_objects(&mut self, ids: &[ObjectId]) -> Vec<SceneEvent> {
        let active = self.active_plate;
        let plate_id = self.plates[active].id;
        let plate = &mut self.plates[active].scene;
        let mut events = Vec::new();
        let mut selection_changed = false;
        for id in ids {
            if plate.objects.remove(id).is_some() {
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
            let mut sorted: Vec<ObjectId> =
                plate.selection.iter().copied().collect();
            sorted.sort();
            events.push(SceneEvent::SelectionChanged {
                plate_id,
                selected: sorted,
            });
        }
        events
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
            parent: original.parent,
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

    /// Set the gizmo mode + pivot on the active plate. Returns one
    /// event when state actually changed.
    pub fn set_gizmo(&mut self, new_gizmo: GizmoState) -> Vec<SceneEvent> {
        let active = self.active_plate;
        let plate_id = self.plates[active].id;
        let plate = &mut self.plates[active].scene;
        if (plate.gizmo.mode == new_gizmo.mode)
            && (plate.gizmo.pivot == new_gizmo.pivot)
        {
            return Vec::new();
        }
        plate.gizmo = new_gizmo.clone();
        vec![SceneEvent::GizmoChanged {
            plate_id,
            gizmo: new_gizmo,
        }]
    }

    /// Replace the camera state on the active plate. Always emits
    /// an event (camera state's equality check is expensive enough
    /// to skip).
    pub fn set_camera(&mut self, camera: CameraState) -> Vec<SceneEvent> {
        let active = self.active_plate;
        let plate_id = self.plates[active].id;
        self.plates[active].scene.camera = camera.clone();
        vec![SceneEvent::CameraChanged {
            plate_id,
            camera,
        }]
    }

    // ---- Per-object overrides (PR-5-7) ----------------------------

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

    // ---- Per-plate (project-tier) overrides (PR-5-9) -------------

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

    // ---- Move object between plates (PR-5-11) ---------------------

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
        self.plates[from_idx].scene.objects.remove(&object_id);

        let mesh_bb = self
            .meshes
            .get(&object.mesh)
            .ok_or(SceneOpError::UnknownMesh(object.mesh))?
            .bounding_box
            .clone();
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
                        let recentered =
                            recenter_on_bed(&object, &mesh_bb, target_bed);
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
            .bounding_box
            .clone();
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
fn recenter_on_bed(
    obj: &SceneObject,
    mesh_bb: &BoundingBox,
    target_bed: &BedMesh,
) -> SceneObject {
    let current = obj.transform.to_mat4();
    let (scale, rotation, _trans) = current.to_scale_rotation_translation();
    let local_center = Vec3::new(
        ((mesh_bb.min[0] + mesh_bb.max[0]) * 0.5) as f32,
        ((mesh_bb.min[1] + mesh_bb.max[1]) * 0.5) as f32,
        ((mesh_bb.min[2] + mesh_bb.max[2]) * 0.5) as f32,
    );
    let no_translation =
        glam::Mat4::from_scale_rotation_translation(scale, rotation, Vec3::ZERO);
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
    use crate::core::scene::state::{GizmoMode, MeshProvenance};

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

    fn a1_mini_for_test() -> PrinterProfile {
        use crate::core::printer::profile::{BoundingBox, Toolhead};
        PrinterProfile {
            model: "Bambu A1 mini".into(),
            slot_count: 4,
            supported_build_plates: vec!["Textured PEI".into()],
            toolheads: vec![Toolhead {
                nozzle_diameter: 0.4,
                hotend_type: "stainless_steel".into(),
                max_temp: 300.0,
                slot_indices: vec![0, 1, 2, 3],
            }],
            build_volume: BoundingBox {
                min: [0.0, 0.0, 0.0],
                max: [180.0, 180.0, 180.0],
            },
            exclusion_zones: vec![],
        }
    }

    fn small_printer() -> PrinterProfile {
        use crate::core::printer::profile::{BoundingBox, Toolhead};
        PrinterProfile {
            model: "Small".into(),
            slot_count: 1,
            supported_build_plates: vec!["Plain".into()],
            toolheads: vec![Toolhead {
                nozzle_diameter: 0.4,
                hotend_type: "stainless_steel".into(),
                max_temp: 300.0,
                slot_indices: vec![0],
            }],
            build_volume: BoundingBox {
                min: [0.0, 0.0, 0.0],
                max: [100.0, 100.0, 100.0],
            },
            exclusion_zones: vec![],
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
            .rotate_object(
                obj,
                Vec3::Z,
                std::f32::consts::FRAC_PI_2,
                Some(Vec3::ZERO),
            )
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
    fn gizmo_change_no_op_emits_nothing() {
        let mut p = Project::default();
        let initial = p.active_plate().scene.gizmo.clone();
        let events = p.set_gizmo(initial);
        assert!(events.is_empty());
        let mut next = p.active_plate().scene.gizmo.clone();
        next.mode = GizmoMode::Rotate;
        let events = p.set_gizmo(next);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], SceneEvent::GizmoChanged { .. }));
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
            parent: None,
        });
        p.active_plate_mut().scene.gizmo.mode = GizmoMode::Translate;

        let json = serde_json::to_string(&p).unwrap();
        let parsed: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.meshes.len(), 1);
        assert_eq!(parsed.active_plate().scene.objects.len(), 1);
        let obj = parsed.active_plate().scene.objects.values().next().unwrap();
        assert_eq!(obj.name, "test-cube");
        assert_eq!(obj.extruder_id, Some(2));
        assert_eq!(parsed.active_plate().scene.gizmo.mode, GizmoMode::Translate);
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
            vertices: vec![
                -5.0, -5.0, -3.0,
                 5.0, -5.0, -3.0,
                 0.0,  5.0,  3.0,
            ],
            normals: vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
            indices: vec![0, 1, 2],
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
        let events = p
            .translate_object(obj, Vec3::new(500.0, 0.0, 0.0))
            .unwrap();
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

    // ---- Plate list mutations (PR-5-2) ----------------------------

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
        assert!(matches!(events.first(), Some(SceneEvent::PlateAdded { plate_id }) if *plate_id == new_id));
        assert!(events.iter().any(|e| matches!(e, SceneEvent::BedChanged { plate_id, .. } if *plate_id == new_id)));
    }

    #[test]
    fn add_plate_inherits_printer_from_active_plate() {
        use crate::core::project::binding::PrinterBinding;
        let mut p = Project::default();
        // Override the bootstrap plate's binding so we can tell the
        // inheritance apart from the bundled-default fallback.
        p.plates[0].printer = Some(PrinterBinding {
            printer_identity: "snapmaker-u1".into(),
            build_plate_identity: "Magnetic".into(),
        });
        let (new_id, _) = p.add_plate(None);
        let new_plate = p.plate(new_id).unwrap();
        assert_eq!(
            new_plate.printer.as_ref().map(|b| b.printer_identity.as_str()),
            Some("snapmaker-u1"),
            "inherits from active plate",
        );
        assert_eq!(
            new_plate.printer.as_ref().map(|b| b.build_plate_identity.as_str()),
            Some("Magnetic"),
        );
    }

    #[test]
    fn add_plate_falls_back_to_default_binding_when_active_unbound() {
        let mut p = Project::default();
        // Clear the bootstrap binding so the fallback can fire.
        p.plates[0].printer = None;
        p.plates[0].scene.bed = None;
        let (new_id, _) = p.add_plate(None);
        let new_plate = p.plate(new_id).unwrap();
        assert!(
            new_plate.printer.is_some(),
            "fallback to bundled-default binding",
        );
    }

    #[test]
    fn add_plate_respects_explicit_binding() {
        use crate::core::project::binding::PrinterBinding;
        let mut p = Project::default();
        let explicit = PrinterBinding {
            printer_identity: "snapmaker-u1".into(),
            build_plate_identity: "Magnetic".into(),
        };
        let (new_id, _) = p.add_plate(Some(explicit.clone()));
        let new_plate = p.plate(new_id).unwrap();
        assert_eq!(
            new_plate.printer.as_ref().map(|b| b.printer_identity.as_str()),
            Some("snapmaker-u1"),
            "explicit binding wins over inheritance",
        );
    }

    #[test]
    fn project_default_bootstraps_with_bundled_printer() {
        let p = Project::default();
        assert!(
            p.plates[0].printer.is_some(),
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

    // ---- Per-object overrides (PR-5-7) ----------------------------

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

        p.object_override_clear(active_id, obj, "layer_height").unwrap();
        let map = p.plates[0].scene.object_overrides.get(&obj).unwrap();
        assert_eq!(map.len(), 1);

        p.object_override_clear(active_id, obj, "infill_density").unwrap();
        assert!(p.plates[0].scene.object_overrides.get(&obj).is_none());
    }

    #[test]
    fn object_override_clear_missing_key_is_silent_noop() {
        let mut p = Project::default();
        let (_, obj) = add_cube(&mut p);
        let active_id = p.active_plate().id;
        let events = p.object_override_clear(active_id, obj, "never_set").unwrap();
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
        assert!(p.plates[0].scene.object_overrides.get(&obj).is_none());
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
            p.object_override_set(
                PlateId(1),
                ObjectId(9999),
                "k".into(),
                "v".into(),
            )
            .unwrap_err(),
            SceneOpError::UnknownObject(ObjectId(9999)),
        );
    }

    // ---- Project-tier (per-plate) overrides (PR-5-9) -------------

    #[test]
    fn project_override_set_then_get_round_trips() {
        let mut p = Project::default();
        let events = p
            .project_override_set(PlateId(1), "layer_height".into(), "0.12".into())
            .unwrap();
        assert!(matches!(
            events.as_slice(),
            [SceneEvent::ProjectOverridesChanged { plate_id: PlateId(1) }],
        ));
        assert_eq!(
            p.plates[0].project_overrides.get("layer_height").map(|s| s.as_str()),
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
        assert!(p.plates[0].project_overrides.get("k").is_none());
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

    // ---- move_object (PR-5-11) ------------------------------------

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
        assert!(p.plates[0].scene.object_overrides.get(&obj).is_none());
        let landed = p.plates[1].scene.object_overrides.get(&obj).unwrap();
        assert_eq!(landed["layer_height"], "0.12");
    }

    #[test]
    fn move_object_recenters_when_target_bed_smaller() {
        let mut p = Project::default();
        let (id_b, _) = p.add_plate(None);
        p.set_active_printer(Some(&a1_mini_for_test()));
        let (_, obj) = add_cube(&mut p);
        p.translate_object(obj, Vec3::new(160.0, 160.0, 0.0)).unwrap();
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

    // ---- Per-plate printer (PR-5-4 backend) -----------------------

    #[test]
    fn set_plate_printer_targets_specific_plate() {
        let mut p = Project::default();
        // Project::default + add_plate both auto-populate the bed;
        // clear both so the assertion below pins set_plate_printer's
        // targeting (only the named plate's bed flips).
        p.plates[0].scene.bed = None;
        let (id_b, _) = p.add_plate(None);
        p.plates[1].scene.bed = None;
        p.set_plate_printer(id_b, Some(&a1_mini_for_test())).unwrap();
        assert!(p.plates[0].scene.bed.is_none(), "plate 0 bed untouched");
        assert!(p.plates[1].scene.bed.is_some(), "plate 1 bed set");
        assert_eq!(p.active_plate, 0);
    }

    #[test]
    fn set_plate_printer_with_none_clears_bed() {
        let mut p = Project::default();
        let (id_b, _) = p.add_plate(None);
        p.set_plate_printer(id_b, Some(&a1_mini_for_test())).unwrap();
        p.set_plate_printer(id_b, None).unwrap();
        assert!(p.plates[1].scene.bed.is_none());
    }

    #[test]
    fn set_plate_printer_unknown_errors() {
        let mut p = Project::default();
        let err = p
            .set_plate_printer(PlateId(99), Some(&a1_mini_for_test()))
            .unwrap_err();
        assert_eq!(err, SceneOpError::UnknownPlate(PlateId(99)));
    }

    #[test]
    fn set_active_printer_delegates_to_active_plate() {
        let mut p = Project::default();
        p.set_active_printer(Some(&a1_mini_for_test()));
        assert!(p.active_plate().scene.bed.is_some());
    }

    // ---- rebind_plate_printer (PR-5-4 picker flow) ----------------

    #[test]
    fn rebind_plate_printer_updates_binding_and_emits_events() {
        use crate::core::project::PrinterBinding;
        use crate::core::scene::events::SceneEvent;
        let mut p = Project::default();
        // Clear the bootstrap auto-bind so this test pins the
        // rebinding-from-unbound case explicitly (previous_printer
        // = None). The rebind-from-bound case is covered by
        // `rebind_plate_printer_records_previous_printer`.
        p.plates[0].printer = None;
        p.plates[0].scene.bed = None;
        let profile = a1_mini_for_test();
        let binding = PrinterBinding {
            printer_identity: "bambu-lab-a1-mini".into(),
            build_plate_identity: "Textured PEI".into(),
        };
        let (report, events) = p
            .rebind_plate_printer(PlateId(1), binding.clone(), &profile)
            .unwrap();
        // Binding updated.
        assert_eq!(p.plates[0].printer, Some(binding));
        // Report shape — previous was None for a fresh project.
        assert_eq!(report.plate_id, PlateId(1));
        assert_eq!(report.previous_printer, None);
        assert_eq!(report.new_printer, "bambu-lab-a1-mini");
        assert_eq!(report.new_build_plate, "Textured PEI");
        assert!(report.incompatible.is_empty());
        assert!(report.clamped.is_empty());
        // Events emitted: BedChanged + PlateMetadataChanged.
        assert_eq!(events.len(), 2);
        assert!(matches!(&events[0], SceneEvent::BedChanged { plate_id: PlateId(1), .. }));
        assert!(matches!(&events[1], SceneEvent::PlateMetadataChanged { plate_id: PlateId(1) }));
    }

    #[test]
    fn rebind_plate_printer_records_previous_identity() {
        use crate::core::project::PrinterBinding;
        let mut p = Project::default();
        p.plates[0].printer = Some(PrinterBinding {
            printer_identity: "snapmaker-u1".into(),
            build_plate_identity: "Magnetic".into(),
        });
        let profile = a1_mini_for_test();
        let (report, _) = p
            .rebind_plate_printer(
                PlateId(1),
                PrinterBinding {
                    printer_identity: "bambu-lab-a1-mini".into(),
                    build_plate_identity: "Textured PEI".into(),
                },
                &profile,
            )
            .unwrap();
        assert_eq!(report.previous_printer.as_deref(), Some("snapmaker-u1"));
        assert_eq!(report.new_printer, "bambu-lab-a1-mini");
    }

    #[test]
    fn rebind_plate_printer_rejects_unsupported_build_plate() {
        use crate::core::project::PrinterBinding;
        let mut p = Project::default();
        // Clear the bootstrap auto-bind so the "binding NOT updated"
        // assertion below isn't conflated with the default binding.
        p.plates[0].printer = None;
        p.plates[0].scene.bed = None;
        let profile = a1_mini_for_test();
        let err = p
            .rebind_plate_printer(
                PlateId(1),
                PrinterBinding {
                    printer_identity: "bambu-lab-a1-mini".into(),
                    build_plate_identity: "Magnetic".into(), // not in A1 mini's list
                },
                &profile,
            )
            .unwrap_err();
        assert!(matches!(
            err,
            SceneOpError::UnsupportedBuildPlate { plate_id: PlateId(1), .. },
        ));
        // Binding NOT updated on validation failure.
        assert!(p.plates[0].printer.is_none());
    }

    #[test]
    fn rebind_plate_printer_unknown_plate_errors() {
        use crate::core::project::PrinterBinding;
        let mut p = Project::default();
        let profile = a1_mini_for_test();
        let err = p
            .rebind_plate_printer(
                PlateId(99),
                PrinterBinding {
                    printer_identity: "bambu-lab-a1-mini".into(),
                    build_plate_identity: "Textured PEI".into(),
                },
                &profile,
            )
            .unwrap_err();
        assert_eq!(err, SceneOpError::UnknownPlate(PlateId(99)));
    }

    // ---- Plate rename (PR-5-3) -----------------------------------

    #[test]
    fn set_plate_name_writes_and_emits() {
        let mut p = Project::default();
        let events = p.set_plate_name(PlateId(1), "Bench".into()).unwrap();
        assert_eq!(p.plates[0].name, "Bench");
        assert!(matches!(
            events.as_slice(),
            [SceneEvent::PlateMetadataChanged { plate_id: PlateId(1) }],
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
            SceneOpError::InvalidPlateMetadata { plate_id: PlateId(1), .. },
        ));
    }

    #[test]
    fn set_plate_name_over_max_errors() {
        let mut p = Project::default();
        let too_long = "x".repeat(PLATE_NAME_MAX + 1);
        let err = p.set_plate_name(PlateId(1), too_long).unwrap_err();
        assert!(matches!(
            err,
            SceneOpError::InvalidPlateMetadata { plate_id: PlateId(1), .. },
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

    // ---- Composition order (PR-5-5) -------------------------------

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
        assert_eq!(orders, vec![
            (PlateId(1), 3),
            (id_b, 1),
            (id_c, 2),
            (id_d, 4),
        ]);
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
        assert_eq!(orders, vec![
            (PlateId(1), 1),
            (id_b, 3),
            (id_c, 4),
            (id_d, 2),
        ]);
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
            SceneOpError::InvalidPlateMetadata { plate_id: PlateId(1), .. },
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
            SceneOpError::InvalidPlateMetadata { plate_id: PlateId(1), .. },
        ));
    }

    #[test]
    fn set_composition_order_unknown_plate_errors() {
        let mut p = Project::default();
        let err = p.set_plate_composition_order(PlateId(99), 1).unwrap_err();
        assert_eq!(err, SceneOpError::UnknownPlate(PlateId(99)));
    }

    // ---- Material → slot routing (PR-S-7) ------------------------

    use crate::core::printer::SlotRef;

    fn cube_mesh() -> NewMesh {
        NewMesh {
            vertices: vec![0.0; 24],
            normals: vec![0.0; 24],
            indices: vec![0, 1, 2],
            bounding_box: BoundingBox {
                min: [0.0, 0.0, 0.0],
                max: [1.0, 1.0, 1.0],
            },
            provenance: MeshProvenance::Primitive("cube".into()),
        }
    }

    fn add_cube_with_material(p: &mut Project, mat: u8) {
        let mesh_id = p.register_mesh(cube_mesh());
        p.register_object(NewSceneObject {
            mesh: mesh_id,
            transform: Transform::IDENTITY,
            name: format!("cube-m{mat}"),
            visible: true,
            extruder_id: Some(mat),
            parent: None,
        });
    }

    #[test]
    fn register_object_auto_binds_material_to_slot_on_bambi() {
        // Default project boots into Bambi (1 extruder × 5 slots:
        // Ext + AMS:1..AMS:4). Because the instance carries AMS slots,
        // auto-bind skips the external spool (slot 0) — assigning
        // material 1 to Ext would make the firmware halt at print
        // time asking the user to feed the PTFE tube. Materials
        // rotate through the 4 AMS slots: material 1 → AMS:1
        // (slot 1), material 2 → AMS:2 (slot 2), … material 5 wraps
        // back to AMS:1.
        let mut p = Project::default();
        add_cube_with_material(&mut p, 1);
        add_cube_with_material(&mut p, 2);
        add_cube_with_material(&mut p, 5); // wraps back to AMS:1
        assert_eq!(
            p.plates[0].material_to_slot.get(&1),
            Some(&SlotRef { extruder: 0, slot: 1 }),
        );
        assert_eq!(
            p.plates[0].material_to_slot.get(&2),
            Some(&SlotRef { extruder: 0, slot: 2 }),
        );
        assert_eq!(
            p.plates[0].material_to_slot.get(&5),
            Some(&SlotRef { extruder: 0, slot: 1 }),
        );
    }

    #[test]
    fn set_material_slot_overrides_auto_bind_and_idempotent_on_repeat() {
        let mut p = Project::default();
        add_cube_with_material(&mut p, 1);
        // Auto-bind on Bambi puts material 1 on AMS:1 (slot 1);
        // setting the same value should be a silent no-op.
        let target = SlotRef { extruder: 0, slot: 1 };
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
                SlotRef { extruder: 5, slot: 0 },
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
            Some(&SlotRef { extruder: 0, slot: 1 }),
        );
    }
}
