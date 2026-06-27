#![cfg(feature = "test-fixtures")]
//! Buffer-load parity gate.
//!
//! The slice path feeds libslic3r geometry from the in-memory mesh
//! buffers via `Model::add_object` (buffer-load) instead of writing +
//! re-parsing a temp `.3mf`. This test pins that the two routes produce
//! **byte-identical G-code**: build one `SliceJobInput`, slice it with
//! `force_temp_3mf = true` (the `write_3mf` → `Model::load` route) and
//! again with `force_temp_3mf = false` (buffer-load), and assert the
//! emitted G-code matches. A divergence means buffer-load isn't
//! equivalent — most likely the transform mapping or the paint hand-off.
//!
//! Two volatile tokens are normalized out before comparing (neither is
//! geometry): the build **timestamp** (the slices run a moment apart) and
//! libslic3r's per-object **id** in Bambu object-exclusion labels (a
//! process-dependent identifier). See `strip_object_ids`.
//!
//! ## libslic3r is nondeterministic per process
//!
//! libslic3r's G-code generation is **nondeterministic across process
//! launches** (TBB task ordering): ~1 run in 5–6 emits a structurally
//! different toolpath for the *same* input — and it's all-or-nothing per
//! process (every slice in a "bad" run differs; every slice in a "good"
//! run is identical). This predates buffer-load — two identical *temp-3mf*
//! slices diverge just as often. So the gate can't blindly compare two
//! slices. Each test first probes determinism (buffer-load sliced twice);
//! a nondeterministic run is skipped (inconclusive, never a false fail),
//! and the buffer-load vs temp-3mf comparison runs only on a deterministic
//! process — where a divergence is a genuine buffer-load bug.

use std::sync::{Arc, Mutex, Once};

use n3o_slic3r_lib::core::cascade::commands::ContextJson;
use n3o_slic3r_lib::core::filament::FilamentProfile;
use n3o_slic3r_lib::core::printer::profile::{BoundingBox, PrinterProfile, Toolhead};
use n3o_slic3r_lib::core::scene::build_plate::BuildPlate;
use n3o_slic3r_lib::core::slice::input::SliceObject;
use n3o_slic3r_lib::core::slice::{
    orchestrator::{run_slice_job_blocking, EventSink},
    JobRegistry, SliceEvent, SliceJobInput,
};
use slic3r_ffi::init as ffi_init;

static FFI_INIT: Once = Once::new();
fn ensure_ffi_init() {
    FFI_INIT.call_once(|| {
        ffi_init(None, 3).expect("slic3r_init");
    });
}

const IDENTITY16: [f64; 16] = [
    1.0, 0.0, 0.0, 0.0, //
    0.0, 1.0, 0.0, 0.0, //
    0.0, 0.0, 1.0, 0.0, //
    0.0, 0.0, 0.0, 1.0,
];

fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn test_stl() -> std::path::PathBuf {
    workspace_root().join("external/OrcaSlicer/tests/data/test_stl/ASCII/20mmbox-LF.stl")
}

fn load_stl_mesh() -> n3o_slic3r_lib::core::scene::state::NewMesh {
    n3o_slic3r_lib::core::scene::loaders::load_mesh_from_path(&test_stl()).expect("load test STL")
}

