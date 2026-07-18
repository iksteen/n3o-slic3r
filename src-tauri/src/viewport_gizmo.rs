//! Pure gizmo solver: hit-testing the transform handles and turning a grab +
//! cursor ray into a world transform. No GPU — this is the renderer-agnostic 3D
//! math behind the move/rotate/scale gizmos.
//!
//! The frontend captures pointer input and blits frames; all the constraint math
//! lives here. `pick_gizmo` hit-tests the cursor against the active gizmo's
//! handles and returns a `GizmoGrab` capturing the constraint. Each frame of a
//! drag passes that grab back; `compute_pre` turns grab + cursor into a world
//! pre-multiply applied to the selection. On release the same matrix is recomputed
//! and returned as `pre · start` per selected object.

use glam::{Mat4, Quat, Vec3};

use crate::core::project::Session;
use crate::core::scene::state::mesh_bb_corners;
use crate::viewport_gpu::ray_seg_dist;

/// Move/Scale gizmo handle length as a fraction of the eye→gizmo distance, so it
/// holds a constant on-screen size (a TransformControls port). Match the frontend.
pub(crate) const GIZMO_SCREEN_K: f32 = 0.13;

/// Which gizmo to draw at the selection center.
#[derive(serde::Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum GizmoMode {
    #[default]
    None,
    Move,
    Rotate,
    Scale,
}

/// Gizmo center + handle length for the active plate's selection: the world AABB
/// center, and the bounding-*sphere* radius (max distance from that center to any
/// world corner). A handle at that radius encloses the part for any shape — every
/// point is within it by definition — and it's invariant to orientation (rotating
/// a part keeps its corners equidistant from the center), so the gizmo holds its
/// size through a turn where a bounding-box extent would grow/shrink. Min 3mm;
/// shared by the renderer and the hit-test command. `None` if nothing's selected.
pub(crate) fn selection_gizmo(s: &Session) -> Option<(Vec3, f32)> {
    let plate = s.project.active_plate();
    let selection = &s.active_plate_runtime().selection;
    let mut corners: Vec<Vec3> = Vec::new();
    for (id, obj) in plate.scene.objects.iter() {
        if !obj.visible || !selection.contains(id) {
            continue;
        }
        let Some(m) = s.project.meshes.get(&obj.mesh) else {
            continue;
        };
        let model = obj.transform.to_mat4();
        corners.extend(mesh_bb_corners(&m.bounding_box).map(|c| model.transform_point3(c)));
    }
    if corners.is_empty() {
        return None;
    }
    let mut mn = Vec3::splat(f32::MAX);
    let mut mx = Vec3::splat(f32::MIN);
    for c in &corners {
        mn = mn.min(*c);
        mx = mx.max(*c);
    }
    let center = (mn + mx) * 0.5;
    let radius = corners.iter().map(|c| (*c - center).length()).fold(0.0, f32::max);
    Some((center, radius.max(3.0)))
}

/// World-space AABB enclosing the active plate's current selection, with `pre`
/// applied to `drag_ids` (so brackets/gizmo follow a preview).
pub(crate) fn selection_world_aabb(s: &Session, drag_ids: &[u64], pre: Mat4) -> Option<(Vec3, Vec3)> {
    let plate = s.project.active_plate();
    let selection = &s.active_plate_runtime().selection;
    let mut mn = Vec3::splat(f32::MAX);
    let mut mx = Vec3::splat(f32::MIN);
    let mut any = false;
    for (id, obj) in plate.scene.objects.iter() {
        if !obj.visible || !selection.contains(id) {
            continue;
        }
        let Some(m) = s.project.meshes.get(&obj.mesh) else {
            continue;
        };
        let mut model = obj.transform.to_mat4();
        if drag_ids.contains(&id.0) {
            model = pre * model;
        }
        for c in mesh_bb_corners(&m.bounding_box) {
            let w = model.transform_point3(c);
            mn = mn.min(w);
            mx = mx.max(w);
            any = true;
        }
    }
    any.then_some((mn, mx))
}

