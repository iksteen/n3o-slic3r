// Smoke test: does the engine auto-orient actually return a rotation?
// A cone lying on its side has a large overhang; orient() should reorient it
// (stand it up), so the result must NOT be identity.

use slic3r_ffi::orient_mesh;

#[test]
fn orient_reorients_a_cone_lying_on_its_side() {
    let n = 24usize;
    let h = 30.0f32;
    let r = 8.0f32;
    let mut verts: Vec<f32> = Vec::new();
    // apex along +X (cone lying down)
    verts.extend_from_slice(&[h, 0.0, 0.0]); // vertex 0
    // base ring in the YZ plane at x = 0
    for i in 0..n {
        let t = (i as f32) / (n as f32) * std::f32::consts::TAU;
        verts.extend_from_slice(&[0.0, r * t.cos(), r * t.sin()]);
    }
    verts.extend_from_slice(&[0.0, 0.0, 0.0]); // base center, vertex n+1
    let apex = 0u32;
    let center = (n as u32) + 1;
    let mut idx: Vec<u32> = Vec::new();
    for i in 0..n {
        let a = 1 + i as u32;
        let b = 1 + ((i + 1) % n) as u32;
        idx.extend_from_slice(&[apex, a, b]); // side
        idx.extend_from_slice(&[center, b, a]); // base (reversed winding)
    }

    let q = orient_mesh(&verts, &idx, None).expect("orient_mesh should succeed");
    eprintln!("cone orient quaternion (x,y,z,w) = {q:?}");
    let is_identity = q[0].abs() < 1e-2
        && q[1].abs() < 1e-2
        && q[2].abs() < 1e-2
        && (q[3].abs() - 1.0).abs() < 1e-2;
    assert!(
        !is_identity,
        "expected a reorientation for a lying cone, got near-identity {q:?}"
    );
}

#[test]
fn orient_rejects_out_of_range_index() {
    // 3 vertices, but a triangle references vertex 5 — must be rejected at the
    // safe boundary, not passed into libslic3r's unchecked vertex access.
    let verts = vec![0.0f32; 9];
    let bad_indices = vec![0u32, 1, 5];
    assert!(orient_mesh(&verts, &bad_indices, None).is_err());
}
