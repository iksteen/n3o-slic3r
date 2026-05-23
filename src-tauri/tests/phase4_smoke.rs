//! Phase 4 exit-criteria smoke (PR-4-13).
//!
//! Chains the backend-side Phase 4 deliverables into one test so a
//! regression in the introspection surface or the capability filter
//! fails loudly. The frontend halves (form-component contracts,
//! categorize / mode-filter / diff / slot helpers, annotations
//! coverage) are exercised in the vitest suite — see
//! `docs/phase-4-smoke.md` for the full procedure.

use std::sync::Once;

use n3o_slic3r_lib::core::cascade::{
    slicer_options, slicer_options_for_printer, OptMode,
};
use n3o_slic3r_lib::core::printer::profile::{BoundingBox, PrinterProfile, Toolhead};
use n3o_slic3r_lib::core::schema::CapabilityPredicate;
use slic3r_ffi::init as ffi_init;

static FFI_INIT: Once = Once::new();
fn ensure_ffi() {
    FFI_INIT.call_once(|| {
        ffi_init(None, 3).expect("slic3r_init");
    });
}

fn a1_mini() -> PrinterProfile {
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

fn snapmaker_u1() -> PrinterProfile {
    PrinterProfile {
        model: "Snapmaker U1".into(),
        slot_count: 4,
        supported_build_plates: vec!["Textured PEI".into()],
        toolheads: (0..4)
            .map(|i| Toolhead {
                nozzle_diameter: 0.4,
                hotend_type: "stainless_steel".into(),
                max_temp: 300.0,
                slot_indices: vec![i],
            })
            .collect(),
        build_volume: BoundingBox {
            min: [0.0, 0.0, 0.0],
            max: [220.0, 220.0, 220.0],
        },
        exclusion_zones: vec![],
    }
}

/// Backend introspection coverage (PR-4-1):
///   - mode is surfaced
///   - scope bitmask round-trips for project / object / region keys
///   - capability predicates match the audit
#[test]
fn slicer_options_carries_phase4_introspection() {
    ensure_ffi();
    let opts = slicer_options(Some("layer_height".into()));
    let lh = opts
        .iter()
        .find(|o| o.key == "layer_height")
        .expect("layer_height present");
    // Mode is surfaced.
    assert_eq!(lh.mode, OptMode::Simple);
    // libslic3r ships a tooltip we surface.
    assert!(lh.tooltip.is_some());
    // Scope: layer_height is per-object (PrintObjectConfig).
    assert!(lh.scope.object, "layer_height is object-scope");
    assert!(!lh.scope.project, "layer_height is not project-scope");
    // No capability predicate gates layer_height.
    assert!(lh.capability.is_none());

    // wall_filament: PR-4-1 vocabulary check — region-scope.
    let wf_opts = slicer_options(Some("wall_filament".into()));
    let wf = wf_opts
        .iter()
        .find(|o| o.key == "wall_filament")
        .expect("wall_filament present");
    assert!(wf.scope.region, "wall_filament is region-scope");
}

/// A1 mini + U1 capability filter outcomes (FR-UI-7 exit criterion).
///   A1 mini hides toolchange options, U1 hides purge volumes
///   matrix; both show priming tower geometry settings...
#[test]
fn a1_mini_hides_toolchange_keys_u1_hides_purge_tower_keys() {
    ensure_ffi();
    let a1 = slicer_options_for_printer(a1_mini(), None);
    let u1 = slicer_options_for_printer(snapmaker_u1(), None);

    // A1 mini hides toolchanger geometry.
    let toolchanger_keys = [
        "extruder_clearance_radius",
        "machine_load_filament_time",
        "machine_unload_filament_time",
    ];
    for key in toolchanger_keys {
        let a1_row = a1.iter().find(|o| o.summary.key == key);
        if let Some(row) = a1_row {
            assert!(
                row.hidden,
                "A1 mini should hide toolchanger key {key:?}",
            );
            assert_eq!(
                row.summary.capability,
                Some(CapabilityPredicate::RequiresToolchanger),
            );
        }
    }

    // U1 hides purge tower keys.
    let purge_keys = [
        "flush_volumes_matrix",
        "enable_prime_tower",
        "prime_tower_width",
    ];
    for key in purge_keys {
        let u1_row = u1.iter().find(|o| o.summary.key == key);
        if let Some(row) = u1_row {
            assert!(row.hidden, "U1 should hide purge-tower key {key:?}");
            assert_eq!(
                row.summary.capability,
                Some(CapabilityPredicate::RequiresPurgeTower),
            );
        }
    }

    // Cross-check: A1 mini SHOWS purge-tower keys (it's AMS-style).
    let a1_purge = a1
        .iter()
        .find(|o| o.summary.key == "flush_volumes_matrix")
        .expect("flush_volumes_matrix in catalog");
    assert!(
        !a1_purge.hidden,
        "A1 mini should show purge-tower (it's AMS-style)",
    );

    // Cross-check: U1 SHOWS toolchanger keys.
    let u1_tc = u1
        .iter()
        .find(|o| o.summary.key == "extruder_clearance_radius");
    if let Some(row) = u1_tc {
        assert!(!row.hidden, "U1 should show toolchanger geometry");
    }
}

/// Render-budget gate (PR-4-1 / PR-4-4): the printer-aware option
/// list must complete fast enough that the panel's 50 ms re-render
/// budget can absorb it. Debug builds have ~10× more headroom than
/// release; the budget below is a debug-mode gate.
#[test]
fn slicer_options_for_printer_meets_render_budget_in_debug() {
    ensure_ffi();
    let start = std::time::Instant::now();
    let opts = slicer_options_for_printer(a1_mini(), None);
    let elapsed = start.elapsed();
    assert!(
        opts.len() >= 400,
        "expected ≥ 400 options, got {}",
        opts.len(),
    );
    assert!(
        elapsed < std::time::Duration::from_millis(500),
        "slicer_options_for_printer took {elapsed:?}; debug-budget 500 ms",
    );
}