// ─────────────────── gizmo interaction: Rust-owned hit-test + drag math ───────────
//
// The frontend captures pointer input and blits frames; all 3D math lives here.
// `viewport_grab` hit-tests the cursor against the active gizmo's handles (or the
// selected body / empty space) and returns a `GizmoGrab` capturing the constraint.
// Each frame of a drag passes that grab back via `FrameRequest::gizmo_drag`;
// `frame` calls `compute_pre` to turn grab + cursor into a world pre-multiply
// applied to the selection (preview only). On release `viewport_gizmo_commit`
// recomputes the same matrix and returns `pre · start` per selected object.

/// 1 mm translation snap and 15° rotation snap — mirror the frontend defaults.
const TRANSLATE_SNAP_MM: f32 = 1.0;
const ROTATE_SNAP_RAD: f32 = std::f32::consts::PI * 15.0 / 180.0;

fn snap_to(v: f32, step: f32) -> f32 {
    (v / step).round() * step
}

/// Which transform a grabbed handle drives.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum GrabKind {
    Move,
    Rotate,
    Scale,
}

/// A grabbed gizmo handle (or free-move body): the constraint captured at grab
/// time, stateless so drag/commit can recompute the transform from the cursor
/// alone. Round-trips to the frontend as an opaque blob.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy)]
pub struct GizmoGrab {
    /// Handle index (Move 0/1/2 axis, 3/4/5 plane; Rotate 0/1/2 ring; Scale
    /// 0/1/2 axis, 3/4/5 plane, 6 center). -1 for the free-move body.
    pub idx: i32,
    pub kind: GrabKind,
    /// Constraint plane (move/scale) the cursor ray intersects to track motion.
    pub plane_n: [f32; 3],
    pub plane_p: [f32; 3],
    /// Single-axis move direction; `None` → full in-plane delta (planar / free).
    pub axis_dir: Option<[f32; 3]>,
    /// Rotate ring axis.
    pub rot_axis: Option<[f32; 3]>,
    /// Scale: which local axes the factor applies to (with `uniform` = all).
    pub scale_mask: Option<[bool; 3]>,
    pub uniform: bool,
    pub pivot: [f32; 3],
    /// Cursor's world hit on the constraint plane at grab time (delta origin).
    pub start_hit: [f32; 3],
    /// Scale basis (object axes for a single selection, world otherwise), xyzw.
    pub basis: [f32; 4],
    /// Selection size along the basis axes, for the 1 mm scale-dimension snap.
    pub scale_extent: [f32; 3],
}

/// Ray↔plane intersection. `None` when parallel or the hit is behind the camera
/// (matches the `Ray.intersectPlane` the frontend previously used).
pub(crate) fn ray_plane(ro: Vec3, rd: Vec3, n: Vec3, p: Vec3) -> Option<Vec3> {
    let denom = n.dot(rd);
    if denom.abs() < 1e-9 {
        return None;
    }
    let t = n.dot(p - ro) / denom;
    (t >= 0.0).then_some(ro + rd * t)
}

/// Distance from the ray to point `p`, with the ray parameter at the closest
/// point (picks the center uniform-scale handle).
fn ray_point_dist(ro: Vec3, rd: Vec3, p: Vec3) -> (f32, f32) {
    let t = (p - ro).dot(rd).max(0.0);
    ((p - (ro + rd * t)).length(), t)
}

/// Signed angle from `v0` to `v1` measured about `axis` (right-hand rule).
fn signed_angle(v0: Vec3, v1: Vec3, axis: Vec3) -> f32 {
    axis.dot(v0.cross(v1)).atan2(v0.dot(v1))
}

/// Scale-gizmo basis: a single selected object scales along its own (rotated)
/// axes; multi/none is world-aligned. Mirrors the frontend `computeBasis`.
pub(crate) fn selection_basis(s: &Session) -> Quat {
    let plate = s.project.active_plate();
    let selection = &s.active_plate_runtime().selection;
    if selection.len() == 1 {
        if let Some((_, o)) = plate
            .scene
            .objects
            .iter()
            .find(|(id, _)| selection.contains(id))
        {
            let (_, q, _) = o.transform.to_mat4().to_scale_rotation_translation();
            if q.is_finite() {
                return q.normalize();
            }
        }
    }
    Quat::IDENTITY
}

