//! Object library — catalog the UI's scaffolding panel reads from.
//!
//! Two sections:
//! - **Primitives** — procedurally generated meshes (see
//!   [`super::primitives`]).
//! - **Imported** — meshes registered in the live scene; the UI
//!   uses this to re-instance an already-loaded model without
//!   re-reading the source file.

use serde::{Deserialize, Serialize};

use super::primitives::{PrimitiveKind, PrimitiveParams};
use super::state::MeshId;
use crate::core::printer::profile::BoundingBox;
use crate::core::project::Project;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrimitiveDescriptor {
    pub kind: PrimitiveKind,
    pub display_name: String,
    pub default_params: PrimitiveParams,
    /// Which fields of `PrimitiveParams` apply to this kind. The
    /// frontend uses this list to render the parameter form (cube
    /// shows width/depth/height, cylinder shows radius/height, etc.).
    pub fields: Vec<String>,
}

pub fn list_primitives() -> Vec<PrimitiveDescriptor> {
    use PrimitiveKind::*;
    vec![
        PrimitiveDescriptor {
            kind: Cube,
            display_name: "Cube".into(),
            default_params: PrimitiveParams::defaults_for(Cube),
            fields: vec!["width".into(), "depth".into(), "height".into()],
        },
        PrimitiveDescriptor {
            kind: Cylinder,
            display_name: "Cylinder".into(),
            default_params: PrimitiveParams::defaults_for(Cylinder),
            fields: vec!["radius".into(), "height".into(), "radial_segments".into()],
        },
        PrimitiveDescriptor {
            kind: Sphere,
            display_name: "Sphere".into(),
            default_params: PrimitiveParams::defaults_for(Sphere),
            fields: vec!["radius".into(), "radial_segments".into()],
        },
        PrimitiveDescriptor {
            kind: Cone,
            display_name: "Cone".into(),
            default_params: PrimitiveParams::defaults_for(Cone),
            fields: vec!["radius".into(), "height".into(), "radial_segments".into()],
        },
        PrimitiveDescriptor {
            kind: Torus,
            display_name: "Torus".into(),
            default_params: PrimitiveParams::defaults_for(Torus),
            fields: vec!["width".into(), "radius".into(), "radial_segments".into()],
        },
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedDescriptor {
    pub mesh_id: MeshId,
    pub name: String,
    pub bounding_box: BoundingBox,
}

/// Snapshot of the meshes registered in the live scene. The
/// `name` field comes from the *first* object instantiated from
/// each mesh — the same value the user sees in the outliner.
pub fn list_imported(state: &Project) -> Vec<ImportedDescriptor> {
    let mut out = Vec::new();
    for (mesh_id, mesh) in &state.meshes {
        let name = state
            .active_plate()
            .scene
            .objects
            .values()
            .find(|o| o.mesh == *mesh_id)
            .map(|o| o.name.clone())
            .unwrap_or_else(|| match &mesh.provenance {
                super::state::MeshProvenance::File(p) => p.clone(),
                super::state::MeshProvenance::Primitive(n) => n.clone(),
            });
        out.push(ImportedDescriptor {
            mesh_id: *mesh_id,
            name,
            bounding_box: mesh.bounding_box,
        });
    }
    out.sort_by_key(|d| d.mesh_id.0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitives_list_has_all_five_kinds() {
        let prims = list_primitives();
        assert_eq!(prims.len(), 5);
        let kinds: Vec<PrimitiveKind> = prims.iter().map(|p| p.kind).collect();
        assert!(kinds.contains(&PrimitiveKind::Cube));
        assert!(kinds.contains(&PrimitiveKind::Cylinder));
        assert!(kinds.contains(&PrimitiveKind::Sphere));
        assert!(kinds.contains(&PrimitiveKind::Cone));
        assert!(kinds.contains(&PrimitiveKind::Torus));
    }
}
