//! Integration tests for the slic3r-ffi public API.
//!
//! Smoke coverage for init, option introspection, Config set/get/validate,
//! and Model loading. Slicing itself is exercised by examples/slice.rs and
//! isn't repeated here — it's slow and the failure modes are easier to
//! debug as an interactive example than as a test panic.
//!
//! First run is slow because `cargo test` triggers the crate's build.rs
//! and cmake builds libslic3r and the shim. Subsequent runs are fast.

use slic3r_ffi::{init, option_def, option_defs, version, Config, ErrorKind, Model, OptType};
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
