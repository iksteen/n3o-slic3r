//! Integration tests for the slic3r-ffi public API.
//!
//! Smoke coverage for init, option introspection, Config set/get/validate,
//! and Model loading. Slicing itself is exercised by examples/slice.rs and
//! isn't repeated here — it's slow and the failure modes are easier to
//! debug as an interactive example than as a test panic.
//!
//! First run is slow because `cargo test` triggers the crate's build.rs
//! and cmake builds libslic3r and the shim. Subsequent runs are fast.

use slic3r_ffi::{init, option_def, option_defs, version, Config, ErrorKind, Model, OptScope, OptType};
use std::path::PathBuf;
use std::sync::Once;

// libslic3r's init has an internal Once guard, but we still gate via our
// own Once to avoid the (cheap) overhead of repeated mutex acquisition
// when tests run in parallel.
static TEST_INIT: Once = Once::new();
fn ensure_init() {
    TEST_INIT.call_once(|| {
        init(None, 3).expect("slic3r_init failed");
    });
}

fn test_stl() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../external/OrcaSlicer/tests/data/test_stl/ASCII/20mmbox-LF.stl");
    assert!(
        p.exists(),
        "test fixture {} missing — run `git submodule update --init --recursive`",
        p.display()
    );
    p
}

#[test]
fn version_is_nonempty() {
    ensure_init();
    let v = version();
    assert!(!v.is_empty(), "version() returned empty string");
    assert!(v.contains("slic3r"), "unexpected version banner: {v:?}");
}

#[test]
fn init_is_idempotent() {
    ensure_init();
    // Second call should be a no-op (Once-guarded in the shim).
    init(None, 3).expect("re-init should succeed");
}

#[test]
fn option_defs_populated() {
    ensure_init();
    let defs = option_defs();
    // Today this is 737; allow upstream growth/shrinkage without being
    // brittle to single-option churn.
    assert!(defs.len() > 100, "implausibly few options: {}", defs.len());
    assert!(
        defs.iter().any(|d| d.key == "layer_height"),
        "layer_height missing from option set"
    );
}

#[test]
fn option_def_layer_height_shape() {
    ensure_init();
    let d = option_def("layer_height").expect("layer_height should exist");
    assert_eq!(d.ty, OptType::Float);
    assert_eq!(d.sidetext.as_deref(), Some("mm"));
    assert_eq!(d.default_serialized.as_deref(), Some("0.2"));
    assert!(d.label.is_some(), "label should be populated");
    assert!(d.category.is_some(), "category should be populated");
}

#[test]
fn option_def_unknown_returns_unknown_key() {
    ensure_init();
    let err = option_def("definitely_not_a_real_setting_12345").unwrap_err();
    assert_eq!(err.kind, ErrorKind::UnknownKey);
}

#[test]
fn option_scope_classifies_known_keys() {
    ensure_init();
    // layer_height is declared in PrintObjectConfig (FFF per-object) AND in
    // SLAPrintObjectConfig — bitmask should reflect both.
    let lh = option_def("layer_height").expect("layer_height");
    assert!(lh.scope.is_object(),
        "layer_height should be OBJECT-scoped, got {:?}", lh.scope);

    // wall_filament is in PrintRegionConfig.
    let wf = option_def("wall_filament").expect("wall_filament");
    assert!(wf.scope.is_region(),
        "wall_filament should be REGION-scoped, got {:?}", wf.scope);
    assert!(!wf.scope.is_object(),
        "wall_filament should NOT be OBJECT-scoped, got {:?}", wf.scope);

    // gcode_flavor is in GCodeConfig, which is a parent of PrintConfig.
    let gf = option_def("gcode_flavor").expect("gcode_flavor");
    assert!(gf.scope.is_print(),
        "gcode_flavor should be PRINT-scoped, got {:?}", gf.scope);
    assert!(!gf.scope.is_object(),
        "gcode_flavor should NOT be OBJECT-scoped, got {:?}", gf.scope);
    assert!(!gf.scope.is_region(),
        "gcode_flavor should NOT be REGION-scoped, got {:?}", gf.scope);

    // SLA-only example.
    let exp = option_def("exposure_time").expect("exposure_time");
    assert!(exp.scope.is_sla_material(),
        "exposure_time should be SLA_MATERIAL-scoped, got {:?}", exp.scope);
    assert!(exp.scope.is_sla(),
        "exposure_time should report is_sla()");
    assert!(!exp.scope.is_fff(),
        "exposure_time should NOT report is_fff()");
}

