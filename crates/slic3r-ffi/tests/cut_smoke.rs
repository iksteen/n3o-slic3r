// Smoke test: does the engine plane-cut return two watertight halves?
// A unit cube cut by a mid-height horizontal plane must split into two
// non-empty, closed (Euler V-E+F==2) halves on the correct sides; a plane
// that misses the cube must leave one side empty and the other whole.

use slic3r_ffi::{cut_mesh, CutHalf};

/// Axis-aligned unit cube [0,1]^3 — 8 vertices, 12 triangles, outward winding.
fn unit_cube() -> (Vec<f32>, Vec<u32>) {
    #[rustfmt::skip]
    let verts: Vec<f32> = vec![
        0.0,0.0,0.0,  1.0,0.0,0.0,  1.0,1.0,0.0,  0.0,1.0,0.0, // z=0
        0.0,0.0,1.0,  1.0,0.0,1.0,  1.0,1.0,1.0,  0.0,1.0,1.0, // z=1
    ];
    #[rustfmt::skip]
    let idx: Vec<u32> = vec![
        0,2,1, 0,3,2, // bottom
        4,5,6, 4,6,7, // top
        0,1,5, 0,5,4, // y=0
        1,2,6, 1,6,5, // x=1
        2,3,7, 2,7,6, // y=1
        3,0,4, 3,4,7, // x=0
    ];
    (verts, idx)
}

/// Euler characteristic V - E + F for a triangle soup, counting each undirected
/// edge once. A closed manifold mesh gives 2.
fn euler(half: &CutHalf) -> i64 {
    use std::collections::HashSet;
    let v = (half.vertices.len() / 3) as i64;
    let f = (half.indices.len() / 3) as i64;
    let mut edges: HashSet<(u32, u32)> = HashSet::new();
    for t in half.indices.chunks_exact(3) {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            edges.insert((a.min(b), a.max(b)));
        }
    }
    v - edges.len() as i64 + f
}

#[test]
fn cut_unit_cube_through_the_middle_yields_two_closed_halves() {
    let (verts, idx) = unit_cube();
    let (pos, neg) = cut_mesh(&verts, &idx, [0.0, 0.0, 0.5], [0.0, 0.0, 1.0])
        .expect("cut_mesh should succeed");

    assert!(!pos.is_empty(), "+normal side (z>0.5) must have geometry");
    assert!(!neg.is_empty(), "-normal side (z<0.5) must have geometry");

    // Sides land on the right half-spaces (cap sits exactly on z=0.5).
    for z in pos.vertices.chunks_exact(3).map(|c| c[2]) {
        assert!(z >= 0.5 - 1e-3, "positive-half vertex below the plane: z={z}");
    }
    for z in neg.vertices.chunks_exact(3).map(|c| c[2]) {
        assert!(z <= 0.5 + 1e-3, "negative-half vertex above the plane: z={z}");
    }

    // Capped halves are closed: V - E + F == 2.
    assert_eq!(euler(&pos), 2, "positive half not watertight");
    assert_eq!(euler(&neg), 2, "negative half not watertight");
}

#[test]
fn cut_plane_missing_the_mesh_leaves_one_side_empty() {
    let (verts, idx) = unit_cube();
    // Plane well above the cube, normal +Z: everything is on the -Z side.
    let (pos, neg) = cut_mesh(&verts, &idx, [0.0, 0.0, 5.0], [0.0, 0.0, 1.0])
        .expect("cut_mesh should succeed");
    assert!(pos.is_empty(), "nothing should be above a plane that misses");
    assert!(!neg.is_empty(), "the whole cube is below the plane");
    // The untouched side keeps the original 12 triangles (no cut, no cap).
    assert_eq!(neg.indices.len() / 3, 12, "uncut half should be the full cube");
}

#[test]
fn cut_rejects_out_of_range_index() {
    let verts = vec![0.0f32; 9];
    let bad = vec![0u32, 1, 5];
    assert!(cut_mesh(&verts, &bad, [0.0, 0.0, 0.0], [0.0, 0.0, 1.0]).is_err());
}

#[test]
fn cut_rejects_degenerate_normal() {
    let (verts, idx) = unit_cube();
    assert!(cut_mesh(&verts, &idx, [0.0, 0.0, 0.5], [0.0, 0.0, 0.0]).is_err());
}
