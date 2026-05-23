//! Bundled-A1-mini smoke entrypoint (PR-3-4).
//!
//! Phase 3 needs *some* way for the slice button to ship an end-to-end
//! job without the project-management UI Phase 4 will build. This
//! module wraps the orchestrator's `start_slice_job` with a fixed
//! cascade (the bundled `profiles/cascades/bambu-a1-mini-default.toml`)
//! and the canonical A1 mini printer / Textured PEI plate / Generic
//! PLA context — the same triple the integration test in
//! `src-tauri/tests/slice_orchestrator.rs` exercises.
//!
//! When Phase 4's profile UI lands the frontend will build
//! `SliceJobInput` from project state and call `slice_start_job`
//! directly; this module retires.

use std::path::Path;
use std::sync::Mutex;

use tauri::{AppHandle, State};

use super::job::{JobId, JobRegistry, SliceJobInput};
use super::orchestrator::{start_slice_job, SliceStartError};
use crate::core::cascade::commands::{CascadeHandle, ContextJson};
use crate::core::cascade::loader::parse_cascade_str;
use crate::core::cascade::validate::{default_known_dimensions, validate_cascade};
use crate::core::cascade::{Cascade, CascadeRegistry};
use crate::core::filament::FilamentProfile;
use crate::core::printer::profile::{BoundingBox, PrinterProfile, Toolhead};
use crate::core::scene::build_plate::{BuildPlate, SurfaceKind};

/// Embedded copy of the bundled cascade. `include_str!` ties the
/// file's content into the binary at build time so the packaged app
/// works without runtime resource lookup.
const A1_MINI_CASCADE_TOML: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../profiles/cascades/bambu-a1-mini-default.toml"
));

fn canonical_printer() -> PrinterProfile {
    PrinterProfile {
        model: "Bambu A1 mini".into(),
        slot_count: 4,
        supported_build_plates: vec![
            "Cool".into(),
            "Textured PEI".into(),
            "Smooth PEI".into(),
            "Engineering".into(),
            "SuperTack".into(),
        ],
        toolheads: vec![Toolhead {
            nozzle_diameter: 0.4,
            hotend_type: "stainless_steel".into(),
            max_temp: 300.0,
            slot_indices: vec![0, 1, 2, 3],
        }],
        build_volume: BoundingBox {
            min: [0.0, 0.0, 0.0],
            max: [180.0, 180.0, 180.0],
        },
        exclusion_zones: vec![],
    }
}

fn canonical_plate() -> BuildPlate {
    BuildPlate {
        identity: "Textured PEI".into(),
        libslic3r_curr_bed_type: "Textured PEI Plate".into(),
        surface_kind: SurfaceKind::PEI,
    }
}

fn canonical_filament() -> FilamentProfile {
    FilamentProfile {
        identity: "Generic PLA".into(),
        base_type: "PLA".into(),
        vendor: None,
        color: None,
    }
}

/// Parse + validate the bundled cascade and insert it into the
/// shared registry, returning the allocated handle. Re-runs from
/// scratch every call; cost is dominated by the small TOML parse
/// (~milliseconds) and a slice job already runs for orders of
/// magnitude longer, so caching the handle would only complicate
/// test setup without measurable benefit.
fn install_cascade(cascades: &mut CascadeRegistry) -> Result<CascadeHandle, String> {
    let label = Path::new("profiles/cascades/bambu-a1-mini-default.toml");
    let rules = parse_cascade_str(A1_MINI_CASCADE_TOML, label)
        .map_err(|e| format!("bundled cascade parse: {e}"))?;
    let cascade = Cascade { rules };
    if let Err(errs) = validate_cascade(&cascade, &default_known_dimensions()) {
        let msg = errs
            .iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        return Err(format!("bundled cascade validate: {msg}"));
    }
    Ok(cascades.insert(cascade))
}

/// Build the canonical default `SliceJobInput` for the bundled A1
/// mini path. Exposed for tests; the Tauri command below composes
/// this with the cascade lookup + orchestrator spawn.
pub fn default_input(
    model_path: String,
    output_dir: String,
    cascade_handle: CascadeHandle,
) -> SliceJobInput {
    SliceJobInput {
        model_path,
        output_dir,
        cascade_handle,
        context: ContextJson {
            printer: canonical_printer(),
            plate: canonical_plate(),
            filaments: vec![canonical_filament()],
            active_slot: 0,
            user_overrides: vec![],
            project_overrides: vec![],
            object_overrides: std::collections::HashMap::new(),
        },
        plate_ids: vec![1],
    }
}

/// Smoke-test entrypoint: slice `model_path` to
/// `<output_dir>/plate_1.gcode` using the bundled cascade. Frontend
/// calls this until Phase 4's profile UI ships.
#[tauri::command]
#[tracing::instrument(skip(app_handle, jobs, cascades))]
pub fn slice_start_default_a1mini(
    model_path: String,
    output_dir: String,
    app_handle: AppHandle,
    jobs: State<JobRegistry>,
    cascades: State<Mutex<CascadeRegistry>>,
) -> Result<JobId, String> {
    let mut cascades = cascades
        .lock()
        .map_err(|e| format!("cascade registry lock: {e}"))?;
    let handle = install_cascade(&mut cascades)?;
    let input = default_input(model_path, output_dir, handle);
    start_slice_job(input, app_handle, jobs.inner(), &cascades)
        .map_err(|e: SliceStartError| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use slic3r_ffi::init as ffi_init;
    use std::sync::Once;

    static FFI: Once = Once::new();
    fn ensure_ffi() {
        FFI.call_once(|| {
            ffi_init(None, 3).expect("libslic3r init");
        });
    }

    #[test]
    fn bundled_cascade_parses_and_validates() {
        ensure_ffi();
        let mut reg = CascadeRegistry::new();
        let handle = install_cascade(&mut reg).expect("parse + validate");
        assert!(reg.get(handle).is_some());
    }

    #[test]
    fn default_input_targets_plate_1() {
        let input = default_input("/tmp/m.stl".into(), "/tmp".into(), 42);
        assert_eq!(input.plate_ids, vec![1]);
        assert_eq!(input.cascade_handle, 42);
        assert_eq!(input.context.filaments.len(), 1);
    }
}
