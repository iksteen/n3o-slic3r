//! Build-plate visualization + out-of-bounds check (PR-2-6).
//!
//! Owns the scene-side description of the active build plate: the
//! build-volume bounding box, the grid spacing the renderer draws,
//! the origin marker, and the printer's exclusion zones. Also the
//! [`object_out_of_bounds`] check the transform ops in PR-2-5 use
//! to emit non-blocking warnings.
//!
//! The renderer (PR-2-9) subscribes to the `scene:bed_changed`
//! event the scene emits when the active printer changes. The
//! cascade resolver never reads anything here — printer/build-volume
//! data flows the other way (PrinterProfile → BedMesh → renderer).

use serde::{Deserialize, Serialize};

use super::state::ExclusionZone;
use crate::core::printer::profile::{BoundingBox, PrinterProfile};

/// The bed visualization payload + bounds the scene state caches
/// per active printer. Rebuilt by [`bed_for_printer`] every time the
/// active printer changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BedMesh {
    /// Printer-specific build volume in world space. The renderer
    /// draws the grid over the bottom face (z = `extents.min[2]`)
    /// and clips object-shadows against this.
    pub extents: BoundingBox,
    /// Grid line spacing in millimeters. Default 10 mm; PR-2-9 may
    /// make this user-configurable from the viewport overlay.
    pub grid_spacing: f64,
    /// Origin marker location, useful when a printer centers its
    /// coordinate system on the bed center rather than the corner.
    /// MVP printers (Bambu A1 mini, Snapmaker U1) both home the
    /// nozzle to a corner, so this defaults to (0,0,0).
    pub origin_marker: [f64; 3],
    /// World-space exclusion zones (mirrored from
    /// `PrinterProfile.exclusion_zones`, transformed by the active
    /// plate's transform if it's not identity).
    pub exclusion_zones: Vec<ExclusionZone>,
}

