//! Object library — catalog the UI's scaffolding panel reads from
//! (PR-2-7).
//!
//! Three sections:
//! - **Primitives** — procedurally generated meshes (see
//!   [`super::primitives`]).
//! - **Calibration** — file-based fixtures shipped under
//!   `external/OrcaSlicer/resources/`. We resolve paths relative to
//!   the cargo workspace root at build time; the runtime expects
//!   them to be vendored in the same layout. Some fixtures
//!   currently ship as Draco-compressed `.drc` files which our
//!   loaders don't understand — those surface as
//!   [`CalibrationAvailability::UnsupportedFormat`] so the UI can
//!   tell users *why* the entry is greyed out (vs. silently
//!   missing).
//! - **Imported** — meshes registered in the live scene; the UI
//!   uses this to re-instance an already-loaded model without
//!   re-reading the source file.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::primitives::{PrimitiveKind, PrimitiveParams};
use super::state::{MeshId, SceneState};
use crate::core::printer::profile::BoundingBox;

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
            fields: vec![
                "radius".into(),
                "height".into(),
                "radial_segments".into(),
            ],
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
            fields: vec![
                "radius".into(),
                "height".into(),
                "radial_segments".into(),
            ],
        },
        PrimitiveDescriptor {
            kind: Torus,
            display_name: "Torus".into(),
            default_params: PrimitiveParams::defaults_for(Torus),
            fields: vec![
                "width".into(),
                "radius".into(),
                "radial_segments".into(),
            ],
        },
    ]
}

/// One calibration fixture's descriptor. The UI greys out entries
/// whose `availability` isn't `Available` and surfaces the reason
/// in a tooltip.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationDescriptor {
    /// Stable id for the UI (also used as the key in the library
    /// sidebar's preferences).
    pub id: String,
    pub display_name: String,
    /// Short human-readable summary of what the calibration tests.
    pub description: String,
    pub availability: CalibrationAvailability,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data")]
pub enum CalibrationAvailability {
    /// File exists and our loader supports the format.
    Available { path: PathBuf },
    /// File ships in the vendored Orca resources but uses a format
    /// our loader doesn't understand yet (typically Draco-compressed
    /// `.drc`). Tracked as a follow-up.
    UnsupportedFormat { path: PathBuf, format: String },
    /// Expected fixture path is missing — the user's checkout might
    /// not have the OrcaSlicer submodule populated, or we
    /// mis-specified the path.
    MissingFromResources { expected_at: PathBuf },
}

/// Build the calibration catalog for the given printer model. The
/// material-flow entry is printer-specific (Bambu's flow test
/// differs from Snapmaker's); other entries are shared.
///
/// `resources_root` is the path to `external/OrcaSlicer/resources`;
/// at runtime this resolves relative to the app's bundle. Tests pass
/// the workspace-relative path.
pub fn list_calibration(
    printer_model: &str,
    resources_root: &Path,
) -> Vec<CalibrationDescriptor> {
    let mut out = Vec::new();

    out.push(check_calibration(
        "dimension-cube",
        "Dimension Cube (Orca Cube v2)",
        "XYZ accuracy and dimensional check at known size.",
        resources_root.join("handy_models/OrcaCube_v2.3mf"),
    ));

    out.push(check_calibration(
        "temperature-tower",
        "Temperature Tower",
        "Per-layer-band temperature sweep to find your filament's window.",
        resources_root.join("calib/temperature_tower/temperature_tower.drc"),
    ));

    out.push(check_calibration(
        "stringing-tower",
        "Stringing / Retraction Tower",
        "Retraction tuning — minimize strings between towers.",
        resources_root.join("calib/retraction/retraction_tower.drc"),
    ));

    out.push(material_flow_for(printer_model, resources_root));

    out
}

