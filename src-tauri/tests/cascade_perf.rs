#![cfg(feature = "test-fixtures")]
//! Cascade pipeline perf gates.
//!
//! Drives the resolver + adapter in tight loops and asserts mean
//! latency stays under FR-CAS-11's budgets:
//!
//! - Full 4-slot resolution < 10 ms
//! - Resolution + adapter expansion < 100 ms
//!
//! Implemented as plain `#[test]`s rather than a `criterion` bench
//! harness — keeps the regression gate inside the normal
//! `cargo test --release` invocation that CI already runs, avoids a
//! heavy dev dependency, and lets us pick the runtime budget per
//! assertion. Mean over N=100 iterations is enough signal for a
//! regression gate; statistical-quality bench numbers can come back
//! later if Phase 4's UI shows the resolver hot path.

use n3o_slic3r_lib::core::cascade::{resolve_with_overrides, Cascade, OverrideTiers};
use n3o_slic3r_lib::core::cascade_adapter::{adapt_with_overrides, Manifest};
use n3o_slic3r_lib::core::filament::FilamentProfile;
use n3o_slic3r_lib::core::printer::lookup_instance;
use n3o_slic3r_lib::core::printer::profile::{BoundingBox, PrinterProfile, Toolhead};
use n3o_slic3r_lib::core::profile_library::compose_cascade;
use n3o_slic3r_lib::core::project::SlicingContext;
use n3o_slic3r_lib::core::scene::build_plate::BuildPlate;
use slic3r_ffi::init;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Once;
use std::time::{Duration, Instant};

static FFI_INIT: Once = Once::new();
fn ensure_ffi() {
    FFI_INIT.call_once(|| {
        init(None, 3).expect("libslic3r init");
    });
}

const ITERATIONS: u32 = 100;

/// Run `op` N times and return (mean, max) latency.
fn measure<F: FnMut()>(mut op: F) -> (Duration, Duration) {
    // Warm-up to amortize first-iteration costs (allocator, branch
    // predictor, etc.). 10% of measurement count, minimum 5.
    for _ in 0..ITERATIONS.max(50) / 10 {
        op();
    }
    let mut total = Duration::ZERO;
    let mut peak = Duration::ZERO;
    for _ in 0..ITERATIONS {
        let start = Instant::now();
        op();
        let elapsed = start.elapsed();
        total += elapsed;
        if elapsed > peak {
            peak = elapsed;
        }
    }
    (total / ITERATIONS, peak)
}

fn load_reference_cascade() -> Cascade {
    // PR-S-5c: use the composer-derived cascade for the Bambi instance
    // instead of the (now-deleted) monolithic profiles/cascades/...toml.
    // Shape is identical from the resolver's perspective; just sourced
    // from the per-bucket vendor fragments + composer.
    let bambi = lookup_instance("bambi").expect("bambi bundled");
    compose_cascade(&bambi, &[], &BTreeMap::new()).expect("compose bambi cascade")
}

fn a1_mini_pla_pei_context() -> SlicingContext {
    SlicingContext::new(
        Arc::new(PrinterProfile {
            model: "Bambu Lab A1 mini".into(),
            supported_build_plates: vec!["Textured PEI".into()],
            toolheads: vec![Toolhead {
                default_nozzle_diameter: "0.4".into(),
                hotend_type: "stainless_steel".into(),
                max_temp: 300.0,
            }],
            build_volume: BoundingBox::default(),
            exclusion_zones: vec![],
            ..Default::default()
        }),
        Arc::new(BuildPlate {
            identity: "Textured PEI".into(),
            libslic3r_curr_bed_type: "Textured PEI Plate".into(),
        }),
        vec![Arc::new(FilamentProfile {
            identity: "Generic PLA".into(),
            base_type: "PLA".into(),
            vendor: None,
            color: None,
        })],
    )
}

