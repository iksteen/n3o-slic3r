//! Auto-arrange — drives libslic3r's nester (the engine behind
//! OrcaSlicer's "Arrange", on libnest2d) through the FFI.
//!
//! Each *unit* (a lone object, or a whole group kept rigid) contributes
//! one convex footprint = the convex hull of its mesh vertices projected
//! to the bed plane. Those hulls, the bed rectangle, and the printer's
//! exclusion zones go to `slic3r_ffi::arrange`, which packs them with
//! the caller's [`ArrangeOptions`] (spacing + allow-rotations — UI
//! state from the arrange tool panel, passed per call) and returns a
//! translation + rotation + logical bed index per unit, applied to
//! every member of the unit. On i3-structure printers the pack also
//! aligns long sides to Y (derived, like Orca).
//!
//! Phase 1 is single-plate: a unit the nester spills onto an extra bed
//! (`bed_idx > 0`) is reported as `un_placed` so the UI flags it, exactly
//! like the old packer's overflow. Spilling onto *real* extra plates is a
//! follow-up (needs the move-to-plate machinery).

use super::bed::BedMesh;
use super::state::{GroupId, ObjectId};
use super::transform::Transform;
use crate::core::project::{PlateId, Project};
use glam::Vec3;
use std::collections::HashMap;

// Used only by the test-only bbox helpers below.
#[cfg(test)]
use super::state::SceneObject;
#[cfg(test)]
use crate::core::printer::profile::BoundingBox;

/// Nester knobs the arrange tool panel exposes. Passed per call —
/// the UI owns the values; nothing is persisted backend-side.
/// `align_to_y_axis` is deliberately NOT here — it's derived from the
/// bound printer's `printer_structure` (i3 → on), matching OrcaSlicer.
#[derive(Debug, Clone, Copy)]
pub struct ArrangeOptions {
    pub spacing_mm: f32,
    pub allow_rotations: bool,
}

impl Default for ArrangeOptions {
    fn default() -> Self {
        Self {
            // Matches OrcaSlicer's default skirt clearance so prints
            // don't fuse.
            spacing_mm: 5.0,
            allow_rotations: false,
        }
    }
}

/// Whether the plate's bound printer is an i3/bed-slinger structure —
/// OrcaSlicer enables the nester's `align_to_y_axis` for those so long
/// items line up with the moving bed's travel axis. Derived, not a
/// user option (matching Orca, which sets it from `printer_structure`).
fn plate_printer_is_i3(state: &Project) -> bool {
    let Some(instance_id) = state.active_plate().printer_instance_id() else {
        return false;
    };
    let Some(instance) = crate::core::printer::lookup_instance(instance_id) else {
        return false;
    };
    crate::core::profile_library::load_printer_fragment(&instance.printer_fragment_slug)
        .and_then(|cascade| {
            cascade
                .rules
                .iter()
                .find(|r| r.is_default())
                .and_then(|r| r.set.get("printer_structure").cloned())
        })
        .is_some_and(|s| s == "i3")
}

/// Outcome of one auto-arrange pass.
pub struct ArrangeResult {
    /// Objects that fit on the current plate, with their new transforms.
    pub placed: Vec<(ObjectId, Transform)>,
    /// Objects the nester spilled onto extra beds — with the transform that
    /// packs them on that bed and which extra bed (`bed_idx >= 1`) they go to.
    /// `apply_arrangement` turns each extra bed into a new plate.
    pub spilled: Vec<SpilledObject>,
    /// Objects with no usable footprint or that fit no bed at all (degenerate
    /// or larger than the bed). The UI flags these.
    pub un_placed: Vec<ObjectId>,
}

/// One object the nester pushed onto an extra bed (see [`ArrangeResult`]).
pub struct SpilledObject {
    pub id: ObjectId,
    /// Transform that packs the object on its extra bed (bed-local position,
    /// valid on the new plate since it's bound to the same printer).
    pub transform: Transform,
    /// Which extra bed (1-based) the nester assigned it.
    pub bed_idx: i32,
}

