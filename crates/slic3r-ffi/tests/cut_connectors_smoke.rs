// Smoke test for connector (joint) cuts: a Plug bakes a protruding peg into one
// half and a matching cavity into the other; a Dowel cuts a hole in both and
// emits a free pin; degenerate/no connectors fall back to a plain cut. These
// exercise the libslic3r mesh-boolean path, so a connector that silently failed
// would make the geometric assertions fail.

use slic3r_ffi::{
    cut_mesh, cut_mesh_connectors, Connector, ConnectorShape, ConnectorStyle, ConnectorType,
};

/// Axis-aligned unit cube [0,1]^3 — 8 vertices, 12 triangles, outward winding.
fn unit_cube() -> (Vec<f32>, Vec<u32>) {
    #[rustfmt::skip]
    let verts: Vec<f32> = vec![
        0.0,0.0,0.0,  1.0,0.0,0.0,  1.0,1.0,0.0,  0.0,1.0,0.0,
        0.0,0.0,1.0,  1.0,0.0,1.0,  1.0,1.0,1.0,  0.0,1.0,1.0,
    ];
    #[rustfmt::skip]
    let idx: Vec<u32> = vec![
        0,2,1, 0,3,2,  4,5,6, 4,6,7,  0,1,5, 0,5,4,
        1,2,6, 1,6,5,  2,3,7, 2,7,6,  3,0,4, 3,4,7,
    ];
    (verts, idx)
}

fn max_z(half: &slic3r_ffi::CutHalf) -> f32 {
    half.vertices.chunks_exact(3).map(|c| c[2]).fold(f32::MIN, f32::max)
}

fn connector(ty: ConnectorType, style: ConnectorStyle, shape: ConnectorShape) -> Connector {
    Connector {
        pos: [0.5, 0.5, 0.5], // center of the cut cross-section
        radius: 0.18,
        height: 0.5,
        r_tol: 0.0,
        h_tol: 0.0,
        z_angle: 0.0,
        ty,
        style,
        shape,
    }
}

const ORIGIN: [f32; 3] = [0.0, 0.0, 0.5];
const NORMAL: [f32; 3] = [0.0, 0.0, 1.0];

#[test]
fn plug_connector_adds_a_protruding_peg_and_a_cavity() {
    let (v, i) = unit_cube();
    let (base_pos, _base_neg) = cut_mesh(&v, &i, ORIGIN, NORMAL).expect("plain cut");
    let r = cut_mesh_connectors(
        &v,
        &i,
        ORIGIN,
        NORMAL,
        &[connector(ConnectorType::Plug, ConnectorStyle::Prism, ConnectorShape::Circle)],
        None,
    )
    .expect("connector cut");

    assert!(!r.pos.is_empty() && !r.neg.is_empty(), "both halves present");
    assert!(r.dowels.is_empty(), "a plug emits no free pin");
    // Peg side (neg, z<0.5 by default) gains a peg that pokes above the cut
    // plane — its max Z climbs past 0.5 (peg straddles, half sticks out to 0.75).
    assert!(max_z(&r.neg) > 0.6, "peg should protrude above the cut, got {}", max_z(&r.neg));
    // Hole side (pos) gains an internal cavity → more triangles than the plain cut.
    assert!(
        r.pos.indices.len() > base_pos.indices.len(),
        "hole side should gain cavity geometry",
    );
}

#[test]
fn dowel_connector_emits_a_pin_and_holes_both_halves() {
    let (v, i) = unit_cube();
    let (base_pos, base_neg) = cut_mesh(&v, &i, ORIGIN, NORMAL).expect("plain cut");
    let r = cut_mesh_connectors(
        &v,
        &i,
        ORIGIN,
        NORMAL,
        &[connector(ConnectorType::Dowel, ConnectorStyle::Prism, ConnectorShape::Circle)],
        None,
    )
    .expect("connector cut");

    assert_eq!(r.dowels.len(), 1, "one free dowel pin");
    assert!(!r.dowels[0].is_empty(), "the pin has geometry");
    assert!(r.pos.indices.len() > base_pos.indices.len(), "pos got a hole");
    assert!(r.neg.indices.len() > base_neg.indices.len(), "neg got a hole");
}

#[test]
fn snap_and_square_shapes_cut_without_error() {
    let (v, i) = unit_cube();
    for (ty, style, shape) in [
        (ConnectorType::Snap, ConnectorStyle::Prism, ConnectorShape::Circle),
        (ConnectorType::Plug, ConnectorStyle::Frustum, ConnectorShape::Square),
        (ConnectorType::Plug, ConnectorStyle::Prism, ConnectorShape::Hexagon),
    ] {
        let r = cut_mesh_connectors(&v, &i, ORIGIN, NORMAL, &[connector(ty, style, shape)], None)
            .expect("connector cut");
        assert!(!r.pos.is_empty() && !r.neg.is_empty(), "{ty:?}/{style:?}/{shape:?}");
    }
}