/// Selection bounding-box size along the axes of `basis` (the scale-snap reference).
fn selection_extent(s: &Session, basis: Quat) -> Vec3 {
    let plate = s.project.active_plate();
    let selection = &s.active_plate_runtime().selection;
    let inv = basis.inverse();
    let mut mn = Vec3::splat(f32::MAX);
    let mut mx = Vec3::splat(f32::MIN);
    let mut any = false;
    for (id, obj) in plate.scene.objects.iter() {
        if !obj.visible || !selection.contains(id) {
            continue;
        }
        let Some(m) = s.project.meshes.get(&obj.mesh) else { continue };
        let model = obj.transform.to_mat4();
        for c in mesh_bb_corners(&m.bounding_box) {
            let w = inv * model.transform_point3(c);
            mn = mn.min(w);
            mx = mx.max(w);
            any = true;
        }
    }
    if any { mx - mn } else { Vec3::ZERO }
}

/// Hit-test the Move gizmo's handles placed at an arbitrary `center` with arm
/// length `arm` — independent of the selection. The split tool drags its
/// cutting plane with this; `pick_gizmo`'s Move arm also delegates here. The
/// returned grab carries an identity basis / zero extent (Move never reads
/// them — only Scale does), so `compute_pre` yields a pure translation.
pub(crate) fn pick_move_at(center: Vec3, arm: f32, ro: Vec3, rd: Vec3, eye: Vec3) -> Option<GizmoGrab> {
    let thick = GIZMO_SCREEN_K * (eye - center).length();
    let mk = |idx: i32, plane_n: Vec3, axis_dir: Option<[f32; 3]>| -> GizmoGrab {
        GizmoGrab {
            idx,
            kind: GrabKind::Move,
            plane_n: plane_n.to_array(),
            plane_p: center.to_array(),
            axis_dir,
            rot_axis: None,
            scale_mask: None,
            uniform: false,
            pivot: center.to_array(),
            start_hit: ray_plane(ro, rd, plane_n, center).unwrap_or(center).to_array(),
            basis: Quat::IDENTITY.to_array(),
            scale_extent: Vec3::ZERO.to_array(),
        }
    };
    let mut best: Option<(f32, GizmoGrab)> = None;
    let pick_r = thick * 0.14;
    let axes = [Vec3::X, Vec3::Y, Vec3::Z];
    for (i, dir) in axes.iter().enumerate() {
        let (dist, t) = ray_seg_dist(ro, rd, center, center + *dir * arm);
        if dist < pick_r && best.as_ref().map_or(true, |(bt, _)| t < *bt) {
            let mut n = rd - *dir * rd.dot(*dir);
            if n.length() < 1e-4 {
                n = axes[(i + 1) % 3];
            }
            best = Some((t, mk(i as i32, n.normalize(), Some(dir.to_array()))));
        }
    }
    let (o, s) = (thick * 0.28, thick * 0.24);
    let planes = [(Vec3::Z, Vec3::X, Vec3::Y), (Vec3::X, Vec3::Y, Vec3::Z), (Vec3::Y, Vec3::X, Vec3::Z)];
    for (i, (n, a, b)) in planes.iter().enumerate() {
        if let Some(hit) = ray_plane(ro, rd, *n, center) {
            let (da, db) = ((hit - center).dot(*a), (hit - center).dot(*b));
            if da >= o && da <= o + s && db >= o && db <= o + s {
                let t = (hit - ro).dot(rd);
                if best.as_ref().map_or(true, |(bt, _)| t < *bt) {
                    best = Some((t, mk(3 + i as i32, *n, None)));
                }
            }
        }
    }
    best.map(|(_, g)| g)
}