/// Reasons a scene object can fail the out-of-bounds check. Multiple
/// reasons may apply to the same object (e.g., an object dragged
/// off-bed *and* dipped below z=0).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "data")]
pub enum OutOfBoundsReason {
    /// Object's world-space bounding box extends past the build
    /// volume in some axis. Carries the offending axis name for
    /// the UI to pick a specific message.
    OutOfBuildVolume { axis: BoundsAxis },
    /// Object intersects one of the printer's exclusion zones.
    /// Carries the zone label so the UI can say *which* zone
    /// (e.g., "AMS feed") rather than just "an exclusion zone".
    IntersectsExclusion { label: String },
    /// Object dips below the build plate (min Z < 0). This is the
    /// common case when the user rotates an object and forgets to
    /// drop it back down — PR-2-5's lay_flat is the one-keystroke
    /// fix.
    BelowBuildPlate,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum BoundsAxis {
    X,
    Y,
    Z,
}

/// Build a [`BedMesh`] from a printer profile. The grid spacing is
/// always 10 mm for MVP; the origin marker pulls from the build
/// volume's `min` corner (which is `[0, 0, 0]` for A1 mini and U1).
pub fn bed_for_printer(printer: &PrinterProfile) -> BedMesh {
    let zones: Vec<ExclusionZone> = printer
        .exclusion_zones
        .iter()
        .enumerate()
        .map(|(idx, bb)| ExclusionZone {
            label: format!("exclusion-{idx}"),
            bounds: *bb,
        })
        .collect();
    BedMesh {
        extents: printer.build_volume,
        grid_spacing: 10.0,
        origin_marker: printer.build_volume.min,
        exclusion_zones: zones,
    }
}

/// Return every reason `obj`'s world-space bounding box fails the
/// bed's out-of-bounds check. Empty Vec means the object is fully
/// inside the build volume and clear of every exclusion zone.
///
/// The check uses the *world-space* corners of the mesh's local
/// bounding box (transformed through the object's
/// [`Transform`](super::transform::Transform)). That's an
/// over-approximation when the object is rotated — a rotated bbox
/// reports a larger axis-aligned bbox than the actual mesh — but
/// that's the right MVP behavior: false positives are user-fixable
/// ("rotate me back"), false negatives ship a print off the bed.
pub fn object_out_of_bounds(
    obj: &super::state::SceneObject,
    mesh: &super::state::Mesh,
    bed: &BedMesh,
) -> Vec<OutOfBoundsReason> {
    let mut reasons = Vec::new();
    let world_bb = transformed_bbox(obj, mesh);

    let extents = bed.extents;
    if world_bb.min[0] < extents.min[0] || world_bb.max[0] > extents.max[0] {
        reasons.push(OutOfBoundsReason::OutOfBuildVolume {
            axis: BoundsAxis::X,
        });
    }
    if world_bb.min[1] < extents.min[1] || world_bb.max[1] > extents.max[1] {
        reasons.push(OutOfBoundsReason::OutOfBuildVolume {
            axis: BoundsAxis::Y,
        });
    }
    if world_bb.max[2] > extents.max[2] {
        // Below the plate is its own dedicated reason — split it
        // out so the UI message is specific ("object is below the
        // plate" vs. "object is too tall").
        reasons.push(OutOfBoundsReason::OutOfBuildVolume {
            axis: BoundsAxis::Z,
        });
    }
    if world_bb.min[2] < 0.0 {
        reasons.push(OutOfBoundsReason::BelowBuildPlate);
    }
    for zone in &bed.exclusion_zones {
        if bb_intersects(&world_bb, &zone.bounds) {
            reasons.push(OutOfBoundsReason::IntersectsExclusion {
                label: zone.label.clone(),
            });
        }
    }
    reasons
}

/// Bounding box of `mesh` after `obj.transform`. Computes by
/// transforming the 8 corners of the mesh's local bbox and unioning.
fn transformed_bbox(obj: &super::state::SceneObject, mesh: &super::state::Mesh) -> BoundingBox {
    let bb = &mesh.bounding_box;
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for &x in &[bb.min[0] as f32, bb.max[0] as f32] {
        for &y in &[bb.min[1] as f32, bb.max[1] as f32] {
            for &z in &[bb.min[2] as f32, bb.max[2] as f32] {
                let p = obj
                    .transform
                    .apply_point(glam::Vec3::new(x, y, z));
                for (axis, v) in [p.x as f64, p.y as f64, p.z as f64].iter().enumerate() {
                    if *v < min[axis] {
                        min[axis] = *v;
                    }
                    if *v > max[axis] {
                        max[axis] = *v;
                    }
                }
            }
        }
    }
    BoundingBox { min, max }
}

fn bb_intersects(a: &BoundingBox, b: &BoundingBox) -> bool {
    for axis in 0..3 {
        if a.max[axis] < b.min[axis] || a.min[axis] > b.max[axis] {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::printer::profile::Toolhead;
    use crate::core::scene::state::{Mesh, MeshId, MeshProvenance, SceneObject};
    use crate::core::scene::transform::Transform;

    fn a1_mini_with_ams_zone() -> PrinterProfile {
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
            // The A1 mini's AMS feed sits at the back-left corner,
            // ~30mm into the bed. Tests don't need a real value —
            // any reproducible zone proves the math.
            exclusion_zones: vec![BoundingBox {
                min: [0.0, 150.0, 0.0],
                max: [30.0, 180.0, 5.0],
            }],
            ..Default::default()
        }
    }

    fn unit_cube_at(translation: glam::Vec3) -> (SceneObject, Mesh) {
        // Manual SceneObject construction is fine — these tests
        // call object_out_of_bounds directly, not the SceneState
        // mutation API.
        let mesh = Mesh {
            id: MeshId(1),
            vertices: vec![],
            normals: vec![],
            indices: vec![],
            bounding_box: BoundingBox {
                min: [0.0, 0.0, 0.0],
                max: [1.0, 1.0, 1.0],
            },
            provenance: MeshProvenance::Primitive("unit-cube".into()),
        };
        let obj = SceneObject {
            id: crate::core::scene::state::ObjectId(1),
            mesh: MeshId(1),
            transform: Transform::translation(translation),
            name: "cube".into(),
            visible: true,
            extruder_id: None,
            parent: None,
            group_id: None,
        };
        (obj, mesh)
    }

    #[test]
    fn bed_for_a1_mini_carries_build_volume_and_zone() {
        let bed = bed_for_printer(&a1_mini_with_ams_zone());
        assert_eq!(bed.extents.max, [180.0, 180.0, 180.0]);
        assert_eq!(bed.grid_spacing, 10.0);
        assert_eq!(bed.origin_marker, [0.0, 0.0, 0.0]);
        assert_eq!(bed.exclusion_zones.len(), 1);
        assert_eq!(bed.exclusion_zones[0].label, "exclusion-0");
    }

    #[test]
    fn object_inside_volume_clears() {
        let bed = bed_for_printer(&a1_mini_with_ams_zone());
        let (obj, mesh) = unit_cube_at(glam::Vec3::new(50.0, 50.0, 0.0));
        let reasons = object_out_of_bounds(&obj, &mesh, &bed);
        assert!(
            reasons.is_empty(),
            "in-bounds object should have zero reasons, got {reasons:?}"
        );
    }

    #[test]
    fn object_past_build_volume_x_reports_axis() {
        let bed = bed_for_printer(&a1_mini_with_ams_zone());
        let (obj, mesh) = unit_cube_at(glam::Vec3::new(200.0, 50.0, 0.0));
        let reasons = object_out_of_bounds(&obj, &mesh, &bed);
        assert!(reasons.iter().any(|r| matches!(
            r,
            OutOfBoundsReason::OutOfBuildVolume { axis: BoundsAxis::X }
        )));
    }

    #[test]
    fn object_below_plate_reports_below() {
        let bed = bed_for_printer(&a1_mini_with_ams_zone());
        let (obj, mesh) = unit_cube_at(glam::Vec3::new(50.0, 50.0, -10.0));
        let reasons = object_out_of_bounds(&obj, &mesh, &bed);
        assert!(reasons
            .iter()
            .any(|r| matches!(r, OutOfBoundsReason::BelowBuildPlate)));
    }

    #[test]
    fn object_inside_ams_zone_reports_intersection_with_label() {
        let bed = bed_for_printer(&a1_mini_with_ams_zone());
        // Place cube inside the exclusion zone (min x=0, y=150, z=0).
        let (obj, mesh) = unit_cube_at(glam::Vec3::new(10.0, 160.0, 0.0));
        let reasons = object_out_of_bounds(&obj, &mesh, &bed);
        assert!(reasons.iter().any(|r| matches!(
            r,
            OutOfBoundsReason::IntersectsExclusion { label } if label == "exclusion-0"
        )));
    }
}
