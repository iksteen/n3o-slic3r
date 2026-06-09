//! Axis-alignment heuristic: find a model's dominant horizontal line
//! direction and the Z rotation that snaps it to the X or Y axis.
//!
//! "Most common line direction" = the length-weighted dominant direction of
//! the model's **feature edges** — edges where the two adjacent triangles
//! actually fold — projected onto the XY plane. Restricting to feature edges
//! is what makes this work on solids: a mesh box isn't just its 12 real edges,
//! every quad face is split into two triangles, so the soup also contains
//! coplanar *diagonals* (e.g. a 100×50 face's diagonal sits at ~27°). Counting
//! those would drag the estimate off the real grid. A crease test (adjacent
//! face normals differ by more than a threshold) drops them and keeps only the
//! true edges; open-mesh boundary edges are kept as real outline.
//!
//! With the feature edges in hand we recover the grid orientation with a
//! mod-90° circular mean (a direction and its perpendicular reinforce, so it's
//! robust even for a square), then split the edge weight between that direction
//! and its perpendicular to pick the *dominant* family — what the X / Y buttons
//! target. The result is a pure yaw; callers re-seat + clamp afterwards.

use std::collections::HashMap;
use std::f32::consts::{FRAC_PI_2, PI};

/// Which axis the dominant line direction should end up parallel to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
pub enum AlignAxis {
    X,
    Y,
}

/// Minimum XY-projected length for an edge to count — drops near-vertical
/// edges (large in Z, tiny in plane) whose planar direction is just noise.
const MIN_EDGE_XY: f32 = 1.0e-4;

/// How prominent the dominant grid direction must be (resultant ÷ total weight
/// of the mod-90° circular sum) for the alignment to mean anything. Below this
/// the footprint is ~isotropic (e.g. a disc) — no sensible line to align, so we
/// report `None` and the caller no-ops.
const MIN_PROMINENCE: f32 = 0.05;

/// Two triangles sharing an edge are a *crease* (real feature edge) when their
/// normals differ by more than this angle. ~25° keeps genuine part edges
/// (box edges are 90°) while dropping both flat-face triangulation diagonals
/// (0°) and the small facet-to-facet steps of a tessellated curved surface.
const FEATURE_ANGLE_COS: f32 = 0.906; // cos(25°)

/// Unit normal of triangle `(i0, i1, i2)`, or `None` for a degenerate (zero
/// area) triangle.
fn face_normal(v: &[f32], i0: usize, i1: usize, i2: usize) -> Option<[f32; 3]> {
    let p = |i: usize| [v[i * 3], v[i * 3 + 1], v[i * 3 + 2]];
    let (a, b, c) = (p(i0), p(i1), p(i2));
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let w = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let n = [
        u[1] * w[2] - u[2] * w[1],
        u[2] * w[0] - u[0] * w[2],
        u[0] * w[1] - u[1] * w[0],
    ];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    (len >= 1.0e-12).then(|| [n[0] / len, n[1] / len, n[2] / len])
}

/// Adjacent-face normals collected per undirected edge — at most two are kept
/// (enough to classify manifold edges); a third sighting just marks the edge
/// non-manifold, which we treat as a feature.
#[derive(Default)]
struct EdgeFaces {
    normals: [[f32; 3]; 2],
    count: u8,
}

impl EdgeFaces {
    fn push(&mut self, n: [f32; 3]) {
        if (self.count as usize) < 2 {
            self.normals[self.count as usize] = n;
        }
        self.count = self.count.saturating_add(1);
    }

    /// A crease (or boundary / non-manifold) edge worth counting as a line.
    fn is_feature(&self) -> bool {
        match self.count {
            0 => false,
            1 => true, // open-mesh boundary edge — a real outline
            2 => {
                let (a, b) = (self.normals[0], self.normals[1]);
                let dot = a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
                // Compare on |dot| so the test is winding-independent: two
                // coplanar triangles read as a crease only if their planes
                // actually diverge, whether their normals come out parallel
                // (consistent winding, dot≈+1) or antiparallel (a flipped
                // triangle, dot≈−1). The cost is that near-flat folds (>155°)
                // also read as coplanar — rare, and marginal for alignment.
                dot.abs() < FEATURE_ANGLE_COS
            }
            _ => true, // non-manifold — keep, conservatively
        }
    }
}

/// Feature edges of the mesh as `(lo, hi)` vertex-index pairs.
fn feature_edges(vertices: &[f32], indices: &[u32]) -> Vec<(usize, usize)> {
    let vc = vertices.len() / 3;
    let mut edges: HashMap<(usize, usize), EdgeFaces> = HashMap::new();
    for tri in indices.chunks_exact(3) {
        let (i0, i1, i2) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
        if i0 >= vc || i1 >= vc || i2 >= vc {
            continue;
        }
        let Some(n) = face_normal(vertices, i0, i1, i2) else {
            continue;
        };
        for (a, b) in [(i0, i1), (i1, i2), (i2, i0)] {
            let key = if a < b { (a, b) } else { (b, a) };
            edges.entry(key).or_default().push(n);
        }
    }
    edges
        .into_iter()
        .filter_map(|(e, faces)| faces.is_feature().then_some(e))
        .collect()
}