/// Hit-test the active gizmo's handles for the cursor ray; nearest handle → grab.
pub(crate) fn pick_gizmo(s: &Session, ro: Vec3, rd: Vec3, eye: Vec3, mode: GizmoMode) -> Option<GizmoGrab> {
    let (center, arm) = selection_gizmo(s)?;
    let basis_q = selection_basis(s);
    let extent = selection_extent(s, basis_q);
    let thick = GIZMO_SCREEN_K * (eye - center).length();
    // Common grab builder: `start_hit` is the cursor's hit on the constraint plane.
    let mk = |idx: i32,
              kind: GrabKind,
              plane_n: Vec3,
              plane_p: Vec3,
              axis_dir: Option<[f32; 3]>,
              rot_axis: Option<[f32; 3]>,
              scale_mask: Option<[bool; 3]>,
              uniform: bool|
     -> GizmoGrab {
        GizmoGrab {
            idx,
            kind,
            plane_n: plane_n.to_array(),
            plane_p: plane_p.to_array(),
            axis_dir,
            rot_axis,
            scale_mask,
            uniform,
            pivot: center.to_array(),
            start_hit: ray_plane(ro, rd, plane_n, plane_p).unwrap_or(plane_p).to_array(),
            basis: basis_q.to_array(),
            scale_extent: extent.to_array(),
        }
    };
    match mode {
        // Move handles don't depend on the selection basis/extent, so the
        // standalone hit-test (shared with the split tool) covers it.
        GizmoMode::Move => pick_move_at(center, arm, ro, rd, eye),
        GizmoMode::Rotate => {
            let mut best: Option<(f32, GizmoGrab)> = None;
            let tol = arm * 0.12;
            for (i, axis) in [Vec3::X, Vec3::Y, Vec3::Z].iter().enumerate() {
                if let Some(hit) = ray_plane(ro, rd, *axis, center) {
                    if ((hit - center).length() - arm).abs() > tol {
                        continue;
                    }
                    let t = (hit - ro).dot(rd);
                    if best.as_ref().map_or(true, |(bt, _)| t < *bt) {
                        best = Some((t, mk(i as i32, GrabKind::Rotate, *axis, center, None, Some(axis.to_array()), None, false)));
                    }
                }
            }
            best.map(|(_, g)| g)
        }
        GizmoMode::Scale => {
            let axes = [basis_q * Vec3::X, basis_q * Vec3::Y, basis_q * Vec3::Z];
            // Center uniform handle first: the axis rods pass through the center,
            // so without priority one of them always wins the tie.
            let (cd, _) = ray_point_dist(ro, rd, center);
            if cd < thick * 0.16 {
                return Some(mk(6, GrabKind::Scale, rd, center, None, None, None, true));
            }
            let mut best: Option<(f32, GizmoGrab)> = None;
            let pick_r = thick * 0.14;
            for i in 0..3 {
                let dir = axes[i];
                let (dist, t) = ray_seg_dist(ro, rd, center, center + dir * arm);
                if dist < pick_r && best.as_ref().map_or(true, |(bt, _)| t < *bt) {
                    let mut n = rd - dir * rd.dot(dir);
                    if n.length() < 1e-4 {
                        n = axes[(i + 1) % 3];
                    }
                    let mut mask = [false; 3];
                    mask[i] = true;
                    best = Some((t, mk(i as i32, GrabKind::Scale, n.normalize(), center, None, None, Some(mask), false)));
                }
            }
            let plane_defs = [(0usize, 1usize, 2usize), (1, 2, 0), (0, 2, 1)];
            let (o, s) = (thick * 0.28, thick * 0.24);
            for (i, (ai, bi, ni)) in plane_defs.iter().enumerate() {
                if let Some(hit) = ray_plane(ro, rd, axes[*ni], center) {
                    let (da, db) = ((hit - center).dot(axes[*ai]), (hit - center).dot(axes[*bi]));
                    if da >= o && da <= o + s && db >= o && db <= o + s {
                        let t = (hit - ro).dot(rd);
                        let mut mask = [false; 3];
                        mask[*ai] = true;
                        mask[*bi] = true;
                        if best.as_ref().map_or(true, |(bt, _)| t < *bt) {
                            best = Some((t, mk(3 + i as i32, GrabKind::Scale, axes[*ni], center, None, None, Some(mask), false)));
                        }
                    }
                }
            }
            best.map(|(_, g)| g)
        }
        GizmoMode::None => None,
    }
}

