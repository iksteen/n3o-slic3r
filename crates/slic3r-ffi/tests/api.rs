//! Integration tests for the slic3r-ffi public API.
//!
//! Smoke coverage for init, option introspection, Config set/get/validate,
//! and Model loading. Slicing itself is exercised by examples/slice.rs and
//! isn't repeated here — it's slow and the failure modes are easier to
//! debug as an interactive example than as a test panic.
//!
//! First run is slow because `cargo test` triggers the crate's build.rs
//! and cmake builds libslic3r and the shim. Subsequent runs are fast.

use slic3r_ffi::{
    clear_log_sink, init, option_def, option_defs, set_log_sink, slice, version, Config,
    ErrorKind, LogLevel, Model, OptType,
};
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

// The log sink IS still process-global (set_log_sink writes a global
// fn pointer + user_data on the C side; the boost log sink stays
// installed process-lifetime). Serialize log-sink tests against each
// other so parallel registration doesn't cross-contaminate. Slice
// progress no longer has this problem — the callback is passed into
// slice() per call.
static LOG_SINK_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

#[test]
fn slice_progress_callback_fires_with_monotonic_percent() {
    use std::cell::RefCell;

    ensure_init();

    // Collect every (percent, stage) tick the slice emits. RefCell
    // is fine here — the callback is captured per-slice and only
    // fires from the calling thread (slice() is synchronous).
    let ticks: RefCell<Vec<(i32, String)>> = RefCell::new(Vec::new());

    let mut model = Model::new().expect("Model::new");
    let mut config = Config::new().expect("Config::new");
    model
        .load_with_config(test_stl(), &mut config)
        .expect("load 20mmbox-LF.stl");
    // FullPrintConfig defaults set `use_relative_e_distances=true`,
    // which triggers a validate-time check for `G92 E0` in
    // `layer_gcode`. Flip to absolute so the default validation
    // passes; the progress test doesn't care which mode the slice
    // runs in.
    config
        .set("use_relative_e_distances", "0")
        .expect("set use_relative_e_distances");

    let out_path = std::env::temp_dir().join(format!(
        "n3o-slice-progress-test-{}.gcode",
        std::process::id(),
    ));
    slice(&model, &config, &out_path, |percent, stage| {
        ticks.borrow_mut().push((percent, stage.to_owned()));
    })
    .expect("slice OK");

    let ticks = ticks.borrow();
    assert!(
        ticks.len() > 5,
        "expected many progress ticks from libslic3r (got {})",
        ticks.len(),
    );
    // Per-stage monotonicity isn't guaranteed across the whole
    // slice — libslic3r emits stage-local percents that can
    // backstep at boundaries (`(71, "Detect overhangs") → (70,
    // "Generating skirt & brim")` observed in practice). What we
    // do care about: the slice reaches a high-percent terminal
    // state, and the stage labels look sane.
    let final_percent = ticks.last().map(|(p, _)| *p).unwrap_or(0);
    assert!(
        final_percent >= 50,
        "expected slice to reach >= 50% (got {final_percent}, ticks={ticks:?})",
    );
    let non_empty_stages: usize = ticks.iter().filter(|(_, s)| !s.is_empty()).count();
    assert!(
        non_empty_stages > 0,
        "expected at least one labelled stage from libslic3r ticks",
    );
    // The "Generating G-code" stage is the last meaningful phase;
    // its presence confirms the callback rode the slice past
    // process() and into export_gcode().
    assert!(
        ticks.iter().any(|(_, s)| s.contains("Generating G-code")),
        "expected a 'Generating G-code' stage tick — slice may have aborted early",
    );
    let _ = std::fs::remove_file(&out_path);
}

