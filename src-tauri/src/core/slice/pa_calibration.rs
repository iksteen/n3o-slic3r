//! Manual pressure-advance calibration: slice a PA-Line test print for one
//! loaded slot's filament. The user prints it, reads the best line's K by eye,
//! and types that back into the Flow Dynamics tab's manual-K field.
//!
//! Reuses libslic3r's own PA-Line generator (vendored OrcaSlicer
//! `CalibPressureAdvanceLine`): arming [`slic3r_ffi::CalibParams`] on the slice
//! makes libslic3r synthesize the swept-K line pattern (with inline numeric
//! labels) in place of the anchor object's toolpaths, emitting per-line
//! `SET_PRESSURE_ADVANCE` (Klipper) / `M900 K` (Marlin/Bambu). We assemble the
//! same (config, model) a normal slice would — but for a single chosen slot and
//! with no project plate — so it works for any printer, auto-cali or not.

use std::path::Path;

use slic3r_ffi::{slice_outcome_calib, CalibParams, Model};

use crate::core::cascade::overrides::{resolve_with_overrides, to_resolved, OverrideTiers};
use crate::core::cascade_adapter::adapt;
use crate::core::filament::{self, FilamentProfile};
use crate::core::printer::{self, PrinterInstance, SlotRef};
use crate::core::profile_library::{compose_cascade, with_quality_profile};
use crate::core::project::SlicingContext;
use crate::core::scene::build_plate;
use crate::core::scene::primitives::{generate, PrimitiveKind, PrimitiveParams};

/// Slice a PA-Line calibration print for `slot` on `instance` into `out_path`.
///
/// `start`/`end`/`step` bound the pressure-advance sweep (caller-validated,
/// editable in the UI). The output G-code is the swept-K line test; nothing is
/// applied to the printer here — sending + reading the result are separate.
pub fn slice_pa_calibration(
    instance: &PrinterInstance,
    slot: SlotRef,
    start: f64,
    end: f64,
    step: f64,
    out_path: &Path,
) -> Result<(), String> {
    let profile = printer::lookup(&instance.vendor_profile_ref)
        .ok_or_else(|| format!("printer profile '{}' not found", instance.vendor_profile_ref))?;

    // Bed: the instance's loaded plate, with the same synthesized fallback the
    // normal slice path uses for accepted-but-unauthored plates.
    let bed_identity = instance.bed.identity.clone();
    let build_plate = build_plate::lookup(&bed_identity).unwrap_or_else(|| {
        crate::core::scene::build_plate::BuildPlate {
            identity: bed_identity.clone(),
            libslic3r_curr_bed_type: format!("{bed_identity} Plate"),
        }
    });

    // The filament bound to the chosen slot drives both the cascade (its PA
    // table / material fragment) and the `filament.*` predicates. Same fallback
    // shape as `build_slice_input`.
    let identity = instance
        .extruders
        .get(slot.extruder as usize)
        .and_then(|ext| ext.slots.get(slot.slot as usize))
        .and_then(|s| s.filament_identity.as_deref())
        .unwrap_or(instance.default_filament_fragment_slug.as_str());
    let filament = filament::lookup(identity).unwrap_or_else(|| FilamentProfile {
        identity: identity.to_owned(),
        base_type: "PLA".into(),
        vendor: None,
        color: None,
    });

    // Cascade for a single-filament "plate": one slot in the material layout.
    let effective = with_quality_profile(instance, None);
    let cascade = compose_cascade(&effective, &[Some(slot)])
        .map_err(|e| format!("compose cascade: {e}"))?;

    let ctx = SlicingContext::new(
        std::sync::Arc::new(profile),
        std::sync::Arc::new(build_plate),
        vec![std::sync::Arc::new(filament)],
    );

    let resolved = to_resolved(&resolve_with_overrides(
        &cascade,
        &OverrideTiers::default(),
        &ctx,
    ));

    // Center the anchor on the bed: object world coords are bed-corner space
    // (printable_area origin), so an identity-placed object lands half off the
    // corner and fails validation. Read the bed centroid off printable_area.
    let (cx, cy) = bed_center(&resolved).unwrap_or((0.0, 0.0));

    let mut config = adapt(&resolved, &ctx)
        .map_err(|e| format!("adapter: {e}"))?
        .config;

    // Match OrcaSlicer's `Plater::calib_pa` setup for the Line method: disable
    // the features that would distort a clean calibration print. Best-effort —
    // a key absent from this printer's schema is simply skipped.
    for key in ["overhang_reverse", "precise_z_height", "resonance_avoidance"] {
        let _ = config.set(key, "0");
    }

    // A small anchor cube: libslic3r replaces its toolpaths with the PA-Line
    // pattern, so it only has to exist and validate on the bed.
    let mesh = generate(PrimitiveKind::Cube, PrimitiveParams::defaults_for(PrimitiveKind::Cube));
    let half_h = PrimitiveParams::defaults_for(PrimitiveKind::Cube).height as f64 / 2.0;
    let transform = translation_matrix(cx, cy, half_h);

    let mut model = Model::new().map_err(|e| format!("model alloc: {e}"))?;
    model
        .add_object("pa_calib", &mesh.vertices, &mesh.indices, &transform, 1, &[], &[], &[])
        .map_err(|e| format!("add anchor object: {e}"))?;

    let calib = CalibParams::pa_line(start, end, step);
    slice_outcome_calib(&model, &config, out_path, Some(calib), |_, _| {})
        .result
        .map(|_| ())
        .map_err(|e| format!("slice: {e}"))
}

/// Column-major (Eigen/libslic3r) 4×4 with only a translation.
fn translation_matrix(x: f64, y: f64, z: f64) -> [f64; 16] {
    let mut m = [0.0; 16];
    m[0] = 1.0;
    m[5] = 1.0;
    m[10] = 1.0;
    m[15] = 1.0;
    m[12] = x;
    m[13] = y;
    m[14] = z;
    m
}

/// Centroid of the bed polygon from a resolved config's `printable_area`
/// (`"0x0,256x0,256x256,0x256"`). `None` if the key is missing or unparsable —
/// caller then falls back to the origin.
fn bed_center(
    resolved: &std::collections::BTreeMap<String, crate::core::cascade::resolver::ResolvedValue>,
) -> Option<(f64, f64)> {
    let raw = &resolved.get("printable_area")?.value;
    let mut n = 0.0f64;
    let (mut sx, mut sy) = (0.0f64, 0.0f64);
    for pt in raw.split(',') {
        let (x, y) = pt.trim().split_once('x')?;
        sx += x.trim().parse::<f64>().ok()?;
        sy += y.trim().parse::<f64>().ok()?;
        n += 1.0;
    }
    (n > 0.0).then(|| (sx / n, sy / n))
}