/// Compute a packing without mutating the scene. Pure function so the
/// caller can decide whether to apply the result (e.g., test code
/// vs. a user-facing "preview" flow).
pub fn plan_arrangement(
    state: &Project,
    bed: &BedMesh,
    opts: ArrangeOptions,
) -> ArrangeResult {
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
    let mut spilled = Vec::new();
    let mut un_placed = Vec::new();
    if contours.is_empty() {
        return ArrangeResult {
            placed,
            spilled,
            un_placed,
        };
    }

    let mut excludes: Vec<[f64; 4]> = bed
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

    // Reserve the wipe/prime tower's footprint so the pack doesn't sit objects
    // on it. It exists only for a multi-material plate (the helper returns None
    // otherwise) and sits at a fixed config position. Every spill plate is the
    // same printer, so the tower occupies the same spot on each — the FFI
    // replicates every exclude across all beds the pack opens (see its
    // `bed_count` arg), so this one rect protects every bed. `(x, y)` is the
    // lower-left corner; pad by the brim.
    let plate_id = state.active_plate().id;
    if let Ok(Some(t)) =
        crate::core::project::resolve::tower_geometry_for_plate(state, plate_id)
    {
        excludes.push([
            t.x - t.brim - bed_min.0,
            t.y - t.brim - bed_min.1,
            t.x + t.width + t.brim - bed_min.0,
            t.y + t.width + t.brim - bed_min.1,
        ]);
    }

    let placements = match slic3r_ffi::arrange(
        &contours,
        &excludes,
        // Worst case the pack spills one unit per bed, so reserve the per-plate
        // obstacles (exclusion zones + tower) on up to that many beds.
        arranged_units.len(),
        bed_size,
        opts.spacing_mm as f64,
        opts.allow_rotations,
        plate_printer_is_i3(state),
    ) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "libslic3r arrange failed; leaving objects in place");
            for unit in &arranged_units {
                un_placed.extend(unit.iter().copied());
            }
            return ArrangeResult {
                placed,
                spilled,
                un_placed,
            };
        }
    };

    for (unit, placement) in arranged_units.iter().zip(&placements) {
        // The packed pose, applied to every member so a group stays rigid.
        // The nester's convention (ArrangePolygon::transformed_poly) is
        // rotate-about-the-contour-origin THEN translate; our contours were
        // fed in bed-local coords, so the rotation pivot is the bed-local
        // origin = `bed_min` in world space:
        //   world' = T(bed_min) ∘ T(t) ∘ Rz(rot) ∘ T(-bed_min) ∘ world
        // With rotations disabled (rot = 0) this reduces to the plain
        // translation delta.
        let t = Vec3::new(
            placement.translation[0] as f32,
            placement.translation[1] as f32,
            0.0,
        );
        let delta = if placement.rotation != 0.0 {
            let pivot = Vec3::new(bed_min.0 as f32, bed_min.1 as f32, 0.0);
            Transform::translation(pivot + t)
                .compose(Transform::rotation_around(Vec3::Z, placement.rotation as f32))
                .compose(Transform::translation(-pivot))
        } else {
            Transform::translation(t)
        };
        match placement.bed_idx {
            0 => {
                for &id in unit.iter() {
                    if let Some(obj) = plate.objects.get(&id) {
                        placed.push((id, delta.compose(obj.transform)));
                    }
                }
            }
            n if n >= 1 => {
                for &id in unit.iter() {
                    if let Some(obj) = plate.objects.get(&id) {
                        spilled.push(SpilledObject {
                            id,
                            transform: delta.compose(obj.transform),
                            bed_idx: n,
                        });
                    }
                }
            }
            // bed_idx < 0: the nester couldn't place it at all.
            _ => un_placed.extend(unit.iter().copied()),
        }
    }

    ArrangeResult {
        placed,
        spilled,
        un_placed,
    }
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

/// Apply a [`plan_arrangement`] result to the scene state. Objects that fit the
/// current plate get their packed transform (via `set_object_transform`, so the
/// OOB check still fires for any exclusion-zone clash). Spilled objects are
/// positioned the same way, then moved onto fresh plates — one per extra bed,
/// each bound to the same printer — via `move_objects_to_plate` (phase 2). The
/// returned `un_placed` is only the genuinely-unplaceable objects.
pub fn apply_arrangement(
    state: &mut Project,
    plan: ArrangeResult,
) -> (Vec<super::events::SceneEvent>, Vec<ObjectId>) {
    let mut events = Vec::new();
    let source = state.active_plate().id;

    // Units that stay on this plate.
    for (id, xform) in plan.placed {
        apply_transform(state, id, xform, &mut events);
    }

    // Spill: one new plate per extra bed (same printer), then position + move.
    if !plan.spilled.is_empty() {
        let max_bed = plan.spilled.iter().map(|s| s.bed_idx).max().unwrap_or(0);
        let mut bed_plate: HashMap<i32, PlateId> = HashMap::new();
        for k in 1..=max_bed {
            // `None` inherits the active plate's printer binding (+ bed), so the
            // packed bed-local positions are valid on the new plate.
            let (pid, evs) = state.add_plate(None);
            events.extend(evs);
            bed_plate.insert(k, pid);
        }
        // Position every spilled object (the move preserves the transform).
        for s in &plan.spilled {
            apply_transform(state, s.id, s.transform, &mut events);
        }
        // Move each extra bed's objects onto its plate.
        for k in 1..=max_bed {
            let ids: Vec<ObjectId> = plan
                .spilled
                .iter()
                .filter(|s| s.bed_idx == k)
                .map(|s| s.id)
                .collect();
            if let Some(&target) = bed_plate.get(&k) {
                match state.move_objects_to_plate(source, target, &ids) {
                    Ok(evs) => events.extend(evs),
                    Err(e) => tracing::warn!(error = %e, "auto-arrange spill move failed"),
                }
            }
        }
    }

    (events, plan.un_placed)
}

