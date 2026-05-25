//! Integration test for the Phase 1 reference profiles.
//!
//! Resolves the small spec-0/1/2 demo cascade (tests/fixtures/) against
//! a `SlicingContext` built from the registry-loaded A1 mini printer +
//! Textured PEI Plate + Generic PLA filament. Exercises the cascade
//! resolver end-to-end against an auditable surface; the BBS-derived
//! production cascade is exercised separately in
//! `bbs_production_cascade.rs`.
//!
//! The fixture lives next to this test (test-only); production code
//! reads vendor profiles from `profiles/vendor/` via the
//! `core::profile_library` disk loader.
//!
//! NB: The demo cascade hits identities/strings only; surface_kind
//! semantics are exercised through other paths.

use n3o_slic3r_lib::core::cascade::{
    loader::parse_cascade_str, resolve, validate_cascade, Cascade, KnownDimensions,
};
use n3o_slic3r_lib::core::filament;
use n3o_slic3r_lib::core::printer::registry as printer_registry;
use n3o_slic3r_lib::core::project::SlicingContext;
use n3o_slic3r_lib::core::scene::build_plate;
use slic3r_ffi::init;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Once;

static FFI_INIT: Once = Once::new();
fn ensure_ffi() {
    FFI_INIT.call_once(|| {
        init(None, 3).expect("libslic3r init");
    });
}

#[test]
fn reference_profiles_resolve_canonical_pla_pei_context() {
    ensure_ffi();

    let printer = printer_registry::lookup("bambu-lab-a1-mini")
        .expect("bambu-lab-a1-mini in registry");
    let plate = build_plate::lookup("Textured PEI Plate")
        .expect("Textured PEI Plate in registry");
    let filament = filament::registry::lookup("generic-pla")
        .expect("generic-pla in registry");

    assert_eq!(printer.model, "Bambu A1 mini");
    assert_eq!(plate.identity, "Textured PEI Plate");
    assert_eq!(filament.base_type, "PLA");

    let ctx = SlicingContext::new(Arc::new(printer), Arc::new(plate), vec![Arc::new(filament)]);

    let cascade_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/bambu-lab-a1-mini-demo.toml");
    let src = std::fs::read_to_string(&cascade_path).expect("read cascade");
    let cascade = Cascade {
        rules: parse_cascade_str(&src, Path::new("bambu-lab-a1-mini-demo.toml"))
            .expect("parse cascade"),
    };

    validate_cascade(&cascade, &KnownDimensions::new(
        ["printer.model", "filament.type", "filament.name", "plate.type"],
    ))
    .expect("reference cascade validates against the schema");

    let resolved = resolve(&cascade, &ctx);

    assert_eq!(
        resolved.get("layer_height").map(|v| v.value.as_str()),
        Some("0.2"),
        "layer_height from default rule",
    );

    assert_eq!(
        resolved.get("nozzle_temperature").map(|v| v.value.as_str()),
        Some("220"),
        "nozzle_temperature from filament rule",
    );

    let bed_temp = resolved.get("bed_temp").expect("bed_temp resolved");
    assert_eq!(bed_temp.value, "65");
    assert_eq!(bed_temp.winning_specificity, 2);

    let specs: Vec<usize> = bed_temp.matching_rules.iter().map(|m| m.specificity).collect();
    assert!(specs.contains(&1), "spec-1 plate rule is a loser: {specs:?}");
    assert!(specs.contains(&2), "spec-2 winner is in matching_rules: {specs:?}");
}