/// Resolve a grab + the current cursor ray into the world pre-multiply matrix
/// (preview / commit). Pure: depends only on the grab's captured constraint, the
/// cursor ray (origin `ro`, unit dir `rd`), and the camera `eye`/`cam_center`
/// (the latter only for the dead-center uniform-scale fallback direction).
/// Identity when the ray misses the constraint plane.
pub(crate) fn compute_pre(grab: &GizmoGrab, ro: Vec3, rd: Vec3, eye: Vec3, cam_center: Vec3, shift: bool) -> Mat4 {
    let pivot = Vec3::from(grab.pivot);
    let start_hit = Vec3::from(grab.start_hit);
    let plane_n = Vec3::from(grab.plane_n);
    let plane_p = Vec3::from(grab.plane_p);
    match grab.kind {
        GrabKind::Move => {
            let Some(hit) = ray_plane(ro, rd, plane_n, plane_p) else { return Mat4::IDENTITY };
            let mut t = hit - start_hit;
            if let Some(ax) = grab.axis_dir {
                let ax = Vec3::from(ax);
                t = ax * t.dot(ax); // single-axis only
            }
            if !shift {
                t = Vec3::new(
                    snap_to(t.x, TRANSLATE_SNAP_MM),
                    snap_to(t.y, TRANSLATE_SNAP_MM),
                    snap_to(t.z, TRANSLATE_SNAP_MM),
                );
            }
            Mat4::from_translation(t)
        }
        GrabKind::Rotate => {
            let axis = Vec3::from(grab.rot_axis.unwrap_or([0.0, 0.0, 1.0]));
            let Some(hit) = ray_plane(ro, rd, axis, pivot) else { return Mat4::IDENTITY };
            let mut angle = signed_angle(start_hit - pivot, hit - pivot, axis);
            if !shift {
                angle = snap_to(angle, ROTATE_SNAP_RAD);
            }
            Mat4::from_translation(pivot)
                * Mat4::from_axis_angle(axis.normalize(), angle)
                * Mat4::from_translation(-pivot)
        }
        GrabKind::Scale => {
            let Some(hit) = ray_plane(ro, rd, plane_n, plane_p) else { return Mat4::IDENTITY };
            let basis_q = Quat::from_array(grab.basis).normalize();
            let axes = [basis_q * Vec3::X, basis_q * Vec3::Y, basis_q * Vec3::Z];
            let mask = if grab.uniform { [true; 3] } else { grab.scale_mask.unwrap_or([false; 3]) };
            let mut f = if grab.uniform {
                // Center handle: no directional anchor → a zoom, doubling per
                // handle-length of drag along the radial (or camera-right) direction.
                let radial = start_hit - pivot;
                let g = if radial.length() > 1e-3 {
                    radial.normalize()
                } else {
                    (cam_center - eye).normalize().cross(Vec3::Z).normalize_or_zero()
                };
                let l = GIZMO_SCREEN_K * (eye - pivot).length();
                2f32.powf((hit - start_hit).dot(g) / l)
            } else {
                // 1:1 — the grabbed point tracks the cursor along the gesture dir.
                let g = ((if mask[0] { axes[0] } else { Vec3::ZERO })
                    + (if mask[1] { axes[1] } else { Vec3::ZERO })
                    + (if mask[2] { axes[2] } else { Vec3::ZERO }))
                .normalize_or_zero();
                let start_proj = (start_hit - pivot).dot(g);
                let cur_proj = (hit - pivot).dot(g);
                let r = if start_proj.abs() > 1e-3 { start_proj } else { GIZMO_SCREEN_K * (eye - pivot).length() };
                cur_proj / r
            };
            // Snap the largest masked dimension to whole mm (the stable reference).
            if !shift {
                let mut ref_ext = 0.0f32;
                for k in 0..3 {
                    if mask[k] {
                        ref_ext = ref_ext.max(grab.scale_extent[k]);
                    }
                }
                if ref_ext > 1e-3 {
                    let snapped = TRANSLATE_SNAP_MM.max((ref_ext * f).round());
                    f = snapped / ref_ext;
                }
            }
            f = f.max(0.01); // never collapse to zero / mirror
            let ratio = Vec3::new(
                if mask[0] { f } else { 1.0 },
                if mask[1] { f } else { 1.0 },
                if mask[2] { f } else { 1.0 },
            );
            Mat4::from_translation(pivot)
                * Mat4::from_quat(basis_q)
                * Mat4::from_scale(ratio)
                * Mat4::from_quat(basis_q.inverse())
                * Mat4::from_translation(-pivot)
        }
    }
}

