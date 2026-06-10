//! Auto-arrange — drives libslic3r's nester (the engine behind
//! OrcaSlicer's "Arrange", on libnest2d) through the FFI.
//!
//! Each *unit* (a lone object, or a whole group kept rigid) contributes
//! one convex footprint = the convex hull of its mesh vertices projected
//! to the bed plane. Those hulls, the bed rectangle, and the printer's
//! exclusion zones go to `slic3r_ffi::arrange`, which packs them with
//! per-object spacing and returns a translation + logical bed index per
//! unit. We apply the translation (XY only — authored rotation/scale are
//! preserved; rotation is off for now) to every member of the unit.
//!
//! Phase 1 is single-plate: a unit the nester spills onto an extra bed
//! (`bed_idx > 0`) is reported as `un_placed` so the UI flags it, exactly
//! like the old packer's overflow. Spilling onto *real* extra plates is a
//! follow-up (needs the move-to-plate machinery).

use super::bed::BedMesh;
use super::state::{GroupId, ObjectId};
use super::transform::Transform;
use crate::core::project::Project;
use glam::Vec3;
use std::collections::HashMap;

// Used only by the test-only bbox helpers below.
#[cfg(test)]
use super::state::SceneObject;
#[cfg(test)]
use crate::core::printer::profile::BoundingBox;

/// Spacing between adjacent placed objects, in millimeters. Matches
/// OrcaSlicer's default skirt clearance so prints don't fuse.
pub const PLACEMENT_SPACING: f32 = 5.0;

/// Outcome of one auto-arrange pass.
pub struct ArrangeResult {
    /// Objects that fit on the plate, with their new transforms.
    pub placed: Vec<(ObjectId, Transform)>,
    /// Objects we couldn't fit. Listed in original sort order so the
    /// UI can highlight them top-to-bottom in the outliner.
    pub un_placed: Vec<ObjectId>,
}

/// Compute a packing without mutating the scene. Pure function so the
/// caller can decide whether to apply the result (e.g., test code
/// vs. a user-facing "preview" flow).
pub fn plan_arrangement(state: &Project, bed: &BedMesh) -> ArrangeResult {
    let plate = &state.active_plate().scene;

    // Group visible objects into arrange units: one per group (kept rigid),
    // plus each ungrouped object on its own. Sorted by the unit's smallest
    // object id so the nester sees a deterministic order.
    let mut groups: HashMap<GroupId, Vec<ObjectId>> = HashMap::new();
    let mut units: Vec<Vec<ObjectId>> = Vec::new();
    for o in plate.objects.values().filter(|o| o.visible) {
        match o.group {
            Some(g) => groups.entry(g).or_default().push(o.id),
            None => units.push(vec![o.id]),
        }
    }
    units.extend(groups.into_values());
    for u in &mut units {
        u.sort_by_key(|id| id.0);
    }
    units.sort_by_key(|u| u[0].0);

    // The nester works in a bed-local frame with origin (0,0); shift footprints
    // (and exclusion zones) by -bed_min and shift the resulting translation back
    // implicitly (it's a delta, invariant under the constant shift).
    let bed_min = (bed.extents.min[0] as f64, bed.extents.min[1] as f64);
    let bed_size = [
        (bed.extents.max[0] - bed.extents.min[0]) as f64,
        (bed.extents.max[1] - bed.extents.min[1]) as f64,
    ];

    // One convex footprint per unit; units with no usable footprint (empty /
    // degenerate mesh) are left out of the pack and untouched.
    let mut contours: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut arranged_units: Vec<&Vec<ObjectId>> = Vec::new();
    for unit in &units {
        let mut pts: Vec<[f64; 2]> = Vec::new();
        for &id in unit {
            let Some(obj) = plate.objects.get(&id) else {
                continue;
            };
            let Some(mesh) = state.meshes.get(&obj.mesh) else {
                continue;
            };
            for v in mesh.vertices.chunks_exact(3) {
                let w = obj.transform.apply_point(Vec3::new(v[0], v[1], v[2]));
                pts.push([w.x as f64 - bed_min.0, w.y as f64 - bed_min.1]);
            }
        }
        if let Some(contour) = footprint_contour(&pts) {
            contours.push(contour);
            arranged_units.push(unit);
        }
    }

    let mut placed = Vec::new();
    let mut un_placed = Vec::new();
    if contours.is_empty() {
        return ArrangeResult { placed, un_placed };
    }

    let excludes: Vec<[f64; 4]> = bed
        .exclusion_zones
        .iter()
        .map(|z| {
            [
                z.bounds.min[0] as f64 - bed_min.0,
                z.bounds.min[1] as f64 - bed_min.1,
                z.bounds.max[0] as f64 - bed_min.0,
                z.bounds.max[1] as f64 - bed_min.1,
            ]
        })
        .collect();

    let placements = match slic3r_ffi::arrange(
        &contours,
        &excludes,
        bed_size,
        PLACEMENT_SPACING as f64,
        false, // preserve authored rotation for now
    ) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "libslic3r arrange failed; leaving objects in place");
            for unit in &arranged_units {
                un_placed.extend(unit.iter().copied());
            }
            return ArrangeResult { placed, un_placed };
        }
    };

    for (unit, placement) in arranged_units.iter().zip(&placements) {
        if placement.bed_idx == 0 {
            // A pure-XY world translation, applied to every member so a group
            // stays rigid. (Rotation is off, so there's no yaw to compose.)
            let delta = Transform::translation(Vec3::new(
                placement.translation[0] as f32,
                placement.translation[1] as f32,
                0.0,
            ));
            for &id in unit.iter() {
                if let Some(obj) = plate.objects.get(&id) {
                    placed.push((id, delta.compose(obj.transform)));
                }
            }
        } else {
            // Spilled onto an extra bed (phase 1: report as overflow).
            un_placed.extend(unit.iter().copied());
        }
    }

    ArrangeResult { placed, un_placed }
}