#[test]
fn most_options_have_a_scope() {
    // Some options are present in print_config_def for preset-bundle /
    // host-integration / UI metadata reasons but aren't declared by any
    // of the static config classes used by slicing (`compatible_printers`,
    // `bbl_use_printhost`, etc.). Those legitimately report scope == 0.
    //
    // For real slicing settings, scope should be non-zero. A jump in the
    // unscoped count suggests either upstream added a new static class we
    // haven't wired up, or moved real options out of the static classes.
    ensure_init();
    let defs = option_defs();
    let scoped = defs.iter().filter(|d| d.scope.0 != 0).count();
    let unscoped = defs.len() - scoped;
    // Today: ~666 scoped / ~71 unscoped out of ~737. Bound generously so
    // option churn doesn't flake the test, but tight enough to catch a
    // missing class wiring (which would push the unscoped count into the
    // hundreds).
    assert!(
        scoped > 500,
        "only {scoped}/{} options have a scope — did a static class wiring break?",
        defs.len()
    );
    assert!(
        unscoped < 150,
        "{unscoped} options have no scope (max 150 expected for preset/metadata) — did a class get unwired?"
    );
}

#[test]
fn coenums_have_default_values() {
    // libslic3r's standard serializer crashes on coEnums defaults (null
    // keys_map on the cloned default value). The shim works around it via
    // a reverse-lookup using the def's enum_keys_map. Every coEnums option
    // should have a non-empty default that resolves to one of its enum
    // values.
    ensure_init();
    let enums: Vec<_> = option_defs()
        .into_iter()
        .filter(|d| d.ty == OptType::Enums)
        .collect();
    assert!(!enums.is_empty(), "no coEnums options registered");
    for d in &enums {
        let default = d.default_serialized.as_deref().unwrap_or("");
        assert!(
            !default.is_empty(),
            "{} (coEnums) has no default_serialized",
            d.key
        );
        // The default should be a comma-separated list of values that
        // all appear in enum_values (the def's curated set).
        for part in default.split(',') {
            assert!(
                d.enum_values.iter().any(|v| v == part),
                "{}: default {:?} contains {:?} which is not in enum_values {:?}",
                d.key, default, part, d.enum_values,
            );
        }
    }
}

#[test]
fn config_set_get_roundtrip() {
    ensure_init();
    let mut cfg = Config::new().expect("Config::new");
    cfg.set("layer_height", "0.3").expect("set");
    assert_eq!(cfg.get("layer_height").expect("get"), "0.3");
}

#[test]
fn config_set_unknown_key() {
    ensure_init();
    let mut cfg = Config::new().expect("Config::new");
    let err = cfg.set("definitely_not_a_real_setting_12345", "1").unwrap_err();
    assert_eq!(err.kind, ErrorKind::UnknownKey);
}

#[test]
fn config_set_parse_failure() {
    ensure_init();
    let mut cfg = Config::new().expect("Config::new");
    let err = cfg.set("layer_height", "not-a-number").unwrap_err();
    assert_eq!(err.kind, ErrorKind::ParseValue);
}

#[test]
fn config_validate_runs_cleanly() {
    ensure_init();
    let cfg = Config::new().expect("Config::new");
    // Default FullPrintConfig isn't guaranteed to pass libslic3r's
    // cross-option validator (some defaults conflict with each other for
    // headless invocation). Either Ok or Validate is acceptable — we just
    // assert the call doesn't panic and returns a known error kind.
    match cfg.validate() {
        Ok(()) => {}
        Err(e) => assert_eq!(e.kind, ErrorKind::Validate, "unexpected error: {e}"),
    }
}

#[test]
fn model_load_missing_file_is_io_error() {
    ensure_init();
    let mut m = Model::new().expect("Model::new");
    let err = m.load("/nonexistent/no_such_dir/no_such_file.stl").unwrap_err();
    assert_eq!(err.kind, ErrorKind::Io);
}

#[test]
fn model_load_test_stl() {
    ensure_init();
    let mut m = Model::new().expect("Model::new");
    m.load(test_stl()).expect("load 20mmbox-LF.stl");
}

#[test]
fn load_with_config_stl_keeps_defaults() {
    ensure_init();
    // STL files carry no embedded config. load_with_config should leave
    // the caller's config untouched, per the API contract.
    let mut cfg = Config::new().expect("Config::new");
    let before = cfg.get("layer_height").expect("get before");
    let mut m = Model::new().expect("Model::new");
    m.load_with_config(test_stl(), &mut cfg).expect("load_with_config");
    let after = cfg.get("layer_height").expect("get after");
    assert_eq!(before, after, "STL load should not modify config");
}