#[test]
fn degenerate_connector_is_skipped_plain_cut_survives() {
    let (v, i) = unit_cube();
    let bad = Connector {
        radius: 0.0, // degenerate → skipped
        ..connector(ConnectorType::Plug, ConnectorStyle::Prism, ConnectorShape::Circle)
    };
    let r = cut_mesh_connectors(&v, &i, ORIGIN, NORMAL, &[bad], None).expect("connector cut");
    assert!(!r.pos.is_empty() && !r.neg.is_empty(), "plain cut still returns");
    assert!(r.dowels.is_empty());
}

#[test]
fn many_dowels_batch_into_one_cut() {
    // Three disjoint dowels across the cross-section → all three pins, both
    // halves holed. Exercises the batched (merge-then-one-boolean) path that
    // replaced the per-connector loop.
    let (v, i) = unit_cube();
    let (base_pos, base_neg) = cut_mesh(&v, &i, ORIGIN, NORMAL).expect("plain cut");
    let dowel = |x: f32, y: f32| Connector {
        pos: [x, y, 0.5],
        radius: 0.1,
        height: 0.5,
        r_tol: 0.0,
        h_tol: 0.0,
        z_angle: 0.0,
        ty: ConnectorType::Dowel,
        style: ConnectorStyle::Prism,
        shape: ConnectorShape::Circle,
    };
    let conns = [dowel(0.3, 0.3), dowel(0.7, 0.3), dowel(0.5, 0.7)];
    let r = cut_mesh_connectors(&v, &i, ORIGIN, NORMAL, &conns, None).expect("connector cut");
    assert_eq!(r.dowels.len(), 3, "one pin per dowel");
    assert!(r.dowels.iter().all(|d| !d.is_empty()), "every pin has geometry");
    assert!(r.pos.indices.len() > base_pos.indices.len(), "pos got holes");
    assert!(r.neg.indices.len() > base_neg.indices.len(), "neg got holes");
}

#[test]
fn paint_survives_a_diagonal_cut() {
    // Exact-identity paint mapping happens in the cut-aligned frame; a non-axis
    // normal forces a real rotation, so this guards the frame round-trip the
    // +Z-normal test above can't.
    let (v, i) = unit_cube();
    let paint = vec!["4".to_string(); i.len() / 3];
    let s = (1.0f32 / 3.0).sqrt();
    let r = cut_mesh_connectors(&v, &i, [0.5, 0.5, 0.5], [s, s, s], &[], Some(&paint))
        .expect("diagonal painted cut");
    for half in [&r.pos, &r.neg] {
        let p = half.paint.as_ref().expect("kept half carries paint");
        assert!(p.iter().any(|s| s == "4"), "kept faces stay painted through a rotated cut");
        assert!(p.iter().any(|s| s.is_empty()), "the cut cap stays unpainted");
    }
}

#[test]
fn no_connectors_equals_a_plain_cut() {
    let (v, i) = unit_cube();
    let r = cut_mesh_connectors(&v, &i, ORIGIN, NORMAL, &[], None).expect("connector cut");
    assert!(!r.pos.is_empty() && !r.neg.is_empty());
    assert!(r.dowels.is_empty());
    assert!(r.pos.paint.is_none() && r.neg.paint.is_none(), "no paint in → no paint out");
}

#[test]
fn paint_survives_the_cut_and_the_cap_stays_clean() {
    let (v, i) = unit_cube();
    let tri_count = i.len() / 3;
    // Paint the whole cube filament 4 (per-triangle FacetsAnnotation = "4").
    let paint: Vec<String> = vec!["4".to_string(); tri_count];
    let r = cut_mesh_connectors(
        &v,
        &i,
        ORIGIN,
        NORMAL,
        &[connector(ConnectorType::Plug, ConnectorStyle::Prism, ConnectorShape::Circle)],
        Some(&paint),
    )
    .expect("painted connector cut");

    for half in [&r.pos, &r.neg] {
        let p = half.paint.as_ref().expect("kept half carries paint");
        assert_eq!(p.len(), half.indices.len() / 3, "one paint string per triangle");
        // Original surface faces stay painted; the cut cap + connector walls are
        // fresh interior geometry the remap leaves unpainted.
        assert!(p.iter().any(|s| s == "4"), "original paint survived the cut");
        assert!(p.iter().any(|s| s.is_empty()), "cut cap / connector faces stay unpainted");
    }
    // Dowel-free plug → no pins, and pins never carry paint anyway.
    assert!(r.dowels.is_empty());
}