/// Convex hull of a unit's projected points, as a CCW contour the nester can
/// pack. Falls back to the points' bounding rectangle if the hull degenerates
/// (collinear), and returns `None` for an empty or zero-area footprint.
fn footprint_contour(points: &[[f64; 2]]) -> Option<Vec<[f64; 2]>> {
    let hull = convex_hull(points);
    if hull.len() >= 3 {
        return Some(hull);
    }
    // Degenerate hull — use the axis-aligned bounding rectangle if it has area.
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for p in points {
        min[0] = min[0].min(p[0]);
        min[1] = min[1].min(p[1]);
        max[0] = max[0].max(p[0]);
        max[1] = max[1].max(p[1]);
    }
    if !(max[0] - min[0] > 1e-6 && max[1] - min[1] > 1e-6) {
        return None;
    }
    Some(vec![
        [min[0], min[1]],
        [max[0], min[1]],
        [max[0], max[1]],
        [min[0], max[1]],
    ])
}

/// 2D convex hull (Andrew's monotone chain), returned CCW without the closing
/// duplicate. Fewer than 3 unique points yields a degenerate result the caller
/// handles.
fn convex_hull(points: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let mut pts = points.to_vec();
    pts.sort_by(|a, b| {
        a[0]
            .partial_cmp(&b[0])
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a[1].partial_cmp(&b[1]).unwrap_or(std::cmp::Ordering::Equal))
    });
    pts.dedup();
    if pts.len() < 3 {
        return pts;
    }
    let cross = |o: [f64; 2], a: [f64; 2], b: [f64; 2]| {
        (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
    };
    let mut hull: Vec<[f64; 2]> = Vec::with_capacity(pts.len() + 1);
    for &p in &pts {
        while hull.len() >= 2 && cross(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0.0 {
            hull.pop();
        }
        hull.push(p);
    }
    let lower = hull.len() + 1;
    for &p in pts.iter().rev() {
        while hull.len() >= lower && cross(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0.0 {
            hull.pop();
        }
        hull.push(p);
    }
    hull.pop(); // drop the closing point (== first)
    hull
}

/// Apply a [`plan_arrangement`] result to the scene state. Calls
/// `set_object_transform` per placed object so OOB check
/// fires naturally — packing keeps objects on the bed but the user
/// might have an object on a plate with a custom-shape exclusion
/// zone, and the OOB event surfaces that correctly.
pub fn apply_arrangement(
    state: &mut Project,
    plan: ArrangeResult,
) -> (Vec<super::events::SceneEvent>, Vec<ObjectId>) {
    let mut all_events = Vec::new();
    for (id, xform) in plan.placed {
        match state.set_object_transform(id, xform) {
            Ok(events) => all_events.extend(events),
            Err(e) => {
                tracing::warn!(
                    object_id = id.0,
                    error = %e,
                    "auto-arrange could not apply transform (deleted between plan and apply?)"
                );
            }
        }
    }
    (all_events, plan.un_placed)
}

// Test-only footprint helpers: the live packer uses convex hulls via the FFI,
// but the tests still verify placements with simple bbox math.
#[cfg(test)]
struct XyFootprint {
    min: Vec3,
    size: Vec3,
}

#[cfg(test)]
fn xy_footprint(obj: &SceneObject, mesh_bb: &BoundingBox) -> XyFootprint {
    let bb = mesh_bb;
    let mut min = Vec3::new(f32::INFINITY, f32::INFINITY, 0.0);
    let mut max = Vec3::new(f32::NEG_INFINITY, f32::NEG_INFINITY, 0.0);
    for &x in &[bb.min[0] as f32, bb.max[0] as f32] {
        for &y in &[bb.min[1] as f32, bb.max[1] as f32] {
            for &z in &[bb.min[2] as f32, bb.max[2] as f32] {
                let p = obj.transform.apply_point(Vec3::new(x, y, z));
                min.x = min.x.min(p.x);
                min.y = min.y.min(p.y);
                max.x = max.x.max(p.x);
                max.y = max.y.max(p.y);
            }
        }
    }
    XyFootprint {
        min,
        size: Vec3::new(max.x - min.x, max.y - min.y, 0.0),
    }
}

#[cfg(test)]
struct AAbb {
    min: Vec3,
    max: Vec3,
}

#[cfg(test)]
fn aabb_intersects_xy(a: &AAbb, b: &BoundingBox) -> bool {
    let bmin_x = b.min[0] as f32;
    let bmax_x = b.max[0] as f32;
    let bmin_y = b.min[1] as f32;
    let bmax_y = b.max[1] as f32;
    !(a.max.x <= bmin_x || a.min.x >= bmax_x || a.max.y <= bmin_y || a.min.y >= bmax_y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::printer::profile::{BoundingBox, PrinterProfile, Toolhead};
    use crate::core::scene::primitives::{PrimitiveKind, PrimitiveParams};

    fn a1_mini() -> PrinterProfile {
        PrinterProfile {
            model: "Bambu Lab A1 mini".into(),
            supported_build_plates: vec!["Textured PEI Plate".into()],
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

    fn add_n_cubes(state: &mut Project, n: usize, edge: f32) {
        for _ in 0..n {
            state.add_from_primitive(
                PrimitiveKind::Cube,
                PrimitiveParams {
                    width: edge,
                    depth: edge,
                    height: edge,
                    radius: 0.0,
                    radial_segments: 0,
                },
            );
        }
    }

    fn footprints_overlap_after_plan(plan: &ArrangeResult, state: &Project) -> bool {
        let placed_objs: Vec<(ObjectId, XyFootprint)> = plan
            .placed
            .iter()
            .map(|(id, xform)| {
                let obj_clone = SceneObject {
                    id: *id,
                    mesh: state.active_plate().scene.objects.get(id).unwrap().mesh,
                    transform: *xform,
                    name: String::new(),
                    visible: true,
                    extruder_id: None,
                    group: None,
                };
                let mesh = state.meshes.get(&obj_clone.mesh).unwrap();
                let fp = xy_footprint(&obj_clone, &mesh.bounding_box);
                (*id, fp)
            })
            .collect();
        for (i, (_, a)) in placed_objs.iter().enumerate() {
            for (_, b) in &placed_objs[i + 1..] {
                // Strictly overlapping XY rects (touching edges is fine).
                let no_overlap = (a.min.x + a.size.x) <= b.min.x + 1e-3
                    || (b.min.x + b.size.x) <= a.min.x + 1e-3
                    || (a.min.y + a.size.y) <= b.min.y + 1e-3
                    || (b.min.y + b.size.y) <= a.min.y + 1e-3;
                if !no_overlap {
                    return true;
                }
            }
        }
        false
    }

    #[test]
    fn ten_small_cubes_all_fit_on_a1_mini() {
        let mut s = Project::new();
        s.set_active_printer(Some(&a1_mini()));
        add_n_cubes(&mut s, 10, 20.0);
        let bed = s.active_plate().scene.bed.clone().unwrap();
        let plan = plan_arrangement(&s, &bed);
        assert_eq!(plan.placed.len(), 10);
        assert!(plan.un_placed.is_empty());
        assert!(
            !footprints_overlap_after_plan(&plan, &s),
            "no overlap after arrange"
        );
    }

    #[test]
    fn one_hundred_cubes_overflow_lists_un_placed() {
        let mut s = Project::new();
        s.set_active_printer(Some(&a1_mini()));
        add_n_cubes(&mut s, 100, 30.0);
        let bed = s.active_plate().scene.bed.clone().unwrap();
        let plan = plan_arrangement(&s, &bed);
        assert!(!plan.placed.is_empty(), "some should fit");
        assert!(!plan.un_placed.is_empty(), "some should overflow");
        assert_eq!(plan.placed.len() + plan.un_placed.len(), 100);
        assert!(
            !footprints_overlap_after_plan(&plan, &s),
            "no overlap among placed"
        );
    }

    #[test]
    fn placed_objects_stay_inside_build_volume_after_apply() {
        let mut s = Project::new();
        s.set_active_printer(Some(&a1_mini()));
        add_n_cubes(&mut s, 6, 25.0);
        let bed = s.active_plate().scene.bed.clone().unwrap();
        let plan = plan_arrangement(&s, &bed);
        let (events, un_placed) = apply_arrangement(&mut s, plan);
        assert!(un_placed.is_empty());
        // No OOB events should have fired.
        let oob_count = events
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    super::super::events::SceneEvent::ObjectOutOfBounds { .. }
                )
            })
            .count();
        assert_eq!(oob_count, 0, "arrange should keep everything on plate");
    }

    #[test]
    fn arrange_is_idempotent_on_no_overflow_scene() {
        let mut s = Project::new();
        s.set_active_printer(Some(&a1_mini()));
        add_n_cubes(&mut s, 4, 30.0);
        let bed = s.active_plate().scene.bed.clone().unwrap();
        let plan1 = plan_arrangement(&s, &bed);
        let _ = apply_arrangement(&mut s, plan1);
        // Snapshot the placed transforms.
        let after_first: Vec<(ObjectId, Transform)> = s
            .active_plate()
            .scene
            .objects
            .iter()
            .map(|(id, o)| (*id, o.transform))
            .collect();

        let plan2 = plan_arrangement(&s, &bed);
        let _ = apply_arrangement(&mut s, plan2);
        let after_second: Vec<(ObjectId, Transform)> = s
            .active_plate()
            .scene
            .objects
            .iter()
            .map(|(id, o)| (*id, o.transform))
            .collect();

        // Sort both by id for stable comparison.
        let mut a = after_first;
        let mut b = after_second;
        a.sort_by_key(|p| p.0 .0);
        b.sort_by_key(|p| p.0 .0);
        for ((_, t1), (_, t2)) in a.iter().zip(b.iter()) {
            for (x, y) in t1.matrix.iter().zip(t2.matrix.iter()) {
                assert!(
                    (x - y).abs() < 1e-4,
                    "transform drift between idempotent arranges: {t1:?} vs {t2:?}"
                );
            }
        }
    }

    #[test]
    fn exclusion_zone_at_back_left_pushes_objects_to_the_right() {
        let mut printer = a1_mini();
        // Push a chunky exclusion zone at the back-left so packing
        // has to maneuver around it.
        printer.exclusion_zones.push(BoundingBox {
            min: [0.0, 0.0, 0.0],
            max: [50.0, 50.0, 1.0],
        });
        let mut s = Project::new();
        s.set_active_printer(Some(&printer));
        add_n_cubes(&mut s, 3, 30.0);
        let bed = s.active_plate().scene.bed.clone().unwrap();
        let plan = plan_arrangement(&s, &bed);
        // Every placed cube's XY bbox should clear the zone.
        for (id, xform) in &plan.placed {
            let obj_clone = SceneObject {
                id: *id,
                mesh: s.active_plate().scene.objects.get(id).unwrap().mesh,
                transform: *xform,
                name: String::new(),
                visible: true,
                extruder_id: None,
                group: None,
            };
            let mesh = s.meshes.get(&obj_clone.mesh).unwrap();
            let fp = xy_footprint(&obj_clone, &mesh.bounding_box);
            let aabb = AAbb {
                min: fp.min,
                max: fp.min + fp.size,
            };
            assert!(
                !aabb_intersects_xy(&aabb, &printer.exclusion_zones[0]),
                "placed object intersects exclusion zone"
            );
        }
    }

    #[test]
    fn a_grouped_unit_moves_rigidly() {
        let cube = PrimitiveParams {
            width: 20.0,
            depth: 20.0,
            height: 20.0,
            radius: 0.0,
            radial_segments: 0,
        };
        let mut s = Project::new();
        s.set_active_printer(Some(&a1_mini()));
        let (_, a, _) = s.add_from_primitive(PrimitiveKind::Cube, cube);
        let (_, b, _) = s.add_from_primitive(PrimitiveKind::Cube, cube);
        // Offset b so the group has internal structure, and add a loose cube so
        // the pack actually has to move the group.
        s.translate_object(b, Vec3::new(40.0, 5.0, 0.0)).unwrap();
        let (_, _c, _) = s.add_from_primitive(PrimitiveKind::Cube, cube);
        s.group_objects(&[a, b], "grp".into()).unwrap();

        let origin = |s: &Project, id| {
            s.active_plate()
                .scene
                .objects
                .get(&id)
                .unwrap()
                .transform
                .apply_point(Vec3::ZERO)
        };
        let rel_before = origin(&s, b) - origin(&s, a);

        let bed = s.active_plate().scene.bed.clone().unwrap();
        let plan = plan_arrangement(&s, &bed);
        let _ = apply_arrangement(&mut s, plan);

        let rel_after = origin(&s, b) - origin(&s, a);
        assert!(
            (rel_after - rel_before).length() < 1e-3,
            "group should move as one rigid unit: {rel_before:?} -> {rel_after:?}"
        );
    }
}