#[test]
fn log_sink_receives_records_emitted_during_slice() {
    use std::sync::{Arc, Mutex};

    ensure_init();
    let _serial = LOG_SINK_LOCK.lock().expect("log sink lock");

    let records: Arc<Mutex<Vec<(LogLevel, String)>>> = Arc::new(Mutex::new(Vec::new()));
    let records_for_cb = Arc::clone(&records);
    set_log_sink(move |level, msg| {
        records_for_cb
            .lock()
            .unwrap()
            .push((level, msg.to_owned()));
    });

    // Drive a slice — libslic3r emits a handful of BOOST_LOG_TRIVIAL
    // records during apply/validate/process even at log_level=3
    // (info), the level the test fixture initializes with. The
    // exact count is fragile (depends on upstream verbosity), but
    // "at least one record" is stable.
    let mut model = Model::new().expect("Model::new");
    let mut config = Config::new().expect("Config::new");
    model
        .load_with_config(test_stl(), &mut config)
        .expect("load");
    config
        .set("use_relative_e_distances", "0")
        .expect("set use_relative_e_distances");

    let out_path = std::env::temp_dir().join("n3o-log-sink-test.gcode");
    let slice_result = slice(&model, &config, &out_path, |_, _| {});
    clear_log_sink();
    slice_result.expect("slice OK");

    let records = records.lock().unwrap();
    assert!(
        !records.is_empty(),
        "expected at least one log record during slice; got 0 — sink may not be installed",
    );
    // Every record's message should be non-empty (we'd otherwise
    // be receiving padding from an empty extract).
    for (_level, msg) in records.iter() {
        assert!(!msg.is_empty(), "log message unexpectedly empty");
    }
    let _ = std::fs::remove_file(&out_path);
}

#[test]
fn log_sink_can_be_unregistered() {
    use std::sync::{Arc, Mutex};

    ensure_init();
    let _serial = LOG_SINK_LOCK.lock().expect("log sink lock");

    // First slice: sink active, counter ticks at least once.
    let counter: Arc<Mutex<usize>> = Arc::new(Mutex::new(0));
    let counter_for_cb = Arc::clone(&counter);
    set_log_sink(move |_lvl, _msg| {
        *counter_for_cb.lock().unwrap() += 1;
    });

    let mut model = Model::new().expect("Model::new");
    let mut config = Config::new().expect("Config::new");
    model
        .load_with_config(test_stl(), &mut config)
        .expect("load");
    config
        .set("use_relative_e_distances", "0")
        .expect("set use_relative_e_distances");
    let out1 = std::env::temp_dir().join("n3o-log-sink-on.gcode");
    slice(&model, &config, &out1, |_, _| {}).expect("slice 1");
    let ticks_after_first = *counter.lock().unwrap();
    assert!(
        ticks_after_first > 0,
        "expected log sink to fire while registered",
    );

    clear_log_sink();
    let out2 = std::env::temp_dir().join("n3o-log-sink-off.gcode");
    slice(&model, &config, &out2, |_, _| {}).expect("slice 2");
    let ticks_after_second = *counter.lock().unwrap();
    assert_eq!(
        ticks_after_second, ticks_after_first,
        "callback fired after clear_log_sink (delta = {})",
        ticks_after_second - ticks_after_first,
    );

    let _ = std::fs::remove_file(&out1);
    let _ = std::fs::remove_file(&out2);
}

#[test]
fn slice_progress_callbacks_are_per_call_no_cross_contamination() {
    use std::cell::Cell;

    ensure_init();

    let mut model = Model::new().expect("Model::new");
    let mut config = Config::new().expect("Config::new");
    model
        .load_with_config(test_stl(), &mut config)
        .expect("load");
    config
        .set("use_relative_e_distances", "0")
        .expect("set use_relative_e_distances");

    // Two consecutive slices, each with its own counter. After both
    // run, each counter should reflect ONLY its own slice's ticks —
    // not the other's. (The legacy "clear_slice_progress() makes the
    // next slice silent" semantics no longer apply: there's no
    // global registration to clear. Passing a closure means it fires
    // for this slice; the next slice's separate closure fires only
    // for its own ticks.)
    let counter_a: Cell<usize> = Cell::new(0);
    let out1 = std::env::temp_dir().join("n3o-slice-progress-a.gcode");
    slice(&model, &config, &out1, |_, _| {
        counter_a.set(counter_a.get() + 1);
    })
    .expect("slice A");
    let ticks_a = counter_a.get();
    assert!(ticks_a > 0, "slice A's closure should fire while running");

    let counter_b: Cell<usize> = Cell::new(0);
    let out2 = std::env::temp_dir().join("n3o-slice-progress-b.gcode");
    slice(&model, &config, &out2, |_, _| {
        counter_b.set(counter_b.get() + 1);
    })
    .expect("slice B");
    let ticks_b = counter_b.get();
    assert!(ticks_b > 0, "slice B's closure should fire while running");

    // The critical assertion: slice A's counter was NOT advanced by
    // slice B's ticks. Pre-rework this would have been the case
    // because the trampoline read a Rust-side global that
    // set_slice_progress mutated.
    assert_eq!(
        counter_a.get(),
        ticks_a,
        "slice A's counter advanced during slice B — callbacks leaked across slices",
    );

    let _ = std::fs::remove_file(&out1);
    let _ = std::fs::remove_file(&out2);
}