/// Synthesized 4-slot context (no real U1 profile yet — PR-1-8 cut).
/// Used to exercise the resolver under the canonical multi-slot case.
fn four_slot_context() -> SlicingContext {
    SlicingContext::new(
        Arc::new(PrinterProfile {
            model: "Snapmaker U1".into(),
            supported_build_plates: vec!["Textured PEI".into()],
            toolheads: (0..4)
                .map(|i| Toolhead {
                    default_nozzle_diameter: if i % 2 == 0 {
                        "0.4".into()
                    } else {
                        "0.6".into()
                    },
                    hotend_type: "hardened_steel".into(),
                    max_temp: 300.0,
                })
                .collect(),
            build_volume: BoundingBox::default(),
            exclusion_zones: vec![],
            ..Default::default()
        }),
        Arc::new(BuildPlate {
            identity: "Textured PEI".into(),
            libslic3r_curr_bed_type: "Textured PEI Plate".into(),
        }),
        vec![
            Arc::new(FilamentProfile {
                identity: "PLA Slot 0".into(),
                base_type: "PLA".into(),
                vendor: None,
                color: None,
            }),
            Arc::new(FilamentProfile {
                identity: "PETG Slot 1".into(),
                base_type: "PETG".into(),
                vendor: None,
                color: None,
            }),
            Arc::new(FilamentProfile {
                identity: "ABS Slot 2".into(),
                base_type: "ABS".into(),
                vendor: None,
                color: None,
            }),
            Arc::new(FilamentProfile {
                identity: "PLA Slot 3".into(),
                base_type: "PLA".into(),
                vendor: None,
                color: None,
            }),
        ],
    )
}

#[test]
fn perf_resolve_a1_mini_pla_pei() {
    // Budget: < 10 ms per FR-CAS-11. Today's small reference cascade
    // resolves well under 1 ms — the budget is a regression gate.
    let budget = Duration::from_millis(10);

    let cascade = load_reference_cascade();
    let ctx = a1_mini_pla_pei_context();
    let overrides = OverrideTiers::empty();

    let (mean, peak) = measure(|| {
        let _ = resolve_with_overrides(&cascade, &overrides, &ctx);
    });

    eprintln!("perf_resolve_a1_mini_pla_pei: mean={mean:?} peak={peak:?}");
    assert!(
        mean < budget,
        "resolve mean {mean:?} exceeds budget {budget:?}"
    );
}

#[test]
fn perf_resolve_four_slot_synthetic() {
    // Budget: < 15 ms. Same cascade resolved against each of the 4
    // slots in sequence — exercises per-slot resolve overhead.
    let budget = Duration::from_millis(15);

    let cascade = load_reference_cascade();
    let mut ctx = four_slot_context();
    let overrides = OverrideTiers::empty();

    let (mean, peak) = measure(|| {
        for slot in 0..ctx.printer.toolheads.len() {
            ctx.active_slot = slot;
            let _ = resolve_with_overrides(&cascade, &overrides, &ctx);
        }
    });

    eprintln!("perf_resolve_four_slot_synthetic: mean={mean:?} peak={peak:?}");
    assert!(
        mean < budget,
        "4-slot resolve mean {mean:?} exceeds budget {budget:?}"
    );
}

#[test]
fn perf_resolve_and_adapt_a1_mini_pla_pei() {
    // Budget: < 100 ms per FR-CAS-11. Includes Config::set across
    // every resolved key plus the bed_temp expansion to 12 plate-temp
    // keys.
    ensure_ffi();
    let budget = Duration::from_millis(100);

    let cascade = load_reference_cascade();
    let ctx = a1_mini_pla_pei_context();
    let overrides = OverrideTiers::empty();
    let manifest = Manifest::build();

    let (mean, peak) = measure(|| {
        let resolved = resolve_with_overrides(&cascade, &overrides, &ctx);
        let _ = adapt_with_overrides(&resolved, &ctx, &manifest).expect("adapt");
    });

    eprintln!("perf_resolve_and_adapt_a1_mini_pla_pei: mean={mean:?} peak={peak:?}");
    assert!(
        mean < budget,
        "resolve+adapt mean {mean:?} exceeds budget {budget:?}"
    );
}
