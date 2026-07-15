//! Override tiers for [`Project`]: the user-tier (project-wide),
//! project-tier (per-plate), and object-tier override setters and
//! clearers, plus the foreign-import object-override gate.

use crate::core::project::model::{PlateId, Project};
use crate::core::scene::events::{SceneEvent, SceneOpError};
use crate::core::scene::state::ObjectId;

impl Project {
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

    /// Upsert one override on a specific (plate, object).
    ///
    /// A grouped member routes *object-scope* keys to its group's
    /// override map: the group slices as one libslic3r ModelObject, so
    /// `enable_support`-class settings necessarily apply to every
    /// member — storing them per-member would be a lie the engine
    /// ignores. Region-scope keys (walls, infill, …) stay per-member;
    /// libslic3r honors those per volume.
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
        let group = match plate.objects.get(&object_id) {
            Some(obj) => obj.group,
            None => return Err(SceneOpError::UnknownObject(object_id)),
        };
        if let Some(group_id) = group {
            if crate::core::schema::is_object_scope_only(&key) {
                plate
                    .groups
                    .entry(group_id)
                    .or_default()
                    .overrides
                    .insert(key, value);
                return Ok(vec![SceneEvent::GroupOverridesChanged { plate_id, group_id }]);
            }
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
    ///
    /// On a grouped member the key is also cleared from the group's
    /// override map — the counterpart of `object_override_set`'s
    /// object-scope routing (and lazy cleanup for projects saved
    /// before group overrides existed, where object-scope keys sit in
    /// the member map).
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
        let group = match plate.objects.get(&object_id) {
            Some(obj) => obj.group,
            None => return Err(SceneOpError::UnknownObject(object_id)),
        };
        let mut events = Vec::new();
        if let Some(group_id) = group {
            if let Some(g) = plate.groups.get_mut(&group_id) {
                if g.overrides.remove(key).is_some() {
                    events.push(SceneEvent::GroupOverridesChanged { plate_id, group_id });
                }
            }
        }
        if let Some(map) = plate.object_overrides.get_mut(&object_id) {
            if map.remove(key).is_some() {
                if map.is_empty() {
                    plate.object_overrides.remove(&object_id);
                }
                events.push(SceneEvent::ObjectOverridesChanged {
                    plate_id,
                    object_id,
                });
            }
        }
        Ok(events)
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::project::mutation::test_support::*;

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
    fn grouped_member_routes_object_scope_key_to_group() {
        let _ = slic3r_ffi::init(None, 3);
        let mut p = Project::default();
        let (_, a) = add_cube(&mut p);
        let (_, b) = add_cube(&mut p);
        p.group_objects(&[a, b], "grp".into()).unwrap();
        let group_id = p.active_plate().scene.objects[&a].group.unwrap();
        let plate_id = p.active_plate().id;

        // Object-scope key → the group's map + GroupOverridesChanged.
        let events = p
            .object_override_set(plate_id, a, "enable_support".into(), "1".into())
            .unwrap();
        assert!(matches!(
            events.as_slice(),
            [SceneEvent::GroupOverridesChanged { group_id: g, .. }] if *g == group_id,
        ));
        assert_eq!(
            p.active_plate().scene.groups[&group_id].overrides["enable_support"],
            "1",
        );
        assert!(
            !p.active_plate().scene.object_overrides.contains_key(&a),
            "object-scope key must not be stored per-member",
        );

        // Region-scope key stays per-member.
        let events = p
            .object_override_set(plate_id, a, "wall_loops".into(), "4".into())
            .unwrap();
        assert!(matches!(
            events.as_slice(),
            [SceneEvent::ObjectOverridesChanged { object_id, .. }] if *object_id == a,
        ));
        assert_eq!(
            p.active_plate().scene.object_overrides[&a]["wall_loops"],
            "4",
        );

        // Clear removes the group-stored key through the member surface.
        let events = p.object_override_clear(plate_id, a, "enable_support").unwrap();
        assert!(matches!(
            events.as_slice(),
            [SceneEvent::GroupOverridesChanged { group_id: g, .. }] if *g == group_id,
        ));
        assert!(p.active_plate().scene.groups[&group_id]
            .overrides
            .is_empty());
    }

    #[test]
    fn ungrouping_copies_group_overrides_to_members() {
        let _ = slic3r_ffi::init(None, 3);
        let mut p = Project::default();
        let (_, a) = add_cube(&mut p);
        let (_, b) = add_cube(&mut p);
        p.group_objects(&[a, b], "grp".into()).unwrap();
        let group_id = p.active_plate().scene.objects[&a].group.unwrap();
        let plate_id = p.active_plate().id;
        p.object_override_set(plate_id, a, "enable_support".into(), "1".into())
            .unwrap();

        p.ungroup_objects(group_id);
        for id in [a, b] {
            assert_eq!(
                p.active_plate().scene.object_overrides[&id]["enable_support"],
                "1",
                "ex-member {id:?} keeps the group's setting",
            );
        }
        assert!(!p.active_plate().scene.groups.contains_key(&group_id));
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
}