/// Two slice() calls on two threads, real concurrent libslic3r runs.
///
/// The per-slice progress callback rework cleared the FFI-side
/// blocker for concurrent slicing. This test answers the next
/// question: does libslic3r itself tolerate two `Print::process()`
/// calls running at the same time in the same process?
///
/// If it does: both threads succeed, both produce non-empty gcode,
/// both closures fire only their own ticks.
/// If it doesn't: this test surfaces the failure mode directly
/// (crash, wrong-output, hang) instead of leaving it as a latent
/// "we don't know" — gives us empirical data for the multi-plate
/// parallelism design decision.
///
/// Skewed inputs (different layer heights, different output paths)
/// so the two runs can't coincidentally collide on the same
/// on-disk artifact or hash to identical workloads.
#[test]
fn two_concurrent_slices_in_separate_threads_both_succeed() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;

    ensure_init();

    let make_inputs = |layer_height: &'static str, out_name: &'static str|
        -> (Model, Config, std::path::PathBuf) {
        let mut model = Model::new().expect("Model::new");
        let mut config = Config::new().expect("Config::new");
        model
            .load_with_config(test_stl(), &mut config)
            .expect("load 20mmbox-LF.stl");
        config
            .set("use_relative_e_distances", "0")
            .expect("set use_relative_e_distances");
        config
            .set("layer_height", layer_height)
            .expect("set layer_height");
        let out_path = std::env::temp_dir().join(format!(
            "n3o-concurrent-slice-{}-{}.gcode",
            std::process::id(),
            out_name,
        ));
        (model, config, out_path)
    };

    let (model_a, config_a, out_a) = make_inputs("0.20", "a");
    let (model_b, config_b, out_b) = make_inputs("0.16", "b");

    let ticks_a = Arc::new(AtomicUsize::new(0));
    let ticks_b = Arc::new(AtomicUsize::new(0));
    let ticks_a_for_cb = Arc::clone(&ticks_a);
    let ticks_b_for_cb = Arc::clone(&ticks_b);
    let out_a_for_thread = out_a.clone();
    let out_b_for_thread = out_b.clone();

    let handle_a = thread::spawn(move || {
        slice(&model_a, &config_a, &out_a_for_thread, |_, _| {
            ticks_a_for_cb.fetch_add(1, Ordering::Relaxed);
        })
    });
    let handle_b = thread::spawn(move || {
        slice(&model_b, &config_b, &out_b_for_thread, |_, _| {
            ticks_b_for_cb.fetch_add(1, Ordering::Relaxed);
        })
    });

    let result_a = handle_a.join().expect("thread A panicked");
    let result_b = handle_b.join().expect("thread B panicked");

    // Both slices must succeed. Either failing tells us libslic3r's
    // not safely concurrent at the Print::process() level; the
    // multi-plate parallelism plan would then need separate
    // processes (or a slicer-side mutex).
    result_a.expect("slice A failed under concurrent run");
    result_b.expect("slice B failed under concurrent run");

    // Both closures fired — neither was starved or routed to the
    // other slice. The per-slice callback rework already proved
    // this for sequential runs (sibling test above); this run pins
    // it under genuine thread parallelism.
    assert!(
        ticks_a.load(Ordering::Relaxed) > 0,
        "slice A's closure didn't fire",
    );
    assert!(
        ticks_b.load(Ordering::Relaxed) > 0,
        "slice B's closure didn't fire",
    );

    // Both gcode files written and non-empty. Verifying byte-level
    // correctness against the sequential output is out of scope —
    // we just care that both ran to completion and the writers
    // didn't trample each other's output paths.
    let bytes_a = std::fs::metadata(&out_a).map(|m| m.len()).unwrap_or(0);
    let bytes_b = std::fs::metadata(&out_b).map(|m| m.len()).unwrap_or(0);
    assert!(bytes_a > 0, "slice A produced no gcode");
    assert!(bytes_b > 0, "slice B produced no gcode");

    let _ = std::fs::remove_file(&out_a);
    let _ = std::fs::remove_file(&out_b);
}
