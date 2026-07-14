// Spike: does libslic3r's nester (Slic3r::arrangement::arrange, libnest2d) work
// headlessly through our FFI? We feed convex footprints + a rectangular bed and
// check the returned translations/rotations/bed indices are sane: items land on
// the bed without overlapping, and an overfull bed spills onto extra beds.

use slic3r_ffi::ArrangePlacement;

/// A `w × h` rectangle at the origin, CCW.
fn rect(w: f64, h: f64) -> Vec<[f64; 2]> {
    vec![[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]]
}

/// World-space AABB of a no-rotation placement: the origin rectangle shifted by
/// its translation.
fn placed_aabb(item: &[[f64; 2]], p: &ArrangePlacement) -> [f64; 4] {
    let (w, h) = (item[1][0], item[2][1]);
    let (tx, ty) = (p.translation[0], p.translation[1]);
    [tx, ty, tx + w, ty + h]
}

#[test]
fn arranges_a_few_rectangles_onto_one_bed() {
    let _ = slic3r_ffi::init(None, 3);
    let items = vec![rect(50.0, 50.0), rect(40.0, 60.0), rect(30.0, 30.0)];
    let bed = [180.0, 180.0];
    let placements =
        slic3r_ffi::arrange(&items, &[], items.len(), bed, 2.0, false, false).expect("arrange should succeed");
    assert_eq!(placements.len(), 3);

    for p in &placements {
        assert_eq!(p.bed_idx, 0, "three small rects should all fit bed 0: {p:?}");
        assert!(p.rotation.abs() < 1e-9, "no rotation requested, got {}", p.rotation);
    }

    let boxes: Vec<[f64; 4]> = items
        .iter()
        .zip(&placements)
        .map(|(it, p)| placed_aabb(it, p))
        .collect();
    // Every footprint lands within the bed (small tolerance for spacing math).
    for b in &boxes {
        assert!(
            b[0] >= -1.0 && b[1] >= -1.0 && b[2] <= 181.0 && b[3] <= 181.0,
            "placement off the bed: {b:?}"
        );
    }
    // And no two footprints overlap.
    for i in 0..boxes.len() {
        for j in (i + 1)..boxes.len() {
            let (a, c) = (boxes[i], boxes[j]);
            let disjoint = a[2] <= c[0] + 1e-6
                || c[2] <= a[0] + 1e-6
                || a[3] <= c[1] + 1e-6
                || c[3] <= a[1] + 1e-6;
            assert!(disjoint, "items {i} and {j} overlap: {a:?} vs {c:?}");
        }
    }
}

#[test]
fn spills_overflow_onto_extra_beds() {
    let _ = slic3r_ffi::init(None, 3);
    // Twelve 70 mm tiles can't share a single 180 mm bed (~4 fit), so the nester
    // must spill the rest onto additional beds (bed_idx > 0) — the mechanism we
    // need for "auto-arrange, spilling to extra plates".
    let items: Vec<_> = (0..12).map(|_| rect(70.0, 70.0)).collect();
    let placements = slic3r_ffi::arrange(&items, &[], items.len(), [180.0, 180.0], 2.0, false, false)
        .expect("arrange should succeed");
    assert_eq!(placements.len(), 12);

    let max_bed = placements.iter().map(|p| p.bed_idx).max().unwrap();
    assert!(
        max_bed >= 1,
        "12×70mm tiles on a 180mm bed should spill to >=2 beds, got max bed_idx={max_bed}"
    );
    assert!(
        placements.iter().all(|p| p.bed_idx >= 0),
        "every item should be placed on some bed, got {placements:?}"
    );
}

#[test]
fn keeps_items_clear_of_an_exclusion_region() {
    let _ = slic3r_ffi::init(None, 3);
    // A back-left no-go region (e.g. an AMS feed zone). Items must avoid it.
    let items = vec![rect(40.0, 40.0), rect(40.0, 40.0)];
    let excl = [[0.0, 0.0, 60.0, 60.0]];
    let placements = slic3r_ffi::arrange(&items, &excl, items.len(), [180.0, 180.0], 2.0, false, false)
        .expect("arrange ok");
    for (it, p) in items.iter().zip(&placements) {
        assert_eq!(p.bed_idx, 0);
        let (w, h) = (it[1][0], it[2][1]);
        let (x0, y0) = (p.translation[0], p.translation[1]);
        let (x1, y1) = (x0 + w, y0 + h);
        // The placed footprint must not overlap the [0,60]×[0,60] region.
        let clear = x1 <= 1e-6 || x0 >= 60.0 - 1e-6 || y1 <= 1e-6 || y0 >= 60.0 - 1e-6;
        assert!(clear, "item at ({x0},{y0})-({x1},{y1}) overlaps the exclusion region");
    }
}

#[test]
fn excludes_are_hard_obstacles_on_a_crowded_bed() {
    let _ = slic3r_ffi::init(None, 3);
    // The earlier test leaves the bed roomy, so a *soft* scoring penalty would
    // also keep items clear — it can't tell soft from hard avoidance. Here we
    // crowd the bed: a 180mm bed fits ~16 of these 42mm tiles, but a 90×90
    // back-left exclusion steals four tile-slots. With a *soft* penalty the
    // nester would shove the overflow into the cheap-but-penalised corner;
    // only a *hard* fixed obstacle forces it to spill to a second bed and
    // leave the corner untouched. We assert both: nothing overlaps the corner
    // (on *any* bed — the exclusion is a per-plate obstacle reserved on every
    // bed, so the spilled surplus must dodge it too), and the surplus spills.
    let items: Vec<_> = (0..16).map(|_| rect(42.0, 42.0)).collect();
    let excl = [[0.0, 0.0, 90.0, 90.0]];
    let placements = slic3r_ffi::arrange(&items, &excl, items.len(), [180.0, 180.0], 2.0, false, false)
        .expect("arrange ok");
    assert_eq!(placements.len(), 16);

    for (it, p) in items.iter().zip(&placements) {
        let (w, h) = (it[1][0], it[2][1]);
        let (x0, y0) = (p.translation[0], p.translation[1]);
        let (x1, y1) = (x0 + w, y0 + h);
        let clear = x1 <= 1e-6 || x0 >= 90.0 - 1e-6 || y1 <= 1e-6 || y0 >= 90.0 - 1e-6;
        assert!(
            clear,
            "item on bed {} at ({x0},{y0})-({x1},{y1}) overlaps the hard exclusion region",
            p.bed_idx
        );
    }

    let max_bed = placements.iter().map(|p| p.bed_idx).max().unwrap();
    assert!(
        max_bed >= 1,
        "a hard exclusion should crowd the surplus onto a 2nd bed, got max bed_idx={max_bed}"
    );
}

#[test]
fn rejects_a_degenerate_contour() {
    let _ = slic3r_ffi::init(None, 3);
    let items = vec![vec![[0.0, 0.0], [10.0, 0.0]]]; // only 2 points
    assert!(slic3r_ffi::arrange(&items, &[], 1, [180.0, 180.0], 0.0, false, false).is_err());
}