/// The Z rotation (radians) that aligns the mesh's dominant line direction to
/// `axis`. `vertices` is flattened world-space xyz; `indices` are triangle
/// corners. Returns `None` when there's no dominant direction worth aligning.
pub fn axis_alignment_rotation(vertices: &[f32], indices: &[u32], axis: AlignAxis) -> Option<f32> {
    let edges = feature_edges(vertices, indices);
    let dir = |&(a, b): &(usize, usize)| -> Option<(f32, f32, f32)> {
        let dx = vertices[b * 3] - vertices[a * 3];
        let dy = vertices[b * 3 + 1] - vertices[a * 3 + 1];
        let w = (dx * dx + dy * dy).sqrt();
        (w >= MIN_EDGE_XY).then_some((dx, dy, w))
    };

    // Pass 1: mod-90° circular mean → the grid orientation `theta`. Doubling to
    // 4φ folds the 90°-periodic edge directions onto the full circle, so a
    // direction and its perpendicular reinforce rather than cancel.
    let (mut c, mut s, mut wsum) = (0.0f32, 0.0f32, 0.0f32);
    for e in &edges {
        if let Some((dx, dy, w)) = dir(e) {
            let phi = dy.atan2(dx);
            c += w * (4.0 * phi).cos();
            s += w * (4.0 * phi).sin();
            wsum += w;
        }
    }
    if wsum < MIN_EDGE_XY {
        return None; // no planar feature edges
    }
    let resultant = (c * c + s * s).sqrt();
    if resultant / wsum < MIN_PROMINENCE {
        return None; // ~isotropic — no dominant direction
    }
    let theta = 0.25 * s.atan2(c); // one representative of the grid angle, mod 90°

    // Pass 2: split edge weight between `theta` and `theta + 90°` to find which
    // family carries more length — that's the "most common line direction".
    let (mut along, mut perp) = (0.0f32, 0.0f32);
    for e in &edges {
        if let Some((dx, dy, w)) = dir(e) {
            let d = dy.atan2(dx) - theta;
            along += w * d.cos() * d.cos();
            perp += w * d.sin() * d.sin();
        }
    }
    let dominant = if along >= perp {
        theta
    } else {
        theta + FRAC_PI_2
    };

    // Rotation that lands the dominant line on the target axis. A line is
    // symmetric mod 180°, so reduce to the smallest-magnitude equivalent spin.
    let target = match axis {
        AlignAxis::X => 0.0,
        AlignAxis::Y => FRAC_PI_2,
    };
    Some(reduce_mod_pi(target - dominant))
}

