//! Phase 4 exit-criteria smoke (PR-4-13).
//!
//! Chains the backend-side Phase 4 deliverables into one test so a
//! regression in the introspection surface or the capability filter
//! fails loudly. The frontend halves (form-component contracts,
//! categorize / mode-filter / diff / slot helpers, annotations
//! coverage) are exercised in the vitest suite.

use std::sync::Once;

use n3o_slic3r_lib::core::printer::{slicer_options_for_printer, OptMode};
use n3o_slic3r_lib::core::printer::profile::{BoundingBox, PrinterProfile, Toolhead};
use n3o_slic3r_lib::core::printer::CapabilityPredicate;
use n3o_slic3r_lib::core::printer::options::PrinterAwareOptionSummary;
use slic3r_ffi::init as ffi_init;

static FFI_INIT: Once = Once::new();
fn ensure_ffi() {
    FFI_INIT.call_once(|| {
        ffi_init(None, 3).expect("slic3r_init");
    });
}

fn a1_mini() -> PrinterProfile {
    PrinterProfile {
        model: "Bambu Lab A1 mini".into(),
        ams_max: 1,
        supported_build_plates: vec![
            "Cool".into(),
            "Textured PEI".into(),
            "Smooth PEI".into(),
            "Engineering".into(),
            "SuperTack".into(),
        ],
        toolheads: vec![Toolhead {
            default_nozzle_diameter: "0.4".into(),
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

fn snapmaker_u1() -> PrinterProfile {
    PrinterProfile {
        model: "Snapmaker U1".into(),
        supported_build_plates: vec!["Textured PEI".into()],
        toolheads: (0..4)
            .map(|_i| Toolhead {
                default_nozzle_diameter: "0.4".into(),
                hotend_type: "stainless_steel".into(),
                max_temp: 300.0,
            })
            .collect(),
        build_volume: BoundingBox {
            min: [0.0, 0.0, 0.0],
            max: [220.0, 220.0, 220.0],
        },
        exclusion_zones: vec![],
        ..Default::default()
    }
}

/// Backend introspection coverage (PR-4-1):
///   - mode is surfaced
///   - scope bitmask round-trips for project / object / region keys
///   - capability predicates match the audit
#[test]
fn slicer_options_carries_phase4_introspection() {
    ensure_ffi();
    // Neither key is capability-gated, so the printer-aware view returns
    // them unhidden; read introspection off the inner `summary`.
    let opts = slicer_options_for_printer(a1_mini(), Some("layer_height".into()));
    let lh = opts
        .iter()
        .find(|o| o.summary.key == "layer_height")
        .expect("layer_height present");
    // Mode is surfaced.
    assert_eq!(lh.summary.mode, OptMode::Simple);
    // libslic3r ships a tooltip we surface.
    assert!(lh.summary.tooltip.is_some());
    // Scope: layer_height is per-object (PrintObjectConfig).
    assert!(lh.summary.scope.object, "layer_height is object-scope");
    assert!(!lh.summary.scope.project, "layer_height is not project-scope");
    // No capability predicate gates layer_height.
    assert!(lh.summary.capability.is_none());

    // outer_wall_filament_id: PR-4-1 vocabulary check — region-scope.
    // (renamed from wall_filament upstream.)
    let wf_opts = slicer_options_for_printer(a1_mini(), Some("outer_wall_filament_id".into()));
    let wf = wf_opts
        .iter()
        .find(|o| o.summary.key == "outer_wall_filament_id")
        .expect("outer_wall_filament_id present");
    assert!(wf.summary.scope.region, "outer_wall_filament_id is region-scope");
}

/// A1 mini + U1 capability filter outcomes for the surviving process-bucket
/// capability-gated keys (FR-UI-7 exit criterion).
///
/// After PR-S-2 the panel filters to Process bucket only — printer-bucket
/// toolchanger geometry (`extruder_clearance_radius`, `machine_*_filament_time`)
/// is gone from the panel entirely, so its capability-hide path no longer
/// applies. The purge-tower family stays in Process bucket and exercises the
/// same hide/show logic.
#[test]
fn priming_tower_shows_on_both_purge_amounts_hide_on_toolchanger() {
    ensure_ffi();
    let a1 = slicer_options_for_printer(a1_mini(), None);
    let u1 = slicer_options_for_printer(snapmaker_u1(), None);
    fn row<'a>(
        rows: &'a [PrinterAwareOptionSummary],
        key: &str,
    ) -> &'a PrinterAwareOptionSummary {
        rows.iter()
            .find(|o| o.summary.key == key)
            .unwrap_or_else(|| panic!("{key} should be in process-bucket catalog"))
    }

    // The priming tower is a multi-material feature, not a purge one: BOTH the
    // A1 mini (AMS purge tower) and the U1 (toolchanger, re-entry priming tower)
    // run one, so its keys show on both — gated on multi-slot, not purging.
    for key in ["enable_prime_tower", "prime_tower_width"] {
        assert!(!row(&a1, key).hidden, "A1 mini should show priming key {key:?}");
        assert!(!row(&u1, key).hidden, "U1 should show priming key {key:?}");
        assert_eq!(
            row(&u1, key).summary.capability,
            Some(CapabilityPredicate::RequiresMultiSlot),
        );
    }

    // Purge/flush *amounts* are AMS-only — the U1 swaps heads, nothing to flush.
    for key in ["flush_into_infill", "flush_into_objects"] {
        assert!(!row(&a1, key).hidden, "A1 mini should show purge key {key:?}");
        assert!(row(&u1, key).hidden, "U1 (toolchanger) should hide purge key {key:?}");
        assert_eq!(
            row(&u1, key).summary.capability,
            Some(CapabilityPredicate::RequiresPurgeTower),
        );
    }
}

/// Render-budget gate (PR-4-1 / PR-4-4): the printer-aware option
/// list must complete fast enough that the panel's 50 ms re-render
/// budget can absorb it. After PR-S-2 the panel surfaces Process-bucket
/// options only (~345 keys vs ~624 before); the budget is unchanged.
#[test]
fn slicer_options_for_printer_meets_render_budget_in_debug() {
    ensure_ffi();
    let start = std::time::Instant::now();
    let opts = slicer_options_for_printer(a1_mini(), None);
    let elapsed = start.elapsed();
    assert!(
        opts.len() >= 300,
        "expected ≥ 300 process-bucket options, got {}",
        opts.len(),
    );
    assert!(
        elapsed < std::time::Duration::from_millis(500),
        "slicer_options_for_printer took {elapsed:?}; debug-budget 500 ms",
    );
}
