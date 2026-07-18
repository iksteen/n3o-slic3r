//! Plate lifecycle + metadata for [`Project`]: add/remove/activate
//! plates, rename, quality-profile selection, and per-plate printer
//! binding (bed viz + the picker rebind/unbind flows).

use crate::core::printer::profile::PrinterProfile;
use crate::core::printer::PrinterInstance;
use crate::core::project::model::{Plate, PlateId, Project};
use crate::core::project::session::{derive_bed, Session};
use crate::core::scene::bed;
use crate::core::scene::events::{SceneEvent, SceneOpError};

use super::PLATE_NAME_MAX;

impl Project {
    /// Allocate the next monotonic `PlateId`. IDs start at 1.
    pub(crate) fn next_plate_id(&self) -> PlateId {
        let max = self.plates.iter().map(|p| p.id.0).max().unwrap_or(0);
        PlateId(max + 1)
    }

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
        // Bind the printer (persisted). The bed lives in `SessionRuntime`;
        // the command layer calls `Session::reconcile` after add_plate so it
        // derives — the BedChanged below is the canonical event for listeners.
        if let Some(iid) = instance_id {
            plate.set_printer(Some(iid));
        }
        self.plates.push(plate);

        // PlateAdded triggers the frontend's snapshot refetch which pulls bed
        // + binding back in one go. BedChanged is emitted for symmetry with
        // the other bed-setting paths, its bed derived from the new binding.
        let mut events = vec![SceneEvent::PlateAdded { plate_id: id }];
        let new_plate = self.plates.last().expect("plate just pushed");
        if let Some(bed) = derive_bed(new_plate) {
            events.push(SceneEvent::BedChanged {
                plate_id: id,
                bed: Some(bed),
            });
        }
        (id, events)
    }

    /// Drop a plate by id. Errors when:
    ///   - The plate id isn't in the list.
    ///   - It's the only plate (FR-MP-1: a project must always have
    ///     at least one plate; there is no upper limit).
    ///
    /// On success, adjusts `active_plate` when the removed plate was
    /// the active one or sat before it (emits `ActivePlateChanged` in
    /// those cases).
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

    /// Rename a plate (backs the tab-strip dblclick-rename UI).
    /// Trims surrounding whitespace, rejects an empty result and any
    /// name longer than [`PLATE_NAME_MAX`] bytes. No-op (no event)
    /// when the trimmed value matches the current name. Emits
    /// `PlateChanged` on success — the channel the frontend already
    /// re-fetches the plate snapshot on.
    pub fn set_plate_name(
        &mut self,
        plate_id: PlateId,
        name: String,
    ) -> Result<Vec<SceneEvent>, SceneOpError> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(SceneOpError::InvalidPlateAttribute {
                plate_id,
                message: "plate name must not be empty".into(),
            });
        }
        if trimmed.len() > PLATE_NAME_MAX {
            return Err(SceneOpError::InvalidPlateAttribute {
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
        Ok(vec![SceneEvent::PlateChanged { plate_id }])
    }

    /// Set (or clear, with `None`) this plate's process/quality profile.
    /// `Some(slug)` sets the plate's process-fragment slug; `None` clears the
    /// override so the plate inherits the bound instance's profile again. No-op
    /// (no event) when unchanged. Emits `PlateChanged` — the same channel the
    /// frontend already re-fetches plate metadata on.
    ///
    /// Pure setter: it does **not** validate the slug against the bundled +
    /// custom process libraries — that's the caller's job (the model can't reach
    /// the user-process library without depending on a global). The
    /// `project_set_plate_quality_profile` command validates user-picked slugs
    /// before calling; the internal callers pass slugs they just created or
    /// `None`.
    pub fn set_plate_quality_profile(
        &mut self,
        plate_id: PlateId,
        quality_profile: Option<String>,
    ) -> Result<Vec<SceneEvent>, SceneOpError> {
        let idx = self
            .plate_index(plate_id)
            .ok_or(SceneOpError::UnknownPlate(plate_id))?;
        if self.plates[idx].quality_profile == quality_profile {
            return Ok(Vec::new());
        }
        self.plates[idx].quality_profile = quality_profile;
        Ok(vec![SceneEvent::PlateChanged { plate_id }])
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
    /// `PlateChanged` so the tab strip's printer label
    /// updates and the cascade re-resolves against the new context.
    pub fn rebind_plate_printer(
        &mut self,
        plate_id: PlateId,
        instance_id: String,
        profile: &PrinterProfile,
        previous: Option<&PrinterInstance>,
        new_instance: Option<&PrinterInstance>,
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

        let previous_printer = previous.map(|inst| inst.vendor_profile_ref.clone());
        // `new_instance` is the resolved binding for `instance_id` (the caller
        // looks it up). If the id doesn't resolve (caller passed a stale uuid
        // from a deleted instance) it's `None` — we still bind it; the slice
        // gate downstream refuses the unresolved reference with a meaningful
        // error.
        let new_printer = new_instance
            .map(|i| i.vendor_profile_ref.clone())
            .unwrap_or_else(|| instance_id.clone());
        let new_build_plate = new_instance
            .map(|i| i.bed.identity.clone())
            .unwrap_or_default();
        // Bind the printer (persisted); the session reconciles the bed after.
        self.plates[idx].set_printer(Some(instance_id));
        // Slot refs are physical (extruder, slot) coordinates — they
        // don't survive a topology change. Wipe + re-auto-bind any
        // referenced material against the new printer so existing
        // objects keep a sensible color instead of going gray. The
        // frontend refetches material_to_slot off `PlateChanged`
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
            let still_valid = new_instance.map_or(false, |i| {
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
            self.ensure_default_material_slot_on_active(mat, new_instance);
        }
        self.active_plate = prev_active;

        // Emit the canonical BedChanged (bed derived from the new profile; the
        // session reconciles the stored copy after) + the metadata change the
        // tab strip listens for.
        let events = vec![
            SceneEvent::BedChanged {
                plate_id,
                bed: Some(bed::bed_for_printer(profile)),
            },
            SceneEvent::PlateChanged { plate_id },
        ];

        let report = PrinterChangeReport {
            plate_id,
            previous_printer,
            new_printer,
            new_build_plate,
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
        // Unbind (persisted); the session reconciles the bed away after.
        self.plates[idx].set_printer(None);
        self.plates[idx].material_to_slot.clear();
        let events = vec![
            SceneEvent::BedChanged {
                plate_id,
                bed: None,
            },
            SceneEvent::PlateChanged { plate_id },
        ];
        Ok(events)
    }
}

impl Session {
    /// Install the active plate's bed from a resolved `PrinterProfile`,
    /// without touching the binding — a bed-viz-only path used by the arrange
    /// helpers/tests. `None` clears it. (The picker flow that also updates the
    /// binding is [`Project::rebind_plate_printer`] + `Session::reconcile`.)
    pub fn set_active_printer(&mut self, printer: Option<&PrinterProfile>) -> Vec<SceneEvent> {
        let plate_id = self.project.active_plate().id;
        let new_bed = printer.map(bed::bed_for_printer);
        self.runtime.plates.entry(plate_id).or_default().bed = new_bed.clone();
        vec![SceneEvent::BedChanged {
            plate_id,
            bed: new_bed,
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::project::mutation::test_support::*;
    use crate::core::project::session::derive_bed;
    use crate::core::scene::state::NewSceneObject;

    #[test]
    fn default_project_has_one_plate_active_zero() {
        let p = Project::default();
        assert_eq!(p.plates.len(), 1);
        assert_eq!(p.active_plate, 0);
        assert_eq!(p.plates[0].id, PlateId(1));
    }

    #[test]
    fn add_plate_appends_new_id_and_keeps_active() {
        let mut p = bound_default_project();
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
        p.plates[0].set_printer(Some("snappy".into()));
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
        p.plates[0].set_printer(None);
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
    fn bootstrap_binds_first_instance_and_derives_bed() {
        // `Project::default` is pure/unbound; the command-layer bootstrap
        // (`with_preferred_printer` with the registry's instances) binds the
        // first instance, and that binding derives a bed.
        let unbound = Project::default();
        assert!(
            unbound.plates[0].printer_instance_id().is_none(),
            "default project is unbound",
        );
        let p = bound_default_project();
        assert!(
            p.plates[0].printer_instance_id().is_some(),
            "bootstrap binds the first registered instance",
        );
        assert!(
            derive_bed(&p.plates[0]).is_some(),
            "the bound plate's bed derives",
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
    fn set_active_printer_delegates_to_active_plate() {
        let mut session = Session::new(bound_default_project());
        session.set_active_printer(Some(&a1_mini_for_test()));
        assert!(derive_bed(session.project.active_plate()).is_some());
    }

    #[test]
    fn rebind_plate_printer_updates_binding_and_emits_events() {
        use crate::core::scene::events::SceneEvent;
        let mut p = Project::default();
        // Clear the bootstrap auto-bind so this test pins the
        // rebinding-from-unbound case explicitly (previous_printer
        // = None). The rebind-from-bound case is covered by
        // `rebind_plate_printer_records_previous_printer`.
        p.plates[0].set_printer(None);
        let profile = a1_mini_for_test();
        let previous = active_instance(&p);
        let new_inst = crate::core::printer::lookup_instance("bambi");
        let (report, events) = p
            .rebind_plate_printer(
                PlateId(1),
                "bambi".into(),
                &profile,
                previous.as_ref(),
                new_inst.as_ref(),
            )
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
        // Events emitted: BedChanged + PlateChanged.
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
            SceneEvent::PlateChanged {
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
        p.plates[0].set_printer(Some("bambi".into()));
        p.plates[0].quality_profile = Some("0.20mm-strength".into());

        // Same-model rebind: the A1 mini ships `0.20mm-strength` → kept.
        let previous = active_instance(&p);
        let bambi = crate::core::printer::lookup_instance("bambi");
        p.rebind_plate_printer(
            PlateId(1),
            "bambi".into(),
            &profile,
            previous.as_ref(),
            bambi.as_ref(),
        )
        .unwrap();
        assert_eq!(
            p.plates[0].quality_profile.as_deref(),
            Some("0.20mm-strength"),
        );

        // Cross-printer rebind to the U1 (its process slugs omit the
        // "mm", so `0.20mm-strength` isn't one of them) → cleared.
        let previous = active_instance(&p);
        let snappy = crate::core::printer::lookup_instance("snappy");
        p.rebind_plate_printer(
            PlateId(1),
            "snappy".into(),
            &profile,
            previous.as_ref(),
            snappy.as_ref(),
        )
        .unwrap();
        assert_eq!(p.plates[0].quality_profile, None);
    }

    #[test]
    fn rebind_preserves_a_face_painted_material_binding() {
        let mut p = Project::default();
        p.plates[0].set_printer(Some("bambi".into()));
        // A painted object: base material 1, faces painted filament 2 ("8" →
        // EnforcerBlockerType state 2). No object carries extruder_id 2.
        let mut mesh = unit_cube_mesh();
        mesh.paint_colors = Some(vec!["8".to_string(); 12]); // 12 triangles
        let mesh_id = p.register_mesh(mesh);
        let inst = active_instance(&p);
        let _ = p.register_object(NewSceneObject::at_origin(mesh_id, "painted"), inst.as_ref());
        // The importer binds the painted material as a plate material.
        p.ensure_material_bound_on_active(2, inst.as_ref());
        assert!(
            p.plates[0].material_to_slot.contains_key(&2),
            "painted material 2 bound before the switch",
        );

        // Switch printers. The rebind clears + re-binds; before the fix it
        // re-bound only object materials ({1}) and dropped the painted 2.
        let profile = a1_mini_for_test();
        let previous = active_instance(&p);
        let snappy = crate::core::printer::lookup_instance("snappy");
        p.rebind_plate_printer(
            PlateId(1),
            "snappy".into(),
            &profile,
            previous.as_ref(),
            snappy.as_ref(),
        )
        .unwrap();
        assert!(
            p.plates[0].material_to_slot.contains_key(&2),
            "painted material 2 must survive a printer switch",
        );
    }

    #[test]
    fn rebind_plate_printer_records_previous_identity() {
        let mut p = Project::default();
        p.plates[0].set_printer(Some("snappy".into()));
        let profile = a1_mini_for_test();
        let previous = active_instance(&p);
        let bambi = crate::core::printer::lookup_instance("bambi");
        let (report, _) = p
            .rebind_plate_printer(
                PlateId(1),
                "bambi".into(),
                &profile,
                previous.as_ref(),
                bambi.as_ref(),
            )
            .unwrap();
        assert_eq!(report.previous_printer.as_deref(), Some("snapmaker-u1"));
        assert_eq!(report.new_printer, "bambu-lab-a1-mini");
    }

    #[test]
    fn rebind_plate_printer_unknown_plate_errors() {
        let mut p = Project::default();
        let profile = a1_mini_for_test();
        let previous = active_instance(&p);
        let bambi = crate::core::printer::lookup_instance("bambi");
        let err = p
            .rebind_plate_printer(
                PlateId(99),
                "bambi".into(),
                &profile,
                previous.as_ref(),
                bambi.as_ref(),
            )
            .unwrap_err();
        assert_eq!(err, SceneOpError::UnknownPlate(PlateId(99)));
    }

    #[test]
    fn set_plate_name_writes_and_emits() {
        let mut p = Project::default();
        let events = p.set_plate_name(PlateId(1), "Bench".into()).unwrap();
        assert_eq!(p.plates[0].name, "Bench");
        assert!(matches!(
            events.as_slice(),
            [SceneEvent::PlateChanged {
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
            SceneOpError::InvalidPlateAttribute {
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
            SceneOpError::InvalidPlateAttribute {
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
}
