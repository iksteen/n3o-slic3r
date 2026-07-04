// Smoke test for connector (joint) cuts. The cut is deferred: a plane cut plus
// each connector returned as a separate peg/hole *volume* (the slice path
// subtracts holes per-layer in 2D) — so the halves stay the plain plane cut and
// nothing is baked in. These check the volumes come back tagged correctly and
// that MMU paint is carried onto the halves.

use slic3r_ffi::{
    cut_mesh_deferred, Connector, ConnectorShape, ConnectorStyle, ConnectorType,
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
fn deferred_cut_emits_connector_volumes_not_baked_geometry() {
    let (v, i) = unit_cube();
    let base = cut_mesh_deferred(&v, &i, ORIGIN, NORMAL, &[], None).expect("plain cut");
    let (base_pos, base_neg) = (base.pos, base.neg);
    let dowel = Connector {
        pos: [0.3, 0.3, 0.5],
        radius: 0.12,
        height: 0.5,
        r_tol: 0.0,
        h_tol: 0.0,
        z_angle: 0.0,
        ty: ConnectorType::Dowel,
        style: ConnectorStyle::Prism,
        shape: ConnectorShape::Circle,
    };
    let conns = [connector(ConnectorType::Plug, ConnectorStyle::Prism, ConnectorShape::Circle), dowel];
    let r = cut_mesh_deferred(&v, &i, ORIGIN, NORMAL, &conns, None).expect("deferred cut");

    // Halves are the plain plane cut — nothing baked in, so tri counts match.
    assert_eq!(r.pos.indices.len(), base_pos.indices.len(), "pos half left unbaked");
    assert_eq!(r.neg.indices.len(), base_neg.indices.len(), "neg half left unbaked");
    // Plug → peg(neg, part) + hole(pos, neg); Dowel → hole(pos) + hole(neg) + pin.
    assert_eq!(r.modifiers.len(), 4, "two connectors → four connector volumes");
    assert_eq!(r.dowels.len(), 1, "one dowel pin");
    assert_eq!(r.modifiers.iter().filter(|m| !m.negative).count(), 1, "one peg");
    assert_eq!(r.modifiers.iter().filter(|m| m.negative).count(), 3, "three holes");
    assert!(
        r.modifiers.iter().any(|m| !m.negative && m.half == 1),
        "the peg is a solid volume on the neg half",
    );
    assert!(r.modifiers.iter().all(|m| !m.vertices.is_empty()), "every volume has geometry");
}

#[test]
fn every_type_shape_style_cuts_without_error() {
    let (v, i) = unit_cube();
    for (ty, style, shape) in [
        (ConnectorType::Snap, ConnectorStyle::Prism, ConnectorShape::Circle),
        (ConnectorType::Plug, ConnectorStyle::Frustum, ConnectorShape::Square),
        (ConnectorType::Plug, ConnectorStyle::Prism, ConnectorShape::Hexagon),
        (ConnectorType::Dowel, ConnectorStyle::Frustum, ConnectorShape::Triangle),
    ] {
        let r = cut_mesh_deferred(&v, &i, ORIGIN, NORMAL, &[connector(ty, style, shape)], None)
            .expect("deferred cut");
        assert!(!r.pos.is_empty() && !r.neg.is_empty(), "{ty:?}/{style:?}/{shape:?}");
        assert!(!r.modifiers.is_empty(), "connector produced volumes: {ty:?}");
    }
}

#[test]
fn degenerate_connector_is_skipped_plain_cut_survives() {
    let (v, i) = unit_cube();
    let bad = Connector {
        radius: 0.0, // degenerate → skipped
        ..connector(ConnectorType::Plug, ConnectorStyle::Prism, ConnectorShape::Circle)
    };
    let r = cut_mesh_deferred(&v, &i, ORIGIN, NORMAL, &[bad], None).expect("deferred cut");
    assert!(!r.pos.is_empty() && !r.neg.is_empty(), "plain cut still returns");
    assert!(r.modifiers.is_empty() && r.dowels.is_empty(), "degenerate connector emits nothing");
}

#[test]
fn no_connectors_equals_a_plain_cut() {
    let (v, i) = unit_cube();
    let r = cut_mesh_deferred(&v, &i, ORIGIN, NORMAL, &[], None).expect("deferred cut");
    assert!(!r.pos.is_empty() && !r.neg.is_empty());
    assert!(r.modifiers.is_empty() && r.dowels.is_empty());
    assert!(r.pos.paint.is_none() && r.neg.paint.is_none(), "no paint in → no paint out");
}

#[test]
fn deferred_cut_preserves_paint_on_the_halves() {
    let (v, i) = unit_cube();
    let paint = vec!["4".to_string(); i.len() / 3];
    let r = cut_mesh_deferred(&v, &i, ORIGIN, NORMAL, &[], Some(&paint)).expect("deferred cut");
    for half in [&r.pos, &r.neg] {
        let p = half.paint.as_ref().expect("kept half carries paint");
        assert_eq!(p.len(), half.indices.len() / 3, "one paint string per triangle");
        assert!(p.iter().any(|s| s == "4"), "surface stays painted");
        assert!(p.iter().any(|s| s.is_empty()), "cut cap unpainted");
    }
}

#[test]
fn paint_survives_a_diagonal_cut() {
    // Exact-identity paint mapping happens in the cut-aligned frame; a non-axis
    // normal forces a real rotation, so this guards the frame round-trip the
    // +Z-normal test above can't.
    let (v, i) = unit_cube();
    let paint = vec!["4".to_string(); i.len() / 3];
    let s = (1.0f32 / 3.0).sqrt();
    let r = cut_mesh_deferred(&v, &i, [0.5, 0.5, 0.5], [s, s, s], &[], Some(&paint))
        .expect("diagonal painted cut");
    for half in [&r.pos, &r.neg] {
        let p = half.paint.as_ref().expect("kept half carries paint");
        assert!(p.iter().any(|s| s == "4"), "kept faces stay painted through a rotated cut");
        assert!(p.iter().any(|s| s.is_empty()), "the cut cap stays unpainted");
    }
}
