//! Mutation methods for [`Project`].
//!
//! Each public method takes `&mut Project` and returns the events
//! the renderer needs to apply. The convention is "pure functions
//! that return event lists; the Tauri layer emits each event via
//! `Window::emit`". Tests bypass the Tauri layer and inspect the
//! returned event list directly.
//!
//! Lives in a sibling module from [`super::model`] so the type
//! definitions stay focused; this module has the mechanics.
//!
//! The methods form one logical `impl Project` partitioned across
//! topic files (Rust allows multiple `impl` blocks for one type):
//!
//! - **`geometry`**: mesh/object registration + lifecycle, transforms,
//!   selection, grouping, bounds, cross-plate moves.
//! - **`materials`**: the `material → slot` mapping + auto-bind +
//!   orphan-binding pruning.
//! - **`plates`**: plate lifecycle, metadata, and per-plate printer
//!   binding.
//! - **`overrides`**: the user/project/object override tiers.
//!
//! A handful of private helpers are shared across topic files; those
//! are `pub(super)` so sibling modules can reach them (they remain
//! crate-internal).
//!
//! Plate addressing on the public surface is by [`PlateId`] (stable
//! across reorder + remove). Internal helpers use `usize` indices
//! when they need to mutate sibling plates — the borrow checker
//! wants index-then-deref, not a borrowed `Plate`.

mod geometry;
mod materials;
mod overrides;
mod plates;

/// Split-tool cut types (used by `core::scene::commands::scene_cut_apply`).
pub use geometry::{CutHalfOut, CutResult, CutSide, CutTarget};

/// Upper bound on `Plate.name` byte length. Holds back
/// pathological renames that would blow out the tab strip layout
/// or balloon the project's `.n3o` `project.json` skeleton; the actual UI
/// budget is ~24 chars but we accept up to 200 to leave headroom
/// for emoji / non-ASCII users.
pub const PLATE_NAME_MAX: usize = 200;

#[cfg(test)]
pub(crate) mod test_support {
    //! Shared fixtures for the per-topic `mod tests` blocks. Each topic
    //! file's tests glob-import these.
    use crate::core::printer::profile::{BoundingBox, PrinterProfile, Toolhead};
    use crate::core::project::model::{PlateId, Project};
    use crate::core::project::Session;
    use crate::core::scene::state::{MeshId, MeshProvenance, NewMesh, NewSceneObject, ObjectId};
    use crate::core::scene::transform::Transform;
    use std::collections::HashSet;

    /// Test-only: the active plate's selection from a session (empty when the
    /// plate has no runtime entry yet). Owned clone — fine for assertions.
    pub(crate) fn active_selection(session: &Session) -> HashSet<ObjectId> {
        plate_selection(session, session.project.active_plate().id)
    }

    /// Test-only: a plate's selection by id.
    pub(crate) fn plate_selection(session: &Session, id: PlateId) -> HashSet<ObjectId> {
        session
            .plate_runtime(id)
            .map(|r| r.selection.clone())
            .unwrap_or_default()
    }

    /// Test-only: resolve the active plate's bound `PrinterInstance` from the
    /// registry (`None` when unbound). The pure mutation methods take the
    /// instance as a parameter; auto-bind tests run under a `RegistryGuard` that
    /// populates the printer, so this resolves the realistic slot topology to
    /// pass in — the model itself never reaches for the registry.
    pub(crate) fn active_instance(p: &Project) -> Option<crate::core::printer::PrinterInstance> {
        p.active_plate()
            .printer_instance_id()
            .and_then(crate::core::printer::lookup_instance)
    }

    pub(crate) fn unit_cube_mesh() -> NewMesh {
        // 8-corner cube — enough geometry for tests that don't care
        // about visual quality. Normals left zeroed since the
        // mutation-method tests don't shade.
        NewMesh {
            vertices: vec![
                0.0, 0.0, 0.0, //
                1.0, 0.0, 0.0, //
                0.0, 1.0, 0.0, //
                1.0, 1.0, 0.0, //
                0.0, 0.0, 1.0, //
                1.0, 0.0, 1.0, //
                0.0, 1.0, 1.0, //
                1.0, 1.0, 1.0, //
            ],
            indices: vec![
                0, 1, 2, 1, 3, 2, // bottom
                4, 6, 5, 5, 6, 7, // top
                0, 4, 1, 1, 4, 5, // front
                2, 3, 6, 3, 7, 6, // back
                0, 2, 4, 2, 6, 4, // left
                1, 5, 3, 3, 5, 7, // right
            ],
            paint_colors: None,
            support_paint: None,
            bounding_box: BoundingBox {
                min: [0.0, 0.0, 0.0],
                max: [1.0, 1.0, 1.0],
            },
            provenance: MeshProvenance::Primitive("unit-cube".into()),
        }
    }

