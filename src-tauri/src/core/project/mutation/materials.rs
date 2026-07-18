//! `material → slot` mapping for [`Project`]: the auto-binder, the
//! per-plate slot map setters/clearers, the per-object material
//! setter, and the orphan-binding pruner.

use std::collections::BTreeSet;

use crate::core::printer::PrinterInstance;
use crate::core::project::model::{PlateId, Project};
use crate::core::scene::events::{SceneEvent, SceneOpError};
use crate::core::scene::state::ObjectId;

impl Project {
    /// Bind `model_material` to a default slot on the active plate (public
    /// entry to the auto-binder; no-op if already bound or the plate is
    /// unbound). The Orca importer uses this to materialize a project's
    /// **painted** filaments — ones applied to faces via `paint_color`
    /// rather than a per-object `extruder`, so no object carries
    /// `extruder = N` — as bound plate materials, so `material_count`
    /// counts them and the cascade fans + routes them at slice time.
    pub fn ensure_material_bound_on_active(
        &mut self,
        model_material: u8,
        instance: Option<&PrinterInstance>,
    ) {
        self.ensure_default_material_slot_on_active(model_material, instance);
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
    pub(super) fn ensure_default_material_slot_on_active(
        &mut self,
        model_material: u8,
        instance: Option<&PrinterInstance>,
    ) {
        self.ensure_material_slot_on_plate(self.active_plate, model_material, instance);
    }

    /// Auto-bind `model_material` to a slot on the plate at `idx` (the
    /// active-plate version above just forwards to here). Used both on the add
    /// path and when an object arrives on a non-active plate via a cross-plate
    /// move and needs a slot there.
    pub(super) fn ensure_material_slot_on_plate(
        &mut self,
        idx: usize,
        model_material: u8,
        instance: Option<&PrinterInstance>,
    ) {
        if model_material < 1 {
            return;
        }
        if self.plates[idx]
            .material_to_slot
            .contains_key(&model_material)
        {
            return;
        }
        // `instance` is the resolved binding for plate `idx` (the caller looks it
        // up); `None` when the plate is unbound or the id doesn't resolve — no
        // slot to auto-bind against, so leave the mapping unset.
        let Some(instance) = instance else {
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

    /// Upsert a `material → slot` mapping on `plate_id`. The slot
    /// reference is validated against the plate's bound
    /// PrinterInstance — out-of-range `extruder` or `slot` indices
    /// reject with `SceneOpError::InvalidPlateAttribute`.
    pub fn set_material_slot(
        &mut self,
        plate_id: PlateId,
        model_material: u8,
        slot: crate::core::printer::SlotRef,
        instance: Option<&PrinterInstance>,
    ) -> Result<Vec<SceneEvent>, SceneOpError> {
        if model_material < 1 {
            return Err(SceneOpError::InvalidPlateAttribute {
                plate_id,
                message: "model_material must be >= 1".into(),
            });
        }
        let plate_idx = self
            .plate_index(plate_id)
            .ok_or(SceneOpError::UnknownPlate(plate_id))?;
        // Range-check against the bound instance, if any (`instance` is the
        // resolved binding for `plate_id`). An unbound plate still accepts the
        // mapping (the slice path rejects separately); range-check would error
        // before the picker can round-trip the user's choice.
        if let Some(instance) = instance {
            let e_count = instance.extruders.len();
            if (slot.extruder as usize) >= e_count {
                return Err(SceneOpError::InvalidPlateAttribute {
                    plate_id,
                    message: format!(
                        "instance `{}` has {e_count} extruder(s); index {} is out of range",
                        instance.id, slot.extruder,
                    ),
                });
            }
            let s_count = instance.extruders[slot.extruder as usize].slots.len();
            if (slot.slot as usize) >= s_count {
                return Err(SceneOpError::InvalidPlateAttribute {
                    plate_id,
                    message: format!(
                        "instance `{}` extruder {} has {s_count} slot(s); index {} is out of range",
                        instance.id, slot.extruder, slot.slot,
                    ),
                });
            }
        }
        let prev = self.plates[plate_idx]
            .material_to_slot
            .insert(model_material, slot);
        if prev == Some(slot) {
            return Ok(Vec::new());
        }
        let filament_type_changed = self.rebind_changes_filament_type(prev, slot, instance);
        Ok(vec![SceneEvent::MaterialSlotChanged {
            plate_id,
            filament_type_changed,
        }])
    }

    /// Whether swapping a material's bound slot from `prev` to `new`
    /// changes the resolved filament **type** — which stales a slice's
    /// baked temps — as opposed to a pure routing change (same type: the
    /// firmware routes at print time, the G-code stays valid).
    /// Conservatively `true` when a type can't be resolved (no bound
    /// instance / unknown filament), so a real change never slips through
    /// as routing-only. `prev == None` compares against the instance's
    /// default fragment (what an unbound material sliced with).
    fn rebind_changes_filament_type(
        &self,
        prev: Option<crate::core::printer::SlotRef>,
        new: crate::core::printer::SlotRef,
        instance: Option<&PrinterInstance>,
    ) -> bool {
        let Some(instance) = instance else {
            return true;
        };
        let base_type = |ident: &str| crate::core::filament::lookup(ident).map(|f| f.base_type);
        let slot_base_type = |sr: crate::core::printer::SlotRef| -> Option<String> {
            let slot = instance
                .extruders
                .get(sr.extruder as usize)?
                .slots
                .get(sr.slot as usize)?;
            let ident = slot
                .filament_identity
                .as_deref()
                .unwrap_or(&instance.default_filament_fragment_slug);
            base_type(ident)
        };
        let new_type = slot_base_type(new);
        let prev_type = match prev {
            Some(sr) => slot_base_type(sr),
            None => base_type(&instance.default_filament_fragment_slug),
        };
        match (prev_type, new_type) {
            (Some(a), Some(b)) => a != b,
            // Couldn't resolve one side → be safe and invalidate.
            _ => true,
        }
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
        // Unbinding reverts the material to the instance default fragment,
        // which may differ from the slot it was sliced with.
        Ok(vec![SceneEvent::MaterialSlotChanged {
            plate_id,
            filament_type_changed: true,
        }])
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
        instance: Option<&PrinterInstance>,
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
        self.ensure_default_material_slot_on_active(material, instance);
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
            // Accompanies ObjectUpdated (which invalidates the slice); the
            // flag is moot but kept conservative.
            SceneEvent::MaterialSlotChanged {
                plate_id,
                filament_type_changed: true,
            },
        ])
    }

    /// Drop `material_to_slot` entries on `plate_idx` for any material
    /// in `candidates` that no remaining object on the plate uses.
    /// Returns `true` if anything was dropped — callers emit
    /// [`SceneEvent::MaterialSlotChanged`] when so.
    ///
    /// **Why:** the auto-bind in [`Project::ensure_default_material_slot_on_active`]
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
    pub(super) fn prune_orphan_material_bindings(
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::printer::SlotRef;
    use crate::core::project::mutation::test_support::*;
    use crate::core::project::Session;

    #[test]
    fn register_object_binds_painted_material() {
        // A single object painted with filament 2 must surface *both* materials
        // (base 1 + painted 2) as slot bindings — the materials list reads
        // `material_to_slot`, and the preview already shows both.
        let mut p = Project::default(); // boots bound to Bambi
        add_painted_cube(&mut p);
        assert!(p.plates[0].material_to_slot.contains_key(&1));
        assert!(
            p.plates[0].material_to_slot.contains_key(&2),
            "the MMU-painted filament 2 must be auto-bound, not just the base",
        );
    }

    #[test]
    fn delete_objects_prunes_orphan_painted_material_binding() {
        // Deleting the painted object must clean up the painted material's
        // binding too — it's named by the mesh paint, not any extruder_id, so
        // the delete path has to consult the paint or the binding lingers.
        let mut session = Session::new(Project::default());
        let id = add_painted_cube(&mut session.project);
        assert!(session.project.plates[0].material_to_slot.contains_key(&2));
        session.delete_objects(&[id]);
        assert!(
            !session.project.plates[0].material_to_slot.contains_key(&2),
            "painted material 2's binding should be pruned once its object is gone",
        );
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
        let mut session = Session::new(Project::default());
        let _guard = crate::core::printer::instance_registry::RegistryGuard::acquire();
        session.project.plates[0].set_printer(Some("snappy".into()));
        let cube = add_cube_with_material(&mut session.project, 1);
        // User manually pins M1 → T2 (instead of the auto-bind's T1).
        session.project.plates[0].material_to_slot.insert(
            1,
            SlotRef {
                extruder: 1,
                slot: 0,
            },
        );
        let events = session.delete_objects(&[cube]);
        assert!(!session.project.plates[0].material_to_slot.contains_key(&1));
        assert!(
            events
                .iter()
                .any(|e| matches!(e, SceneEvent::MaterialSlotChanged { .. })),
            "delete that orphans a material must emit MaterialSlotChanged so the panel refreshes",
        );
    }

    /// The key behavior for the send-dialog allocation: rebinding a
    /// material to a slot holding the SAME filament type is pure
    /// print-time routing — `filament_type_changed` is false, so the
    /// frontend leaves the sliced G-code valid.
    #[test]
    fn rebind_within_same_filament_type_is_routing_only() {
        let _guard = crate::core::printer::instance_registry::RegistryGuard::acquire();
        let mut p = Project::default();
        p.plates[0].set_printer(Some("snappy".into()));
        add_cube_with_material(&mut p, 1); // auto-binds M1 to toolhead 0 (generic-pla)

        // Toolhead 1 also carries generic-pla → same base type.
        let inst = active_instance(&p);
        let events = p
            .set_material_slot(
                PlateId(1),
                1,
                SlotRef {
                    extruder: 1,
                    slot: 0,
                },
                inst.as_ref(),
            )
            .expect("rebind");
        match events.as_slice() {
            [SceneEvent::MaterialSlotChanged {
                filament_type_changed,
                ..
            }] => assert!(
                !filament_type_changed,
                "same-type rebind must be routing-only (no re-slice)"
            ),
            other => panic!("expected one MaterialSlotChanged, got {other:?}"),
        }
    }

    /// A rebind that changes the bound filament TYPE stales the slice's
    /// baked temps → `filament_type_changed` is true (the send dialog
    /// blocks this via the same-type picker constraint, but the settings
    /// panel can still do it).
    #[test]
    fn rebind_to_a_different_filament_type_stales_the_slice() {
        let _guard = crate::core::printer::instance_registry::RegistryGuard::acquire();
        let mut p = Project::default();
        p.plates[0].set_printer(Some("snappy".into()));
        add_cube_with_material(&mut p, 1);

        // Load toolhead 1 with ABS — a different base type from M1's
        // PLA-default binding.
        crate::core::printer::mutate_instance("snappy", |inst| {
            inst.extruders[1].slots[0].filament_identity = Some("generic-abs".into());
            Ok(())
        })
        .expect("load ABS on toolhead 1");

        let inst = active_instance(&p);
        let events = p
            .set_material_slot(
                PlateId(1),
                1,
                SlotRef {
                    extruder: 1,
                    slot: 0,
                },
                inst.as_ref(),
            )
            .expect("rebind");
        match events.as_slice() {
            [SceneEvent::MaterialSlotChanged {
                filament_type_changed,
                ..
            }] => assert!(filament_type_changed, "PLA→ABS rebind must stale the slice"),
            other => panic!("expected one MaterialSlotChanged, got {other:?}"),
        }
    }

    #[test]
    fn deleting_one_cube_keeps_binding_when_another_still_uses_the_material() {
        // Two cubes share material 1. Deleting one leaves M1 still in
        // use → binding survives, no MaterialSlotChanged event.
        let mut session = Session::new(Project::default());
        let _guard = crate::core::printer::instance_registry::RegistryGuard::acquire();
        session.project.plates[0].set_printer(Some("snappy".into()));
        let cube_a = add_cube_with_material(&mut session.project, 1);
        let _cube_b = add_cube_with_material(&mut session.project, 1);
        let before = session.project.plates[0].material_to_slot.get(&1).copied();
        assert!(before.is_some(), "auto-bind populated M1");
        let events = session.delete_objects(&[cube_a]);
        assert_eq!(
            session.project.plates[0].material_to_slot.get(&1).copied(),
            before
        );
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
        p.plates[0].set_printer(Some("snappy".into()));
        let cube = add_cube_with_material(&mut p, 2);
        assert!(
            p.plates[0].material_to_slot.contains_key(&2),
            "auto-bind populated M2",
        );
        let inst = active_instance(&p);
        let events = p
            .set_object_material(cube, 3, inst.as_ref())
            .expect("object exists");
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
        p.plates[0].set_printer(Some("snappy".into()));
        let cube_a = add_cube_with_material(&mut p, 2);
        let _cube_b = add_cube_with_material(&mut p, 2);
        let before = p.plates[0].material_to_slot.get(&2).copied();
        assert!(before.is_some(), "auto-bind populated M2");
        let inst = active_instance(&p);
        p.set_object_material(cube_a, 3, inst.as_ref())
            .expect("object exists");
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
        p.plates[0].set_printer(Some("snappy".into()));
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
        let _guard = crate::core::printer::instance_registry::RegistryGuard::acquire();
        let mut p = Project::default();
        p.plates[0].set_printer(Some("bambi".into()));
        add_cube_with_material(&mut p, 1);
        // Auto-bind on Bambi puts material 1 on AMS:1 (slot 0 in
        // the AMS-first layout); setting the same value should be
        // a silent no-op.
        let target = SlotRef {
            extruder: 0,
            slot: 0,
        };
        let inst = active_instance(&p);
        let events = p
            .set_material_slot(PlateId(1), 1, target, inst.as_ref())
            .unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn set_material_slot_out_of_range_extruder_errors() {
        let _guard = crate::core::printer::instance_registry::RegistryGuard::acquire();
        let mut p = Project::default();
        p.plates[0].set_printer(Some("bambi".into()));
        add_cube_with_material(&mut p, 1);
        let inst = active_instance(&p);
        let err = p
            .set_material_slot(
                PlateId(1),
                1,
                SlotRef {
                    extruder: 5,
                    slot: 0,
                },
                inst.as_ref(),
            )
            .unwrap_err();
        assert!(matches!(err, SceneOpError::InvalidPlateAttribute { .. }));
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