/// Fold an angle into `(-π/2, π/2]` — equivalent rotations for a head/tail-
/// symmetric line, picking the shortest turn.
fn reduce_mod_pi(a: f32) -> f32 {
    let mut a = a % PI;
    if a > FRAC_PI_2 {
        a -= PI;
    } else if a <= -FRAC_PI_2 {
        a += PI;
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `w × h` rectangular sheet in the XY plane (z = 0), rotated by `alpha`
    /// about the origin, as two triangles.
    fn rect(alpha: f32, w: f32, h: f32) -> (Vec<f32>, Vec<u32>) {
        let (ca, sa) = (alpha.cos(), alpha.sin());
        let mut verts = Vec::new();
        for (x, y) in [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)] {
            verts.extend_from_slice(&[x * ca - y * sa, x * sa + y * ca, 0.0]);
        }
        (verts, vec![0, 1, 2, 0, 2, 3])
    }

    /// A solid `lx × ly × lz` box, rotated by `alpha` about Z — 12 triangles,
    /// consistent outward winding. This is the case that broke "count every
    /// edge": its faces carry triangulation diagonals at off-grid angles.
    fn box_mesh(alpha: f32, lx: f32, ly: f32, lz: f32) -> (Vec<f32>, Vec<u32>) {
        let (ca, sa) = (alpha.cos(), alpha.sin());
        let local = [
            (0.0, 0.0, 0.0),
            (lx, 0.0, 0.0),
            (lx, ly, 0.0),
            (0.0, ly, 0.0),
            (0.0, 0.0, lz),
            (lx, 0.0, lz),
            (lx, ly, lz),
            (0.0, ly, lz),
        ];
        let mut verts = Vec::new();
        for (x, y, z) in local {
            verts.extend_from_slice(&[x * ca - y * sa, x * sa + y * ca, z]);
        }
        #[rustfmt::skip]
        let indices = vec![
            0, 2, 1, 0, 3, 2, // bottom
            4, 5, 6, 4, 6, 7, // top
            0, 1, 5, 0, 5, 4, // front (y=0)
            3, 6, 2, 3, 7, 6, // back  (y=ly)
            0, 4, 7, 0, 7, 3, // left  (x=0)
            1, 2, 6, 1, 6, 5, // right (x=lx)
        ];
        (verts, indices)
    }

    /// The dominant (long-edge) direction, originally at `alpha`, after a Z
    /// rotation of `angle`.
    fn long_dir_after(alpha: f32, angle: f32) -> (f32, f32) {
        let (lx, ly) = (alpha.cos(), alpha.sin());
        let (c, s) = (angle.cos(), angle.sin());
        (lx * c - ly * s, lx * s + ly * c)
    }

    #[test]
    fn align_x_lays_the_dominant_direction_along_x() {
        let alpha = 25_f32.to_radians();
        let (v, i) = rect(alpha, 100.0, 4.0);
        let angle = axis_alignment_rotation(&v, &i, AlignAxis::X).unwrap();
        let (_dx, dy) = long_dir_after(alpha, angle);
        assert!(
            dy.abs() < 0.05,
            "dominant line not on X after align: dy={dy}"
        );
    }

    #[test]
    fn align_y_lays_the_dominant_direction_along_y() {
        let alpha = 25_f32.to_radians();
        let (v, i) = rect(alpha, 100.0, 4.0);
        let angle = axis_alignment_rotation(&v, &i, AlignAxis::Y).unwrap();
        let (dx, _dy) = long_dir_after(alpha, angle);
        assert!(
            dx.abs() < 0.05,
            "dominant line not on Y after align: dx={dx}"
        );
    }

    #[test]
    fn an_axis_aligned_box_is_left_untouched_by_align_x() {
        // The regression: triangulation diagonals used to pull this ~5° off.
        // A box already aligned to X must return a ~0 rotation.
        let (v, i) = box_mesh(0.0, 100.0, 50.0, 50.0);
        let angle = axis_alignment_rotation(&v, &i, AlignAxis::X).unwrap();
        assert!(
            angle.abs() < 0.01,
            "axis-aligned box should not rotate, got {angle} rad"
        );
    }

    #[test]
    fn a_rotated_solid_box_snaps_its_long_axis_back_to_x() {
        let alpha = 30_f32.to_radians();
        let (v, i) = box_mesh(alpha, 100.0, 50.0, 50.0);
        let angle = axis_alignment_rotation(&v, &i, AlignAxis::X).unwrap();
        let (_dx, dy) = long_dir_after(alpha, angle);
        assert!(
            dy.abs() < 0.02,
            "rotated box long axis not snapped to X: dy={dy}"
        );
    }

    #[test]
    fn a_solid_box_align_y_turns_the_long_axis_ninety_degrees() {
        let (v, i) = box_mesh(0.0, 100.0, 50.0, 50.0);
        let angle = axis_alignment_rotation(&v, &i, AlignAxis::Y).unwrap();
        // Long axis was on X; align-Y must put it on Y (±90°).
        assert!(
            (angle.abs() - FRAC_PI_2).abs() < 0.01,
            "align-Y should turn the bar 90°, got {angle} rad"
        );
    }

    #[test]
    fn a_flipped_coplanar_diagonal_is_still_excluded() {
        // The two triangles of this axis-aligned sheet are wound *oppositely*,
        // so their shared diagonal's adjacent normals are antiparallel. A
        // signed crease test would keep that diagonal and pull the estimate
        // off-axis; the |dot| test must still drop it, leaving align-X ≈ 0.
        let (v, _) = rect(0.0, 100.0, 8.0);
        let indices = vec![0, 1, 2, 0, 3, 2]; // second triangle reversed
        let angle = axis_alignment_rotation(&v, &indices, AlignAxis::X).unwrap();
        assert!(
            angle.abs() < 0.01,
            "flipped-winding diagonal polluted the estimate: {angle} rad"
        );
    }

    #[test]
    fn a_skewed_square_snaps_to_the_axes() {
        let alpha = 20_f32.to_radians();
        let (v, i) = rect(alpha, 50.0, 50.0);
        let angle = axis_alignment_rotation(&v, &i, AlignAxis::X).unwrap();
        let (dx, dy) = long_dir_after(alpha, angle);
        assert!(
            dx.abs() < 0.05 || dy.abs() < 0.05,
            "square edge not axis-aligned: ({dx}, {dy})"
        );
    }

    #[test]
    fn the_chosen_spin_is_minimal() {
        for deg in [5, 35, 50, 80, 100, 170] {
            let alpha = (deg as f32).to_radians();
            let (v, i) = rect(alpha, 100.0, 4.0);
            let angle = axis_alignment_rotation(&v, &i, AlignAxis::X).unwrap();
            assert!(
                angle.abs() <= FRAC_PI_2 + 1e-3,
                "spin not minimal for {deg}°: {angle}"
            );
        }
    }

    #[test]
    fn an_isotropic_fan_has_no_dominant_direction() {
        // Spokes fanning out uniformly — no prominent grid direction. Each
        // spoke is a boundary edge (single adjacent face), so all are features.
        let n = 72;
        let mut verts = vec![0.0, 0.0, 0.0];
        let mut indices = Vec::new();
        for k in 0..n {
            let a = (k as f32) / (n as f32) * std::f32::consts::TAU;
            verts.extend_from_slice(&[a.cos(), a.sin(), 0.0]);
            let rim = (k + 1) as u32;
            indices.extend_from_slice(&[0, rim, 0]);
        }
        assert!(axis_alignment_rotation(&verts, &indices, AlignAxis::X).is_none());
    }
}