    /// Test-only helper that lands a unit cube at the world origin
    /// (transform identity). Doesn't use `load_mesh` because that
    /// path now lifts + centers on the bed — fine for the live
    /// importer (file-loaded meshes need to land on the plate) but
    /// noisy for the transform-math tests below, which want exact
    /// corner positions to assert against.
    pub(crate) fn add_cube(p: &mut Project) -> (MeshId, ObjectId) {
        let mesh_id = p.register_mesh(unit_cube_mesh());
        let inst = active_instance(p);
        let obj_id = p.register_object(NewSceneObject::at_origin(mesh_id, "cube"), inst.as_ref());
        (mesh_id, obj_id)
    }

    /// Octahedron of "radius" `r` centered on the local origin: vertices at
    /// (±r,0,0),(0,±r,0),(0,0,±r). Unlike a cube, its bounding-box corners are
    /// *not* vertices — the box corner (±r,±r,±r) sits at distance r√3 while the
    /// nearest surface is only r away. That gap is what separates a true-vertex
    /// settle from a bbox settle, and what makes the conservative (bbox) bounds
    /// check over-report below-plate once the solid is rotated.
    pub(crate) fn add_octahedron(p: &mut Project, r: f32) -> (MeshId, ObjectId) {
        let mesh = NewMesh {
            vertices: vec![
                r, 0.0, 0.0, // 0 +x
                -r, 0.0, 0.0, // 1 -x
                0.0, r, 0.0, // 2 +y
                0.0, -r, 0.0, // 3 -y
                0.0, 0.0, r, // 4 +z (apex)
                0.0, 0.0, -r, // 5 -z (nadir)
            ],
            indices: vec![
                4, 0, 2, 4, 2, 1, 4, 1, 3, 4, 3, 0, // top fan
                5, 2, 0, 5, 1, 2, 5, 3, 1, 5, 0, 3, // bottom fan
            ],
            paint_colors: None,
            support_paint: None,
            bounding_box: BoundingBox {
                min: [(-r) as f64, (-r) as f64, (-r) as f64],
                max: [r as f64, r as f64, r as f64],
            },
            provenance: MeshProvenance::Primitive("octahedron".into()),
        };
        let mesh_id = p.register_mesh(mesh);
        let inst = active_instance(p);
        let obj_id = p.register_object(
            NewSceneObject::at_origin(mesh_id, "octahedron"),
            inst.as_ref(),
        );
        (mesh_id, obj_id)
    }

    pub(crate) fn a1_mini_for_test() -> PrinterProfile {
        PrinterProfile {
            model: "Bambu Lab A1 mini".into(),
            supported_build_plates: vec!["Textured PEI".into()],
            toolheads: vec![Toolhead {
                default_nozzle_diameter: "0.4".to_string(),
                hotend_type: "stainless_steel".into(),
                max_temp: 300.0,
            }],
            build_volume: BoundingBox {
                min: [0.0, 0.0, 0.0],
                max: [180.0, 180.0, 180.0],
            },
            exclusion_zones: vec![],
            ..Default::default()
        }
    }

    pub(crate) fn cube_mesh() -> NewMesh {
        NewMesh {
            vertices: vec![0.0; 24],
            indices: vec![0, 1, 2],
            paint_colors: None,
            support_paint: None,
            bounding_box: BoundingBox {
                min: [0.0, 0.0, 0.0],
                max: [1.0, 1.0, 1.0],
            },
            provenance: MeshProvenance::Primitive("cube".into()),
        }
    }

    pub(crate) fn add_cube_with_material(p: &mut Project, mat: u8) -> ObjectId {
        let mesh_id = p.register_mesh(cube_mesh());
        let inst = active_instance(p);
        p.register_object(
            NewSceneObject {
                mesh: mesh_id,
                transform: Transform::IDENTITY,
                name: format!("cube-m{mat}"),
                visible: true,
                extruder_id: Some(mat),
                group: None,
            },
            inst.as_ref(),
        )
    }

    /// A single base-material-1 object whose mesh is MMU-painted with filament
    /// 2 (`paint_colors = ["8"]` → state 2). No object carries `extruder = 2`.
    pub(crate) fn add_painted_cube(p: &mut Project) -> ObjectId {
        let mesh = NewMesh {
            paint_colors: Some(vec!["8".into()]),
            ..cube_mesh()
        };
        let mesh_id = p.register_mesh(mesh);
        let inst = active_instance(p);
        p.register_object(
            NewSceneObject {
                mesh: mesh_id,
                transform: Transform::IDENTITY,
                name: "painted".into(),
                visible: true,
                extruder_id: Some(1),
                group: None,
            },
            inst.as_ref(),
        )
    }
}
