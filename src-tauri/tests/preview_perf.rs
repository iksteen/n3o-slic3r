//! Preview pipeline perf gates (PR-6-16).
//!
//! FR-GP-9's contract: 50 MB G-code parsed + IR built + stats
//! computed in < 5 s end-to-end on the dev rig, in release mode.
//! We follow the same pattern as `gcode_parser_perf.rs`: assert
//! against a 5 MB synthetic fixture (CI-friendly), with loose
//! debug-mode budgets that catch O(n²) regressions while leaving
//! headroom for slow CI runners. A 50 MB variant is gated behind
//! `#[ignore]` for nightly / on-demand runs.
//!
//! Budgets (5 MB, debug):
//! - parse + build_preview:                 < 2000 ms
//! - compute_layer_stats + compute_job_stats: <  500 ms
//! - encode_colors (each mode):               <  400 ms
//!
//! Release-mode dev-rig extrapolation: ~5-10× faster → 50 MB
//! end-to-end well under 5 s.
//!
//! Following PR-2-11/PR-3-6's choice to use plain `Instant` +
//! integration tests rather than `criterion`: keeps the workspace
//! dep set narrow + bench setup uniform across phases.

use std::io::Write;
use std::time::Instant;

use n3o_slic3r_lib::core::gcode::parse_str;
use n3o_slic3r_lib::core::preview::{
    build::build_preview,
    colors::{encode_colors, ColorMode, Palette},
    stats::{compute_job_stats, compute_layer_stats},
};

/// Generate a synthetic G-code fixture of approximately
/// `target_bytes` bytes. Same shape as gcode_parser_perf's
/// fixture — layered moves with feature/layer markers — so we
/// exercise the IR's full classification path (extrusion vs
/// travel vs retraction, layer transitions, tool changes).
fn synthetic_gcode(target_bytes: usize) -> String {
    let mut buf = Vec::with_capacity(target_bytes + 1024);
    writeln!(&mut buf, "; estimated printing time = 1h 23m").unwrap();
    writeln!(&mut buf, "; filament used [g] = 12.345").unwrap();
    writeln!(&mut buf, "; total layers count = 247").unwrap();
    writeln!(&mut buf, "; printer_model = A1 mini").unwrap();
    writeln!(&mut buf, "M104 S210").unwrap();
    writeln!(&mut buf, "M140 S60").unwrap();
    writeln!(&mut buf, "G28").unwrap();

    let mut layer = 0u32;
    let mut z = 0.2_f32;
    let features = [
        "Perimeter",
        "External perimeter",
        "Internal infill",
        "Solid infill",
    ];
    let mut x = 100.0_f32;
    let mut y = 100.0_f32;
    let mut e = 0.0_f32;

    while buf.len() < target_bytes {
        writeln!(&mut buf, ";LAYER_CHANGE").unwrap();
        writeln!(&mut buf, ";Z:{z:.3}").unwrap();
        if layer.is_multiple_of(10) {
            writeln!(&mut buf, "T{}", layer % 4).unwrap();
        }
        for (i, feature) in features.iter().enumerate() {
            writeln!(&mut buf, ";TYPE:{feature}").unwrap();
            for j in 0..50 {
                x += 0.5;
                y += if (i + j) % 2 == 0 { 0.1 } else { -0.1 };
                e += 0.02;
                writeln!(
                    &mut buf,
                    "G1 X{x:.3} Y{y:.3} E{e:.4} F{}",
                    1200 + (j as u32 * 5),
                )
                .unwrap();
            }
        }
        layer += 1;
        z += 0.2;
    }
    String::from_utf8(buf).expect("synthetic is ASCII")
}