/// Resolve the right material-flow fixture per printer. Bambu A1
/// (and the rest of the BBS line) consume Orca's bundled
/// `Orca-LinearFlow.3mf`; Snapmaker U1 needs a per-machine fixture
/// we haven't sourced yet (the U1 ships with its own flow tower
/// that differs structurally from Orca's pattern). When that lands
/// the descriptor here updates.
fn material_flow_for(printer_model: &str, resources_root: &Path) -> CalibrationDescriptor {
    let lower = printer_model.to_ascii_lowercase();
    if lower.contains("bambu") || lower.contains("a1") || lower.contains("x1") || lower.contains("p1") {
        check_calibration(
            "material-flow",
            "Material Flow (Orca-LinearFlow)",
            "Linear advance / flow rate tuning for Bambu printers.",
            resources_root.join("calib/filament_flow/Orca-LinearFlow.3mf"),
        )
    } else if lower.contains("snapmaker") || lower.contains("u1") {
        // No vendored Snapmaker flow fixture yet; surface a
        // placeholder so the UI can still list the slot.
        CalibrationDescriptor {
            id: "material-flow".into(),
            display_name: "Material Flow (Snapmaker)".into(),
            description: "Per-toolhead flow calibration for Snapmaker U1.".into(),
            availability: CalibrationAvailability::MissingFromResources {
                expected_at: resources_root.join("calib/filament_flow/snapmaker_u1_flow.3mf"),
            },
        }
    } else {
        check_calibration(
            "material-flow",
            "Material Flow (generic)",
            "Generic flow calibration via Orca's Linear Flow pattern.",
            resources_root.join("calib/filament_flow/Orca-LinearFlow.3mf"),
        )
    }
}

fn check_calibration(
    id: &str,
    display_name: &str,
    description: &str,
    path: PathBuf,
) -> CalibrationDescriptor {
    let availability = if !path.exists() {
        CalibrationAvailability::MissingFromResources { expected_at: path }
    } else {
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_owned());
        match ext.as_deref() {
            Some("3mf") | Some("stl") | Some("obj") => {
                CalibrationAvailability::Available { path }
            }
            Some(e) => CalibrationAvailability::UnsupportedFormat {
                path,
                format: e.to_owned(),
            },
            None => CalibrationAvailability::UnsupportedFormat {
                path,
                format: "unknown".into(),
            },
        }
    };
    CalibrationDescriptor {
        id: id.into(),
        display_name: display_name.into(),
        description: description.into(),
        availability,
    }
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
pub fn list_imported(state: &SceneState) -> Vec<ImportedDescriptor> {
    let mut out = Vec::new();
    for (mesh_id, mesh) in &state.meshes {
        let name = state
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

    fn workspace_orca_resources() -> PathBuf {
        let crate_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
        let mut p = PathBuf::from(crate_dir);
        p.pop();
        p.push("external/OrcaSlicer/resources");
        p
    }

    #[test]
    fn calibration_for_a1_mini_returns_four_entries() {
        let root = workspace_orca_resources();
        if !root.exists() {
            eprintln!("skipping: orca resources missing at {root:?}");
            return;
        }
        let cals = list_calibration("Bambu A1 mini", &root);
        assert_eq!(cals.len(), 4);
        // Dimension cube + material flow should be Available
        // (both 3MF, both shipped in the submodule).
        let dim = cals.iter().find(|c| c.id == "dimension-cube").unwrap();
        assert!(matches!(
            dim.availability,
            CalibrationAvailability::Available { .. }
        ));
        let flow = cals.iter().find(|c| c.id == "material-flow").unwrap();
        assert!(matches!(
            flow.availability,
            CalibrationAvailability::Available { .. }
        ));
        // Temperature + stringing towers ship as .drc → unsupported.
        let temp = cals.iter().find(|c| c.id == "temperature-tower").unwrap();
        assert!(matches!(
            temp.availability,
            CalibrationAvailability::UnsupportedFormat { ref format, .. } if format == "drc"
        ));
        let string = cals.iter().find(|c| c.id == "stringing-tower").unwrap();
        assert!(matches!(
            string.availability,
            CalibrationAvailability::UnsupportedFormat { ref format, .. } if format == "drc"
        ));
    }

    #[test]
    fn calibration_for_snapmaker_u1_marks_flow_as_missing() {
        let root = workspace_orca_resources();
        if !root.exists() {
            return;
        }
        let cals = list_calibration("Snapmaker U1", &root);
        let flow = cals.iter().find(|c| c.id == "material-flow").unwrap();
        assert!(matches!(
            flow.availability,
            CalibrationAvailability::MissingFromResources { .. }
        ));
    }
}
