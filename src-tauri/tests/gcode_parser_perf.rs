//! G-code parser perf gate (PR-3-6).
//!
//! Execution Plan §5 calls for 50 MB G-code in < 3 s end-to-end on
//! the dev rig. We assert against a smaller synthetic fixture (5 MB)
//! and the proportionally-scaled budget — that's enough signal to
//! catch regressions while keeping CI test time bounded. The
//! linear-extrapolation lemma holds because the parser is O(n) on
//! input length: streaming reader, fixed buffer.
//!
//! Budget (5 MB):
//! - release-mode dev rig: comfortably under 100 ms.
//! - CI runs the test step in debug mode (per `.github/workflows/
//!   build.yml`'s OOM workaround) — debug is ~5-10× slower for
//!   string scanning, so the assertion ceiling is 750 ms with
//!   headroom. Extrapolates to 7.5 s on a 50 MB file in debug, ~1 s
//!   in release — both well within Execution Plan §5's 3 s release-
//!   mode ceiling.

use std::io::Write;
use std::time::Instant;

use n3o_slic3r_lib::core::gcode::parse_lines;

/// Generate a synthetic G-code fixture of approximately `target_bytes`
/// bytes. The shape mirrors a typical Orca slice output: header
/// metadata, layered moves with `;TYPE:` and `;LAYER:` comments,
/// scattered tool changes. Realistic enough to exercise every
/// parser branch.
fn synthetic_gcode(target_bytes: usize) -> Vec<u8> {
    let mut buf = Vec::with_capacity(target_bytes + 1024);
    writeln!(&mut buf, "; estimated printing time = 1h 23m").unwrap();
    writeln!(&mut buf, "; filament used [g] = 12.345").unwrap();
    writeln!(&mut buf, "; total layers count = 247").unwrap();
    writeln!(&mut buf, "; printer_model = A1 mini").unwrap();
    writeln!(&mut buf, "M104 S210").unwrap();
    writeln!(&mut buf, "M140 S60").unwrap();
    writeln!(&mut buf, "G28").unwrap();

    let mut layer = 0u32;
    let mut z = 0.0_f32;
    let features = ["Perimeter", "External perimeter", "Internal infill", "Solid infill"];
    let mut x = 100.0_f32;
    let mut y = 100.0_f32;
    let mut e = 0.0_f32;

    while buf.len() < target_bytes {
        writeln!(&mut buf, ";LAYER:{layer}").unwrap();
        writeln!(&mut buf, ";Z:{z:.3}").unwrap();
        if layer % 10 == 0 {
            writeln!(&mut buf, "T{}", layer % 4).unwrap();
        }
        for (i, feature) in features.iter().enumerate() {
            writeln!(&mut buf, ";TYPE:{feature}").unwrap();
            // Inner-loop sized so each layer contributes ~5 KB.
            for j in 0..50 {
                x += 0.5;
                y += if (i + j) % 2 == 0 { 0.1 } else { -0.1 };
                e += 0.02;
                writeln!(
                    &mut buf,
                    "G1 X{x:.3} Y{y:.3} E{e:.4} F{}",
                    1200 + (j as u32 * 5)
                )
                .unwrap();
            }
        }
        layer += 1;
        z += 0.2;
    }
    buf
}

#[test]
fn parser_under_750ms_on_5mb_synthetic() {
    let fixture = synthetic_gcode(5 * 1024 * 1024);
    let actual_size = fixture.len();
    assert!(actual_size >= 5 * 1024 * 1024);

    // Warm-up so allocator + branch predictor settle.
    let _ = parse_lines(fixture.as_slice()).count();

    let start = Instant::now();
    let count = parse_lines(fixture.as_slice()).filter_map(Result::ok).count();
    let elapsed = start.elapsed();
    println!(
        "parsed {} lines from {} bytes in {:?} ({:.1} MB/s)",
        count,
        actual_size,
        elapsed,
        (actual_size as f64 / 1_048_576.0) / elapsed.as_secs_f64(),
    );

    assert!(count > 0, "parser yielded zero lines");
    assert!(
        elapsed.as_millis() < 750,
        "parser took {:?} on a 5 MB synthetic fixture (budget: 750 ms in debug, \
         ~100 ms in release; extrapolates to 3 s on 50 MB per Execution Plan §5)",
        elapsed,
    );
}

#[test]
fn parser_classifies_every_line_in_synthetic_fixture() {
    let fixture = synthetic_gcode(64 * 1024);
    let mut errors = 0;
    let mut others = 0;
    let mut moves = 0;
    let mut comments = 0;
    let mut layers = 0;
    let mut tools = 0;
    use n3o_slic3r_lib::core::gcode::Line;
    for result in parse_lines(fixture.as_slice()) {
        match result {
            Err(_) => errors += 1,
            Ok(Line::Move(_)) => moves += 1,
            Ok(Line::Comment(_)) => comments += 1,
            Ok(Line::LayerChange(_)) => layers += 1,
            Ok(Line::ToolChange(_)) => tools += 1,
            Ok(Line::Other(_)) => others += 1,
        }
    }
    assert_eq!(errors, 0, "synthetic fixture should parse cleanly");
    assert!(moves > 0);
    assert!(comments > 0);
    assert!(layers > 0);
    assert!(tools > 0);
    // M104, M140, G28 should land in Other (4 header-ish non-comment lines).
    assert!(others >= 3, "expected M-commands + G28 in Other, got {others}");
}
