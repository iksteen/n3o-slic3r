// Smoke test: does the FFI paint session drive libslic3r's TriangleSelector
// with Orca semantics? A sphere stroke on the top of a unit cube paints
// enforcer facets; undo clears them; a 30-degree smart fill stays on the one
// flat face (does not leak across the 90-degree edges); and re-seeding a fresh
// session with the serialized strings reproduces the painted facets.

use slic3r_ffi::{BrushKind, PaintSession, PaintState};

/// Column-major 4x4 identity (mesh == world).
const IDENTITY: [f64; 16] = [
    1.0, 0.0, 0.0, 0.0, //
    0.0, 1.0, 0.0, 0.0, //
    0.0, 0.0, 1.0, 0.0, //
    0.0, 0.0, 0.0, 1.0, //
];

/// Axis-aligned unit cube [0,1]^3 — top face (z=1) is triangles 2 and 3.
fn unit_cube() -> (Vec<f32>, Vec<u32>) {
    #[rustfmt::skip]
    let verts: Vec<f32> = vec![
        0.0,0.0,0.0,  1.0,0.0,0.0,  1.0,1.0,0.0,  0.0,1.0,0.0, // z=0
        0.0,0.0,1.0,  1.0,0.0,1.0,  1.0,1.0,1.0,  0.0,1.0,1.0, // z=1
    ];
    #[rustfmt::skip]
    let idx: Vec<u32> = vec![
        0,2,1, 0,3,2, // bottom -> tri 0,1
        4,5,6, 4,6,7, // top    -> tri 2,3
        0,1,5, 0,5,4, // y=0
        1,2,6, 1,6,5, // x=1
        2,3,7, 2,7,6, // y=1
        3,0,4, 3,4,7, // x=0
    ];
    (verts, idx)
}

/// Total area of an indexed triangle mesh.
fn mesh_area(verts: &[f32], idx: &[u32]) -> f32 {
    let p = |i: u32| {
        let o = (i as usize) * 3;
        [verts[o], verts[o + 1], verts[o + 2]]
    };
    let mut area = 0.0;
    for t in idx.chunks_exact(3) {
        let a = p(t[0]);
        let b = p(t[1]);
        let c = p(t[2]);
        let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let cross = [
            ab[1] * ac[2] - ab[2] * ac[1],
            ab[2] * ac[0] - ab[0] * ac[2],
            ab[0] * ac[1] - ab[1] * ac[0],
        ];
        area += 0.5 * (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt();
    }
    area
}

fn nonempty_count(strings: &[String]) -> usize {
    strings.iter().filter(|s| !s.is_empty()).count()
}

#[test]
fn sphere_stroke_paints_then_undo_clears() {
    let (verts, idx) = unit_cube();
    let mut s = PaintSession::new(&verts, &idx, &[]).expect("open session");

    // Sphere brush centered on the top face; radius 0.3 stays clear of the
    // side walls (nearest is 0.5 away).
    s.stroke(
        2,
        [0.5, 0.5, 1.0],
        [0.5, 0.5, 10.0],
        &IDENTITY,
        0.3,
        BrushKind::Sphere,
        PaintState::Enforcer,
        true,
    )
    .expect("stroke");

    let painted = s.serialize().expect("serialize");
    assert!(nonempty_count(&painted) > 0, "stroke must paint some triangle");

    let (fv, fi) = s.facets(PaintState::Enforcer).expect("facets");
    assert!(!fi.is_empty(), "enforcer facets must be non-empty");
    assert!(mesh_area(&fv, &fi) > 0.0);

    assert!(s.undo(), "undo restores the pre-stroke snapshot");
    let after = s.serialize().expect("serialize after undo");
    assert_eq!(nonempty_count(&after), 0, "undo clears all paint");
    let (_, fi2) = s.facets(PaintState::Enforcer).expect("facets after undo");
    assert!(fi2.is_empty(), "no enforcer facets after undo");
    assert!(!s.undo(), "empty undo stack returns false");
}

#[test]
fn smart_fill_stays_on_the_flat_face() {
    let (verts, idx) = unit_cube();
    let mut s = PaintSession::new(&verts, &idx, &[]).expect("open session");

    // 30-degree fill from the top face: the two coplanar top triangles fill
    // (0-degree shared edge), but the 90-degree edges to the walls block it.
    s.fill(2, [0.5, 0.5, 1.0], &IDENTITY, 30.0, PaintState::Enforcer, true)
        .expect("fill");

    let (fv, fi) = s.facets(PaintState::Enforcer).expect("facets");
    let area = mesh_area(&fv, &fi);
    assert!(
        (0.9..=1.1).contains(&area),
        "fill must cover exactly one unit face (area 1.0), got {area} — leaked to a neighbor?"
    );
}

#[test]
fn serialized_paint_reseeds_a_fresh_session() {
    let (verts, idx) = unit_cube();
    let mut s = PaintSession::new(&verts, &idx, &[]).expect("open session");
    s.fill(2, [0.5, 0.5, 1.0], &IDENTITY, 30.0, PaintState::Enforcer, true)
        .expect("fill");
    let strings = s.serialize().expect("serialize");
    let (_, fi) = s.facets(PaintState::Enforcer).expect("facets");

    // Re-open with the serialized strings — the selector must rebuild the same
    // painted region.
    let s2 = PaintSession::new(&verts, &idx, &strings).expect("reseed session");
    let strings2 = s2.serialize().expect("serialize reseeded");
    assert_eq!(strings, strings2, "round-trip must be stable");
    let (_, fi2) = s2.facets(PaintState::Enforcer).expect("facets reseeded");
    assert_eq!(fi.len(), fi2.len(), "reseeded facets must match");
}