#[cfg(test)]
mod gizmo_tests {
    //! Constraint-solve parity for the gizmo drag math (ported from the former
    //! frontend `onMove`). Each test feeds a constructed cursor ray straight at a
    //! known plane so the expected transform is hand-computable.
    use super::*;

    const IDENT: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

    fn grab(kind: GrabKind) -> GizmoGrab {
        GizmoGrab {
            idx: 0,
            kind,
            plane_n: [0.0, 0.0, 1.0],
            plane_p: [0.0; 3],
            axis_dir: None,
            rot_axis: None,
            scale_mask: None,
            uniform: false,
            pivot: [0.0; 3],
            start_hit: [0.0; 3],
            basis: IDENT,
            scale_extent: [0.0; 3],
        }
    }

    fn close(a: Vec3, b: Vec3) {
        assert!((a - b).length() < 1e-3, "{a:?} != {b:?}");
    }

    #[test]
    fn snap_rounds_to_step() {
        assert_eq!(snap_to(5.4, 1.0), 5.0);
        assert_eq!(snap_to(5.6, 1.0), 6.0);
    }

    #[test]
    fn signed_angle_is_ccw_about_axis() {
        // X → Y about +Z is +90°.
        let a = signed_angle(Vec3::X, Vec3::Y, Vec3::Z);
        assert!((a - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
    }

    #[test]
    fn ray_plane_misses_behind_camera() {
        // Ray pointing away from the plane (+Z from above) never hits z=0.
        assert!(ray_plane(Vec3::new(0.0, 0.0, 10.0), Vec3::Z, Vec3::Z, Vec3::ZERO).is_none());
    }

    #[test]
    fn move_single_axis_projects_and_snaps() {
        // Cursor ray straight down hits the XY plane at (5.4, 3.0); an X-axis
        // handle keeps only X, snapped to 1 mm → translate (5, 0, 0).
        let mut g = grab(GrabKind::Move);
        g.axis_dir = Some([1.0, 0.0, 0.0]);
        let pre = compute_pre(
            &g,
            Vec3::new(5.4, 3.0, 100.0),
            -Vec3::Z,
            Vec3::new(0.0, 0.0, 100.0),
            Vec3::ZERO,
            false,
        );
        close(pre.w_axis.truncate(), Vec3::new(5.0, 0.0, 0.0));
    }

    #[test]
    fn move_planar_uses_full_delta() {
        // No axis constraint → full in-plane delta, each component snapped.
        let pre = compute_pre(
            &grab(GrabKind::Move),
            Vec3::new(5.4, 3.0, 100.0),
            -Vec3::Z,
            Vec3::new(0.0, 0.0, 100.0),
            Vec3::ZERO,
            false,
        );
        close(pre.w_axis.truncate(), Vec3::new(5.0, 3.0, 0.0));
    }

    #[test]
    fn rotate_about_z_turns_x_to_y() {
        // Grab at (10,0,0); cursor now at (0,10,0) → +90° about Z.
        let mut g = grab(GrabKind::Rotate);
        g.rot_axis = Some([0.0, 0.0, 1.0]);
        g.start_hit = [10.0, 0.0, 0.0];
        let pre = compute_pre(
            &g,
            Vec3::new(0.0, 10.0, 100.0),
            -Vec3::Z,
            Vec3::new(0.0, 0.0, 100.0),
            Vec3::ZERO,
            false,
        );
        close(pre.transform_vector3(Vec3::X), Vec3::Y);
    }

    #[test]
    fn scale_axis_factor_tracks_cursor() {
        // Grabbed at x=10 on the X handle; cursor now projects to x=20 → ×2.
        let mut g = grab(GrabKind::Scale);
        g.plane_n = [0.0, 1.0, 0.0];
        g.scale_mask = Some([true, false, false]);
        g.start_hit = [10.0, 0.0, 0.0];
        let pre = compute_pre(
            &g,
            Vec3::new(20.0, 5.0, 0.0),
            -Vec3::Y,
            Vec3::new(0.0, 0.0, 100.0),
            Vec3::ZERO,
            false,
        );
        close(pre.transform_point3(Vec3::X), Vec3::new(2.0, 0.0, 0.0));
    }
}