fn apply_transform(
    state: &mut Project,
    id: ObjectId,
    xform: Transform,
    events: &mut Vec<super::events::SceneEvent>,
) {
    match state.set_object_transform(id, xform) {
        Ok(evs) => events.extend(evs),
        Err(e) => tracing::warn!(
            object_id = id.0,
            error = %e,
            "auto-arrange could not apply transform (deleted between plan and apply?)"
        ),
    }
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
        let plan = plan_arrangement(&s, &bed, ArrangeOptions::default());
        assert_eq!(plan.placed.len(), 10);
        assert!(plan.un_placed.is_empty());
        assert!(
            !footprints_overlap_after_plan(&plan, &s),
            "no overlap after arrange"
        );
    }

    #[test]
    fn one_hundred_cubes_overflow_spills_rather_than_un_placing() {
        let mut s = Project::new();
        s.set_active_printer(Some(&a1_mini()));
        add_n_cubes(&mut s, 100, 30.0);
        let bed = s.active_plate().scene.bed.clone().unwrap();
        let plan = plan_arrangement(&s, &bed, ArrangeOptions::default());
        assert!(!plan.placed.is_empty(), "some fit the first plate");
        assert!(!plan.spilled.is_empty(), "the rest spill to extra beds");
        // 30mm cubes all fit *some* bed, so nothing is truly un-placeable.
        assert!(plan.un_placed.is_empty(), "everything fits on some bed");
        assert_eq!(
            plan.placed.len() + plan.spilled.len() + plan.un_placed.len(),
            100
        );
        assert!(
            !footprints_overlap_after_plan(&plan, &s),
            "no overlap among first-plate placements"
        );
    }

    #[test]
    fn arrange_reserves_the_wipe_tower_footprint() {
        use crate::core::project::resolve::tower_geometry_for_plate;
        let _ = slic3r_ffi::init(None, 3);
        // Project::default() binds a real library instance (A1 mini) so the
        // cascade resolves a tower position; two materials make it multi-material
        // so a tower is actually generated.
        let mut s = Project::default();
        // Crowd the bed: with only a handful of cubes the tower footprint is
        // trivially clear and the test can't tell a hard obstacle from a soft
        // scoring penalty. Packing the bed near-full means a soft penalty would
        // be overrun and an object would land on the tower; only hard NFP
        // avoidance keeps bed-0 placements clear (the surplus spills).
        add_n_cubes(&mut s, 16, 42.0);
        let active = s.active_plate;
        let ids: Vec<ObjectId> = s.plates[active].scene.objects.keys().copied().collect();
        for id in ids.iter().take(8) {
            s.plates[active].scene.objects.get_mut(id).unwrap().extruder_id = Some(2);
        }
        let plate_id = s.active_plate().id;
        let Some(tower) = tower_geometry_for_plate(&s, plate_id).ok().flatten() else {
            return; // no tower resolvable in this env — nothing to assert
        };
        let bed = s.active_plate().scene.bed.clone().unwrap();

        let plan = plan_arrangement(&s, &bed, ArrangeOptions::default());
        assert!(!plan.placed.is_empty(), "cubes should pack onto the plate");
        // Crowding this many cubes overflows the bed, so the pack should spill —
        // letting us prove the tower is reserved on the extra beds too, not just
        // bed 0 (the tower sits at the same bed-local spot on every plate).
        assert!(!plan.spilled.is_empty(), "16 cubes + a tower should overflow bed 0");
        let (tx0, ty0) = (tower.x as f32, tower.y as f32);
        let (tx1, ty1) = ((tower.x + tower.width) as f32, (tower.y + tower.width) as f32);
        let assert_clear = |id: &ObjectId, xform: &Transform, bed_label: &str| {
            let mesh_id = s.plates[active].scene.objects.get(id).unwrap().mesh;
            let probe = SceneObject {
                id: *id,
                mesh: mesh_id,
                transform: *xform,
                name: String::new(),
                visible: true,
                extruder_id: None,
                group: None,
            };
            let fp = xy_footprint(&probe, &s.meshes.get(&mesh_id).unwrap().bounding_box);
            let (x0, y0) = (fp.min.x, fp.min.y);
            let (x1, y1) = (fp.min.x + fp.size.x, fp.min.y + fp.size.y);
            let clear = x1 <= tx0 + 1e-3 || x0 >= tx1 - 1e-3 || y1 <= ty0 + 1e-3 || y0 >= ty1 - 1e-3;
            assert!(
                clear,
                "{bed_label} object overlaps the wipe tower: obj x[{x0},{x1}] y[{y0},{y1}] tower x[{tx0},{tx1}] y[{ty0},{ty1}]"
            );
        };
        for (id, xform) in &plan.placed {
            assert_clear(id, xform, "bed-0");
        }
        for sp in &plan.spilled {
            assert_clear(&sp.id, &sp.transform, "spilled");
        }
    }

    #[test]
    fn spill_creates_extra_plates_and_relocates_the_objects() {
        let mut s = Project::new();
        s.set_active_printer(Some(&a1_mini()));
        add_n_cubes(&mut s, 100, 30.0);
        assert_eq!(s.plates.len(), 1);
        let bed = s.active_plate().scene.bed.clone().unwrap();
        let plan = plan_arrangement(&s, &bed, ArrangeOptions::default());
        assert!(!plan.spilled.is_empty());
        let (_events, un_placed) = apply_arrangement(&mut s, plan);
        assert!(un_placed.is_empty());
        // Spill created at least one extra plate, all bound to the same printer.
        assert!(s.plates.len() > 1, "spill should add plates");
        let printer = s.plates[0].printer_instance_id().map(str::to_owned);
        for p in &s.plates[1..] {
            assert_eq!(p.printer_instance_id().map(str::to_owned), printer);
            assert!(!p.scene.objects.is_empty(), "extra plate should hold spill");
        }
        // No object is lost or duplicated.
        let total: usize = s.plates.iter().map(|p| p.scene.objects.len()).sum();
        assert_eq!(total, 100);
    }

    #[test]
    fn placed_objects_stay_inside_build_volume_after_apply() {
        let mut s = Project::new();
        s.set_active_printer(Some(&a1_mini()));
        add_n_cubes(&mut s, 6, 25.0);
        let bed = s.active_plate().scene.bed.clone().unwrap();
        let plan = plan_arrangement(&s, &bed, ArrangeOptions::default());
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
        let plan1 = plan_arrangement(&s, &bed, ArrangeOptions::default());
        let _ = apply_arrangement(&mut s, plan1);
        // Snapshot the placed transforms.
        let after_first: Vec<(ObjectId, Transform)> = s
            .active_plate()
            .scene
            .objects
            .iter()
            .map(|(id, o)| (*id, o.transform))
            .collect();

        let plan2 = plan_arrangement(&s, &bed, ArrangeOptions::default());
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
        let plan = plan_arrangement(&s, &bed, ArrangeOptions::default());
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
        let b_current = s.active_plate().scene.objects.get(&b).unwrap().transform;
        s.set_object_transform(
            b,
            Transform::translation(Vec3::new(40.0, 5.0, 0.0)).compose(b_current),
        )
        .unwrap();
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
        let plan = plan_arrangement(&s, &bed, ArrangeOptions::default());
        let _ = apply_arrangement(&mut s, plan);

        let rel_after = origin(&s, b) - origin(&s, a);
        assert!(
            (rel_after - rel_before).length() < 1e-3,
            "group should move as one rigid unit: {rel_before:?} -> {rel_after:?}"
        );
    }

    #[test]
    fn placements_land_on_a_centered_bed() {
        // Exercise a non-zero bed_min (a bed centered on the origin). The nester
        // works in a bed-local frame; the returned translation is a delta, so
        // objects must still land within the centered extents — not offset by
        // bed_min.
        let mut s = Project::new();
        s.set_active_printer(Some(&a1_mini()));
        add_n_cubes(&mut s, 4, 25.0);
        let mut bed = s.active_plate().scene.bed.clone().unwrap();
        bed.extents.min = [-90.0, -90.0, 0.0];
        bed.extents.max = [90.0, 90.0, 180.0];

        let plan = plan_arrangement(&s, &bed, ArrangeOptions::default());
        let placed_ids: Vec<ObjectId> = plan.placed.iter().map(|(id, _)| *id).collect();
        let _ = apply_arrangement(&mut s, plan);
        assert!(!placed_ids.is_empty(), "cubes should fit a centered 180mm bed");
        for id in placed_ids {
            let o = s.active_plate().scene.objects.get(&id).unwrap();
            let mesh = s.meshes.get(&o.mesh).unwrap();
            let fp = xy_footprint(o, &mesh.bounding_box);
            let (lo_x, lo_y) = (fp.min.x, fp.min.y);
            let (hi_x, hi_y) = (fp.min.x + fp.size.x, fp.min.y + fp.size.y);
            assert!(
                lo_x >= -91.0 && hi_x <= 91.0 && lo_y >= -91.0 && hi_y <= 91.0,
                "object off the centered bed: x[{lo_x},{hi_x}] y[{lo_y},{hi_y}] (bed ±90)"
            );
        }
    }
}
