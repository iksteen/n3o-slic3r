//! Pre-slice validation gate (PR-5-6 follow-up).
//!
//! Walks every plate the caller asked to slice and verifies its
//! material bindings via [`Plate::validate_material_bindings`]. If
//! any plate has an issue list — unbound model material, slot out
//! of range, duplicate binding, invalid binding field — the slice
//! command refuses to launch the worker thread and returns the
//! first offending plate's issues. The frontend surfaces them on
//! the binding panel.
//!
//! Out of scope: cascade validity (PR-1-3's load-time check + the
//! adapter's drop-list cover that), output-path writability
//! (the orchestrator checks before spawn), and per-plate printer
//! profile lookup (Phase 5 follow-up — today the gate uses the
//! single context-level `slot_count`).

use crate::core::project::binding::BindingIssue;
use crate::core::project::{PlateId, Project};

/// One plate's worth of binding issues, surfaced together so the
/// caller can render them as a unit on the binding panel.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PlateValidationFailure {
    /// 1-based plate id matching the wire-side `plate_ids` entry
    /// the caller asked to slice.
    pub plate_id: u32,
    pub issues: Vec<BindingIssue>,
}

/// Validate that every plate in `plate_ids` (that exists on
/// `project`) is ready to slice. Returns the **first** failing
/// plate; the slice command surfaces it as an error and the user
/// fixes that plate's bindings before a second attempt.
///
/// Plates absent from `project` are skipped — the orchestrator
/// today builds its libslic3r model from `input.model_path` on
/// disk, not from `Project`'s plate scene contents. When the
/// orchestrator switches to reading from `Project` (Phase 5
/// follow-up), this function will reject absent plate ids.
///
/// `slot_count` is caller-supplied: comes from the bound printer's
/// resolved profile.
pub fn validate_pre_slice(
    project: &Project,
    plate_ids: &[u32],
    slot_count: u8,
) -> Result<(), PlateValidationFailure> {
    for &raw_id in plate_ids {
        let pid = PlateId(raw_id);
        let Some(plate) = project.plate(pid) else {
            continue;
        };
        let issues = plate.validate_material_bindings(slot_count);
        if !issues.is_empty() {
            return Err(PlateValidationFailure {
                plate_id: raw_id,
                issues,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::project::binding::MaterialBinding;
    use crate::core::scene::state::{MeshProvenance, NewMesh, NewSceneObject};
    use crate::core::scene::transform::Transform;
    use crate::core::printer::profile::BoundingBox;

    fn unit_cube_mesh() -> NewMesh {
        NewMesh {
            vertices: vec![0.0; 24],
            normals: vec![0.0; 24],
            indices: vec![0, 1, 2],
            bounding_box: BoundingBox {
                min: [0.0, 0.0, 0.0],
                max: [1.0, 1.0, 1.0],
            },
            provenance: MeshProvenance::Primitive("cube".into()),
        }
    }

    /// Add a cube to the active plate carrying `extruder_id`.
    fn add_cube(p: &mut Project, mat: u8) {
        let mesh_id = p.register_mesh(unit_cube_mesh());
        p.register_object(NewSceneObject {
            mesh: mesh_id,
            transform: Transform::IDENTITY,
            name: format!("cube-m{mat}"),
            visible: true,
            extruder_id: Some(mat),
            parent: None,
        });
    }

    #[test]
    fn empty_plate_list_is_valid() {
        let p = Project::default();
        assert!(validate_pre_slice(&p, &[], 4).is_ok());
    }

    #[test]
    fn plate_with_no_referenced_materials_passes() {
        let p = Project::default();
        assert!(validate_pre_slice(&p, &[1], 4).is_ok());
    }

    #[test]
    fn plate_with_all_materials_bound_passes() {
        let mut p = Project::default();
        add_cube(&mut p, 1);
        p.set_material_binding(PlateId(1), 1, 1, "PLA".into()).unwrap();
        assert!(validate_pre_slice(&p, &[1], 4).is_ok());
    }

    #[test]
    fn plate_with_unbound_material_fails() {
        let mut p = Project::default();
        add_cube(&mut p, 2);
        let err = validate_pre_slice(&p, &[1], 4).unwrap_err();
        assert_eq!(err.plate_id, 1);
        assert!(err.issues.iter().any(|i| matches!(
            i,
            BindingIssue::UnboundMaterial { model_material: 2 },
        )));
    }

    #[test]
    fn plate_with_out_of_range_slot_fails() {
        let mut p = Project::default();
        add_cube(&mut p, 1);
        p.plates[0].material_bindings.push(MaterialBinding {
            model_material: 1,
            physical_slot: 5,
            filament_identity: "PLA".into(),
        });
        let err = validate_pre_slice(&p, &[1], 4).unwrap_err();
        assert!(err.issues.iter().any(|i| matches!(
            i,
            BindingIssue::SlotOutOfRange {
                model_material: 1,
                physical_slot: 5,
                slot_count: 4,
            },
        )));
    }

    #[test]
    fn absent_plate_id_is_skipped_not_an_error() {
        let p = Project::default();
        // plate 99 doesn't exist on the project; gate passes
        // (the orchestrator's own start_slice_job rejects via
        // a different code path if that ever matters).
        assert!(validate_pre_slice(&p, &[99], 4).is_ok());
    }

    #[test]
    fn first_failing_plate_short_circuits() {
        // Two plates: plate 1 has issues, plate 2 doesn't.
        // We expect the error to name plate 1; plate 2 isn't
        // walked.
        let mut p = Project::default();
        p.add_plate(None); // PlateId(2)
        // plate 1: object references material 5, no binding.
        add_cube(&mut p, 5);
        // plate 2: no objects, no issues.
        let err = validate_pre_slice(&p, &[1, 2], 4).unwrap_err();
        assert_eq!(err.plate_id, 1);
    }
}