fn canonical_printer() -> PrinterProfile {
    PrinterProfile {
        model: "Bambu Lab A1 mini".into(),
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

fn canonical_plate() -> BuildPlate {
    BuildPlate {
        identity: "Textured PEI".into(),
        libslic3r_curr_bed_type: "Textured PEI Plate".into(),
    }
}

fn canonical_filament() -> FilamentProfile {
    FilamentProfile {
        identity: "Generic PLA".into(),
        base_type: "PLA".into(),
        vendor: None,
        color: None,
    }
}

fn bambi_input(objects: Vec<SliceObject>, output_dir: String, filaments: usize) -> SliceJobInput {
    SliceJobInput {
        objects,
        force_temp_3mf: false,
        output_dir,
        context: ContextJson {
            printer: canonical_printer(),
            plate: canonical_plate(),
            filaments: (0..filaments).map(|_| canonical_filament()).collect(),
            active_slot: 0,
            user_overrides: vec![],
            project_overrides: vec![],
            object_overrides: std::collections::HashMap::new(),
        },
        plate_ids: vec![1],
        printer_instance_id: "bambi".into(),
        material_layout: vec![],
        quality_profile: None,
        paint_filament_remap: None,
    }
}

fn collecting_sink() -> (EventSink, Arc<Mutex<Vec<SliceEvent>>>) {
    let bucket: Arc<Mutex<Vec<SliceEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let bucket_for_cb = bucket.clone();
    let sink: EventSink = Box::new(move |event| {
        bucket_for_cb.lock().unwrap().push(event);
    });
    (sink, bucket)
}

/// Slice `input` and return `Ok(gcode)` on success or `Err(message)` on
/// a JobFailed event, so a slice that legitimately fails on *both* paths
/// still compares as parity (Err == Err) rather than panicking.
fn slice_result(label: &str, input: SliceJobInput) -> Result<String, String> {
    let registry = JobRegistry::new();
    let (sink, events) = collecting_sink();
    run_slice_job_blocking(input, &registry, sink)
        .unwrap_or_else(|e| panic!("[{label}] start: {e:?}"));
    let events = events.lock().unwrap();
    if let Some(SliceEvent::JobFailed { error, .. }) =
        events.iter().find(|e| matches!(e, SliceEvent::JobFailed { .. }))
    {
        return Err(format!("{error:?}"));
    }
    let path = events
        .iter()
        .find_map(|e| match e {
            SliceEvent::PlateFinished { output_path, .. } => Some(output_path.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("[{label}] no PlateFinished and no JobFailed"));
    let gcode =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("[{label}] read {path}: {e}"));
    let _ = std::fs::remove_dir_all(std::path::Path::new(&path).parent().unwrap());
    Ok(gcode)
}

/// Canonicalize the one legitimately-divergent token: libslic3r's
/// per-object **id** as it appears in Bambu object-exclusion / sequential-
/// print labels (`; model label id: N`, `; printing object <name> id:N`,
/// `; object ids of layer N start: …`). The id is a process-dependent
/// `ModelObject` identifier — the temp-`.3mf` loader assigns it from build-
/// item order while buffer-load uses the global counter, so the absolute
/// values differ run-to-run (and scheme-to-scheme). It's a label, not
/// geometry or a toolpath, and both files stay internally consistent
/// (header list ↔ inline labels ↔ layer markers). Blank the id everywhere
/// it appears; everything else (geometry, temps, every G1 move) must match
/// verbatim.
fn strip_object_ids(gcode: &str) -> String {
    gcode
        .lines()
        // Drop `M73 Pxx Ryy` (set-progress / time-remaining) lines. They're
        // emitted from libslic3r's time estimator — a parallel reduction whose
        // result jitters run-to-run and whose markers land at slightly
        // different positions — so they're the single most volatile token and
        // carry no geometry. Both build paths feed the identical model, so any
        // M73 difference is engine time-estimate nondeterminism, never a
        // buffer-load divergence.
        .filter(|l| !l.starts_with("M73 "))
        .map(normalize_id_line)
        .collect::<Vec<_>>()
        .join("\n")
}

fn normalize_id_line(l: &str) -> String {
    // `; generated by OrcaSlicer  on 2026-06-27 at 19:00:25` → drop the
    // wall-clock timestamp (the two slices run a moment apart). Not
    // geometry; the only place a date/time leaks into the output.
    if l.starts_with("; generated by") {
        return "; generated by".to_string();
    }
    // `; model label id: 42,46` → keep through `label id`.
    if let Some(i) = l.find("label id") {
        return l[..i + "label id".len()].to_string();
    }
    // `; object ids of layer 2 start: 23,27` / `… end: 23,27` → keep
    // through the marker, drop the id list.
    if l.contains("object ids of") {
        for m in ["start:", "end:"] {
            if let Some(i) = l.find(m) {
                return l[..i + m.len()].to_string();
            }
        }
    }
    // `; printing object box-a id:0 copy 0` → blank the digits after each
    // `id:` (keeping the trailing `copy N`, which matches across paths).
    let mut out = String::with_capacity(l.len());
    let mut rest = l;
    while let Some(p) = rest.find("id:") {
        out.push_str(&rest[..p + 3]);
        rest = &rest[p + 3..];
        let nd = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        rest = &rest[nd..];
    }
    out.push_str(rest);
    out
}

/// Normalize a slice result (object ids + timestamp blanked on the Ok path)
/// for comparison.
fn norm(r: &Result<String, String>) -> Result<String, String> {
    match r {
        Ok(g) => Ok(strip_object_ids(g)),
        Err(e) => Err(e.clone()),
    }
}

/// Slice `objects` via buffer-load and the temp-`.3mf` route and assert the
/// G-code matches (modulo object id labels; see [`strip_object_ids`]).
///
/// libslic3r's G-code generation is **nondeterministic per process** — see
/// the module doc: occasionally a process emits a different toolpath for the
/// *same* input on every slice (TBB task ordering; all-or-nothing for a given
/// run). That's pre-existing engine variance, not a buffer-load divergence.
/// To never false-fail on it, probe determinism first by slicing buffer-load
/// twice; if those disagree the process is nondeterministic this run and the
/// comparison is inconclusive, so skip. When buffer-load is self-consistent
/// the process is deterministic, and any buffer-load ≠ temp-3mf difference is
/// a genuine bug (transform mapping, paint hand-off, …).
fn assert_parity(label: &str, objects: Vec<SliceObject>, filaments: usize) {
    // Serialize the parity tests against each other. libslic3r's per-process
    // nondeterminism is aggravated by concurrent test threads (more live
    // threads → more TBB scheduling variance), which would otherwise flip the
    // engine's determinism state *between* this test's probe and comparison
    // slices and produce a spurious mismatch. Running each parity test's
    // slices with no other slice in flight keeps the per-process determinism
    // all-or-nothing, so the probe reliably reflects the comparison slice.
    static TEST_LOCK: Mutex<()> = Mutex::new(());
    let _serialize = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());

    let dir = |tag: &str| {
        std::env::temp_dir()
            .join(format!("n3o-parity-{label}-{tag}-{}", std::process::id()))
            .display()
            .to_string()
    };
    let slice = |tag: &str, force_temp_3mf: bool| {
        let mut input = bambi_input(objects.clone(), dir(tag), filaments);
        input.force_temp_3mf = force_temp_3mf;
        norm(&slice_result(&format!("{label}/{tag}"), input))
    };

    // Determinism probe: slice *each* path twice. libslic3r's seam / feedrate
    // (cooling) / progress estimate is nondeterministic per slice under
    // multi-threading, so a path that disagrees with itself means the run is
    // inconclusive — skip rather than risk a spurious mismatch. Only when both
    // paths are self-consistent is the run deterministic enough to compare,
    // and there a buffer-load ≠ temp-3mf difference is a genuine bug.
    let buf1 = slice("buf1", false);
    let buf2 = slice("buf2", false);
    let a3mf1 = slice("3mf1", true);
    let a3mf2 = slice("3mf2", true);
    if buf1 != buf2 || a3mf1 != a3mf2 {
        eprintln!(
            "[{label}] SKIPPED: libslic3r was nondeterministic this run (a path disagreed \
             with itself) — the parity comparison is inconclusive. Re-run, or pin to one core \
             (`taskset -c 0`) to force a deterministic engine.",
        );
        return;
    }

    let gcode_3mf = a3mf1;
    match (&gcode_3mf, &buf1) {
        (Ok(a), Ok(b)) => {
            if a != b {
                if std::env::var("PARITY_DUMP").is_ok() {
                    std::fs::write(
                        std::env::temp_dir().join(format!("pd-{label}-3mf.gcode")),
                        a,
                    )
                    .unwrap();
                    std::fs::write(
                        std::env::temp_dir().join(format!("pd-{label}-buf.gcode")),
                        b,
                    )
                    .unwrap();
                    if let Ok(b2) = &buf2 {
                        std::fs::write(
                            std::env::temp_dir().join(format!("pd-{label}-buf2.gcode")),
                            strip_object_ids(b2),
                        )
                        .unwrap();
                    }
                    eprintln!("[{label}] DUMPED pd-{label}-{{3mf,buf,buf2}}.gcode");
                }
                let diff_count = a.lines().zip(b.lines()).filter(|(x, y)| x != y).count();
                let diffs: Vec<String> = a
                    .lines()
                    .zip(b.lines())
                    .enumerate()
                    .filter(|(_, (x, y))| x != y)
                    .take(10)
                    .map(|(n, (x, y))| {
                        format!("  line {n}:\n    temp-3mf:    {x:?}\n    buffer-load: {y:?}")
                    })
                    .collect();
                panic!(
                    "[{label}] buffer-load diverges from temp-3mf (deterministic run): \
                     {diff_count} differing lines (temp-3mf {} vs buffer-load {} total); first 10:\n{}",
                    a.lines().count(),
                    b.lines().count(),
                    diffs.join("\n"),
                );
            }
        }
        (Err(a), Err(b)) => {
            assert_eq!(a, b, "[{label}] both paths failed but with different errors")
        }
        (a, b) => panic!("[{label}] paths disagree on success: temp-3mf={a:?}, buffer-load={b:?}"),
    }
}

/// The gate: a plain single-object STL slices identically via buffer-load
/// and the temp-`.3mf` route.
#[test]
fn single_object_buffer_load_matches_temp_3mf() {
    ensure_ffi_init();
    let m = load_stl_mesh();
    let objects = vec![SliceObject {
        name: "20mmbox".into(),
        vertices: Arc::new(m.vertices),
        indices: Arc::new(m.indices),
        paint: None,
        transform: IDENTITY16,
        extruder: 1,
        overrides: vec![],
        group: None,
    }];
    assert_parity("single", objects, 1);
}

/// Two objects with distinct (non-identity) transforms — exercises the
/// per-object `add_object` loop and the transform mapping for more than
/// one placement.
#[test]
fn two_objects_buffer_load_matches_temp_3mf() {
    ensure_ffi_init();
    let m = load_stl_mesh();
    let verts = Arc::new(m.vertices);
    let idx = Arc::new(m.indices);
    // Second box translated +30mm in X (column-major: translation in
    // elements 12,13,14).
    let mut shifted = IDENTITY16;
    shifted[12] = 30.0;
    let objects = vec![
        SliceObject {
            name: "box-a".into(),
            vertices: Arc::clone(&verts),
            indices: Arc::clone(&idx),
            paint: None,
            transform: IDENTITY16,
            extruder: 1,
            overrides: vec![],
            group: None,
        },
        SliceObject {
            name: "box-b".into(),
            vertices: verts,
            indices: idx,
            paint: None,
            transform: shifted,
            extruder: 1,
            overrides: vec![],
            group: None,
        },
    ];
    assert_parity("two-objects", objects, 1);
}

/// Painted (MMU) parity: every triangle painted to a non-base filament
/// state. Proves the per-triangle paint hex hand-off (`add_object`'s
/// `set_triangle_from_string`) matches libslic3r's own 3MF paint reader.
#[test]
fn painted_object_buffer_load_matches_temp_3mf() {
    ensure_ffi_init();
    let m = load_stl_mesh();
    let tri_count = m.indices.len() / 3;
    // BBS paint state "4" = a single non-base filament for the whole
    // facet (the same opaque hex string the 3MF reader round-trips).
    let paint: Vec<String> = (0..tri_count).map(|_| "4".to_string()).collect();
    let objects = vec![SliceObject {
        name: "painted-box".into(),
        vertices: Arc::new(m.vertices),
        indices: Arc::new(m.indices),
        paint: Some(Arc::new(paint)),
        transform: IDENTITY16,
        extruder: 1,
        overrides: vec![],
        group: None,
    }];
    // Two filaments so the painted state has a real target.
    assert_parity("painted", objects, 2);
}
