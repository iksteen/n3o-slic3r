//! Integration test for the Phase 1 reference profiles.
//!
//! Loads `profiles/printers/bambu-a1-mini.toml`,
//! `profiles/plates/textured-pei.toml`, `profiles/filaments/
//! generic-pla.toml`, parses `profiles/cascades/bambu-a1-mini-demo.toml`
//! (the small spec-0/1/2 fixture), resolves PLA-on-Textured-PEI, and
//! asserts the expected effective values. End-to-end test of PR-1-1
//! through PR-1-7 against an auditable cascade surface — production
//! safety properties of the BBS-derived `*-default.toml` live in
//! `bbs_production_cascade.rs` instead.

use n3o_slic3r_lib::core::cascade::{
    loader::parse_cascade_str, resolve, validate_cascade, Cascade, KnownDimensions,
};
use n3o_slic3r_lib::core::filament::FilamentProfile;
use n3o_slic3r_lib::core::printer::PrinterProfile;
use n3o_slic3r_lib::core::project::SlicingContext;
use n3o_slic3r_lib::core::scene::BuildPlate;
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

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

fn load_toml<T: serde::de::DeserializeOwned>(relative: &str) -> T {
    let path = workspace_root().join(relative);
    let bytes = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!("read {}: {e}", path.display());
    });
    toml::from_str(&bytes).unwrap_or_else(|e| {
        panic!("parse {}: {e}", path.display());
    })
}

#[test]
fn reference_profiles_resolve_canonical_pla_pei_context() {
    ensure_ffi();

    let printer: PrinterProfile = load_toml("profiles/printers/bambu-a1-mini.toml");
    let plate: BuildPlate = load_toml("profiles/plates/textured-pei.toml");
    let filament: FilamentProfile = load_toml("profiles/filaments/generic-pla.toml");

    assert_eq!(printer.model, "Bambu A1 mini");
    assert_eq!(plate.identity, "Textured PEI");
    assert_eq!(filament.base_type, "PLA");

    let ctx = SlicingContext::new(Arc::new(printer), Arc::new(plate), vec![Arc::new(filament)]);

    let cascade_path = workspace_root().join("profiles/cascades/bambu-a1-mini-demo.toml");
    let src = std::fs::read_to_string(&cascade_path).expect("read cascade");
    let cascade = Cascade {
        rules: parse_cascade_str(&src, Path::new("bambu-a1-mini-demo.toml"))
            .expect("parse cascade"),
    };

    // Schema validation must pass — typos here block CI.
    validate_cascade(&cascade, &KnownDimensions::new(
        ["printer.model", "filament.type", "filament.name", "plate.type"],
    ))
    .expect("reference cascade validates against the schema");

    let resolved = resolve(&cascade, &ctx);

    // Default rule (specificity 0) sets layer_height = 0.2.
    assert_eq!(
        resolved.get("layer_height").map(|v| v.value.as_str()),
        Some("0.2"),
        "layer_height from default rule"
    );

    // Filament rule (specificity 1) sets nozzle_temperature = 220 for PLA.
    assert_eq!(
        resolved.get("nozzle_temperature").map(|v| v.value.as_str()),
        Some("220"),
        "nozzle_temperature from filament rule"
    );

    // Plate × filament rule (specificity 2) wins for bed_temp.
    let bed_temp = resolved.get("bed_temp").expect("bed_temp resolved");
    assert_eq!(bed_temp.value, "65");
    assert_eq!(bed_temp.winning_specificity, 2);

    // matching_rules includes the spec-1 plate-only rule as a loser.
    let specs: Vec<usize> = bed_temp.matching_rules.iter().map(|m| m.specificity).collect();
    assert!(specs.contains(&1), "spec-1 plate rule is a loser: {specs:?}");
    assert!(specs.contains(&2), "spec-2 winner is in matching_rules: {specs:?}");
}