#[test]
fn build_preview_under_2s_on_5mb_synthetic() {
    let src = synthetic_gcode(5 * 1024 * 1024);
    assert!(src.len() >= 5 * 1024 * 1024);

    // Warm-up — let the allocator settle so the timed run reflects
    // steady-state cost, not first-touch faulting.
    let _ = parse_str(&src);

    let start = Instant::now();
    let lines = parse_str(&src);
    let geom = build_preview(&lines);
    let elapsed = start.elapsed();
    println!(
        "parse+build_preview: {} bytes → {} extrusions in {:?} ({:.1} MB/s)",
        src.len(),
        geom.extrusions.len(),
        elapsed,
        (src.len() as f64 / 1_048_576.0) / elapsed.as_secs_f64(),
    );

    assert!(
        !geom.extrusions.is_empty(),
        "IR build yielded zero extrusions"
    );
    assert!(
        elapsed.as_millis() < 2000,
        "parse + build_preview took {:?} on 5 MB synthetic (debug budget: 2000 ms; \
         release contract: < 1 s on 50 MB per FR-GP-9)",
        elapsed,
    );
}

#[test]
fn stats_under_500ms_on_5mb_synthetic() {
    let src = synthetic_gcode(5 * 1024 * 1024);
    let lines = parse_str(&src);
    let geom = build_preview(&lines);

    let start = Instant::now();
    let layer_stats = compute_layer_stats(&geom);
    let job_stats = compute_job_stats(&geom, &layer_stats);
    let elapsed = start.elapsed();
    println!(
        "compute_layer_stats + compute_job_stats: {} layers in {:?}",
        layer_stats.len(),
        elapsed,
    );

    assert!(!layer_stats.is_empty(), "stats yielded zero layers");
    assert!(job_stats.layer_count > 0);
    assert!(
        elapsed.as_millis() < 500,
        "stats took {:?} on 5 MB synthetic (debug budget: 500 ms)",
        elapsed,
    );
}

#[test]
fn encode_colors_under_400ms_per_mode_on_5mb_synthetic() {
    let src = synthetic_gcode(5 * 1024 * 1024);
    let lines = parse_str(&src);
    let geom = build_preview(&lines);
    let layer_stats = compute_layer_stats(&geom);
    let layer_times: Vec<f32> = layer_stats.iter().map(|s| s.duration_seconds).collect();

    for mode in [
        ColorMode::Feature,
        ColorMode::Speed,
        ColorMode::Flow,
        ColorMode::LayerTime,
        ColorMode::Tool,
    ] {
        let start = Instant::now();
        let colors = encode_colors(&geom.extrusions, mode, Palette::Default, Some(&layer_times));
        let elapsed = start.elapsed();
        println!(
            "encode_colors({:?}): {} floats in {:?}",
            mode,
            colors.len(),
            elapsed,
        );
        assert_eq!(colors.len(), geom.extrusions.len() * 6);
        assert!(
            elapsed.as_millis() < 400,
            "encode_colors({:?}) took {:?} on 5 MB synthetic (debug budget: 400 ms)",
            mode,
            elapsed,
        );
    }
}

/// 50 MB variant — gated behind `#[ignore]` so default `cargo
/// test` stays CI-friendly. Run manually:
///
///   cargo test --test preview_perf --release -- --ignored --nocapture
///
/// Asserts the FR-GP-9 release-mode contract: 50 MB end-to-end in
/// under 5 s. Use release mode — the debug-mode budget would need
/// to be 10× looser, which defeats the point of the gate.
#[test]
#[ignore = "50 MB fixture — run on-demand with --release"]
fn end_to_end_under_5s_on_50mb_synthetic_release() {
    let src = synthetic_gcode(50 * 1024 * 1024);
    let start = Instant::now();
    let lines = parse_str(&src);
    let geom = build_preview(&lines);
    let layer_stats = compute_layer_stats(&geom);
    let job_stats = compute_job_stats(&geom, &layer_stats);
    let elapsed = start.elapsed();
    println!(
        "50 MB end-to-end: {} layers, {} extrusions in {:?}",
        job_stats.layer_count,
        geom.extrusions.len(),
        elapsed,
    );
    assert!(
        elapsed.as_secs_f32() < 5.0,
        "50 MB end-to-end took {:?} (FR-GP-9 contract: < 5 s in release)",
        elapsed,
    );
}
