//! Auto-arrange — single-plate, no-rotation greedy bin-packing
//! (PR-2-8).
//!
//! Reads each scene object's XY footprint (axis-aligned bbox of the
//! mesh after its current transform), sorts by descending footprint
//! area, and places objects onto the bed using a shelf-style
//! packer: walk left-to-right filling rows, advance to the next row
//! when the current one's right edge is reached. Spacing between
//! adjacent objects is fixed at 5 mm.
//!
//! The packer preserves each object's authored rotation/scale —
//! only XY translation changes. Anything that doesn't fit lands in
//! the returned `un_placed` list so the UI can flag those objects;
//! the placed ones still move so the user sees a partial result.
//!
//! "Cut candidate" per the Execution Plan §4 — implementer can drop
//! this entirely if Phase 2 runs long. The user fallback is to
//! place objects manually via the PR-2-5 transform ops.

use super::bed::BedMesh;
use super::state::{ObjectId, SceneObject};
use super::transform::Transform;
use crate::core::project::Project;
use crate::core::printer::profile::BoundingBox;
use glam::Vec3;

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
    let mut entries: Vec<Entry> = state
        .active_plate()
        .scene
        .objects
        .values()
        .filter(|o| o.visible)
        .filter_map(|o| {
            let mesh = state.meshes.get(&o.mesh)?;
            let footprint = xy_footprint(o, &mesh.bounding_box);
            Some(Entry {
                id: o.id,
                obj: o,
                footprint,
            })
        })
        .collect();

    // Largest footprint first. Sorting by area (not perimeter) gives
    // tighter packs for the typical mix of one-big-print-plus-helpers.
    entries.sort_by(|a, b| {
        let area_a = a.footprint.size.x * a.footprint.size.y;
        let area_b = b.footprint.size.x * b.footprint.size.y;
        area_b
            .partial_cmp(&area_a)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.id.0.cmp(&b.id.0))
    });

    let bed_min = Vec3::new(bed.extents.min[0] as f32, bed.extents.min[1] as f32, 0.0);
    let bed_max = Vec3::new(bed.extents.max[0] as f32, bed.extents.max[1] as f32, 0.0);
    let mut cursor_x = bed_min.x;
    let mut cursor_y = bed_min.y;
    let mut row_top = bed_min.y;

    let mut placed = Vec::new();
    let mut un_placed = Vec::new();

    for entry in &entries {
        let w = entry.footprint.size.x;
        let h = entry.footprint.size.y;

        let mut placed_this = false;
        // Up to 2 attempts: current row, then a fresh row. Repeats
        // until both attempts fail (which means the object is taller
        // than the remaining bed height regardless of row).
        for _ in 0..2 {
            // Advance to a new row if this object overflows the current row.
            if cursor_x + w > bed_max.x {
                cursor_x = bed_min.x;
                cursor_y = row_top + PLACEMENT_SPACING;
            }
            // Check vertical fit.
            if cursor_y + h > bed_max.y {
                break;
            }
            // Check exclusion-zone clash; if any, advance past the
            // conflicting zone's far X edge and retry.
            let candidate_aabb = AAbb {
                min: Vec3::new(cursor_x, cursor_y, 0.0),
                max: Vec3::new(cursor_x + w, cursor_y + h, 0.0),
            };
            if let Some(blocking) = bed
                .exclusion_zones
                .iter()
                .find(|z| aabb_intersects_xy(&candidate_aabb, &z.bounds))
            {
                cursor_x = blocking.bounds.max[0] as f32 + PLACEMENT_SPACING;
                continue;
            }
            // Compute the translation that takes the object's current
            // world-space footprint origin to `(cursor_x, cursor_y)`.
            let dx = cursor_x - entry.footprint.min.x;
            let dy = cursor_y - entry.footprint.min.y;
            // Preserve Z: this packer only moves in XY. The object's
            // existing transform stays intact otherwise.
            let new_xform = Transform::from_mat4(
                glam::Mat4::from_translation(Vec3::new(dx, dy, 0.0)) * entry.obj.transform.to_mat4(),
            );
            placed.push((entry.id, new_xform));
            // Advance the row cursor + bump row_top if this object
            // is taller than anything else in the row.
            cursor_x += w + PLACEMENT_SPACING;
            row_top = row_top.max(cursor_y + h);
            placed_this = true;
            break;
        }
        if !placed_this {
            un_placed.push(entry.id);
        }
    }

    ArrangeResult { placed, un_placed }
}

/// Apply a [`plan_arrangement`] result to the scene state. Calls
/// `set_object_transform` per placed object so PR-2-5's OOB check
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

struct Entry<'a> {
    id: ObjectId,
    obj: &'a SceneObject,
    footprint: XyFootprint,
}

struct XyFootprint {
    min: Vec3,
    size: Vec3,
}

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

struct AAbb {
    min: Vec3,
    max: Vec3,
}

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
            model: "Bambu A1 mini".into(),
            supported_build_plates: vec!["Textured PEI Plate".into()],
            toolheads: vec![Toolhead {
                default_nozzle_diameter: 0.4,
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
                    parent: None,
                    group_id: None,
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
            .filter(|e| matches!(e, super::super::events::SceneEvent::ObjectOutOfBounds { .. }))
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
                parent: None,
                group_id: None,
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
}
