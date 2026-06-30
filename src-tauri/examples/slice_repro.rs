//! Repro spike: load a .3mf, slice it on the bambi instance via the
//! orchestrator, dump every event. Mirrors what `slice_active_plate`
//! does from the UI — minus the Tauri/Project plumbing.
//!
//! Usage from the workspace root:
//!   cargo run -p n3o-slic3r --release --example slice_repro -- \
//!       <path/to.3mf> [--override key=value ...]
//!
//! `--override key=value` injects plate-level config overrides (highest
//! precedence in the cascade). Repeat for multiple keys. Example:
//!   --override enable_support=1 --override support_style=organic
//!
//! Prints each `SliceEvent` in the order it fires. Pay attention to
//! `JobFailed` — that's where the typed `SliceError` lands.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use n3o_slic3r_lib::core::cascade::commands::{ContextJson, OverrideFileSpec};
use n3o_slic3r_lib::core::filament::FilamentProfile;
use n3o_slic3r_lib::core::printer::{lookup, lookup_instance};
use n3o_slic3r_lib::core::scene::build_plate::BuildPlate;
use n3o_slic3r_lib::core::slice::{
    orchestrator::{run_slice_job_blocking, EventSink},
    JobRegistry, SliceEvent, SliceJobInput,
};
use slic3r_ffi::init as ffi_init;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let model_path = args.next().ok_or_else(|| {
        "usage: slice_repro <model.3mf> [--instance ID] [--override key=value ...]".to_string()
    })?;
    let model_abs = std::fs::canonicalize(&model_path)?;
    eprintln!("model: {}", model_abs.display());

    // Collect --instance / --override flags. Override builds a TOML
    // body the orchestrator parses as plate-tier overrides (highest
    // precedence). Instance defaults to "bambi"; pass "snappy" for U1.
    let mut instance_id = "bambi".to_string();
    let mut override_lines: Vec<String> = Vec::new();
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--instance" => {
                instance_id = args.next().ok_or("--instance requires ID")?;
            }
            "--override" => {
                let pair = args.next().ok_or("--override requires key=value")?;
                let (k, v) = pair.split_once('=').ok_or("override must be key=value")?;
                // libslic3r config values serialize as strings; the
                // override loader requires TOML-shaped k = "v".
                override_lines.push(format!("{k} = {:?}", v));
            }
            other => return Err(format!("unexpected arg {other:?}").into()),
        }
    }
    let project_overrides: Vec<OverrideFileSpec> = if override_lines.is_empty() {
        vec![]
    } else {
        let body = override_lines.join("\n") + "\n";
        eprintln!("plate overrides:\n{body}");
        vec![OverrideFileSpec {
            label: "spike-cli".into(),
            content: body,
        }]
    };

    ffi_init(None, 3).map_err(|e| format!("libslic3r init: {e}"))?;

    // The instance registry is seeded lazily by the test helpers — the
    // app's setup hook isn't running here. `lookup_instance` returns a
    // None if the registry hasn't been seeded; force it via the bundled
    // catalog.
    let instance = lookup_instance(&instance_id)
        .ok_or_else(|| format!("instance `{instance_id}` missing from registry"))?;
    let printer = lookup(&instance.printer_fragment_slug)
        .ok_or_else(|| format!("printer profile for `{instance_id}` missing from registry"))?;
    eprintln!(
        "{instance_id}: {} extruders × {} slots, bed = {}",
        instance.extruders.len(),
        instance.extruders[0].slots.len(),
        instance.bed.identity,
    );

    let plate = BuildPlate {
        identity: instance.bed.identity.clone(),
        // The composer hydrates this; the context-side value is only
        // read for predicate matching, not the actual slice config.
        libslic3r_curr_bed_type: instance.bed.identity.clone(),
    };

    let filament = FilamentProfile {
        identity: "Generic PLA".into(),
        base_type: "PLA".into(),
        vendor: None,
        color: None,
    };

    let temp_dir = std::env::temp_dir().join(format!("n3o-slice-repro-{}", std::process::id()));
    std::fs::create_dir_all(&temp_dir)?;
    eprintln!("output dir: {}", temp_dir.display());

    // Build the in-memory geometry the orchestrator now consumes
    // (buffer-load via Model::add_object). A grouped .3mf falls back to
    // the temp-.3mf path, which the single-mesh add_object can't do.
    use n3o_slic3r_lib::core::slice::input::SliceObject;
    let objects: Vec<SliceObject> =
        if model_abs.extension().and_then(|e| e.to_str()) == Some("3mf") {
            let p = n3o_slic3r_lib::core::threemf::load_3mf(&model_abs)?;
            p.objects
                .iter()
                .map(|o| {
                    let m = &p.meshes[o.mesh_idx];
                    SliceObject {
                        name: o.name.clone(),
                        vertices: Arc::new(m.vertices.clone()),
                        indices: Arc::new(m.indices.clone()),
                        paint: m.paint_colors.clone().map(Arc::new),
                        transform: o.transform.matrix.map(f64::from),
                        extruder: o.extruder_id.unwrap_or(1) as i32,
                        overrides: o
                            .overrides
                            .iter()
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect(),
                        group: o.group,
                        modifiers: vec![],
                    }
                })
                .collect()
        } else {
            let m = n3o_slic3r_lib::core::scene::loaders::load_mesh_from_path(&model_abs)?;
            vec![SliceObject {
                name: "model".into(),
                vertices: Arc::new(m.vertices),
                indices: Arc::new(m.indices),
                paint: m.paint_colors.map(Arc::new),
                transform: [
                    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
                ],
                extruder: 1,
                overrides: vec![],
                group: None,
                modifiers: vec![],
            }]
        };

    let input = SliceJobInput {
        objects,
        output_dir: temp_dir.display().to_string(),
        context: ContextJson {
            printer,
            plate,
            // 4-color model wants 4 filaments. Composer pulls real
            // identities off the bound instance's slots; this list is
            // for the cascade-context filament.* predicates only.
            filaments: (0..4).map(|_| filament.clone()).collect(),
            active_slot: 0,
            user_overrides: vec![],
            project_overrides,
            object_overrides: HashMap::new(),
        },
        plate_ids: vec![1],
        printer_instance_id: instance_id.clone(),
        material_layout: vec![],
        quality_profile: None,
        paint_filament_remap: None,
    };

    let bucket: Arc<Mutex<Vec<SliceEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let bucket_for_cb = bucket.clone();
    let sink: EventSink = Box::new(move |event| {
        // Stream each event as it fires (so progress shows up live)
        // AND record it for the post-run dump.
        eprintln!("event: {event:?}");
        bucket_for_cb.lock().unwrap().push(event);
    });

    let registry = JobRegistry::new();
    match run_slice_job_blocking(input, &registry, sink) {
        Ok(job_id) => eprintln!("synchronous start ok (job_id={})", job_id.0),
        Err(e) => {
            eprintln!("SYNCHRONOUS START FAILED: {e}");
            eprintln!("debug: {e:?}");
            std::process::exit(2);
        }
    }

    let events = bucket.lock().unwrap();
    eprintln!("---");
    eprintln!("total events: {}", events.len());
    for ev in events.iter() {
        if let SliceEvent::JobFailed {
            plate_id, error, ..
        } = ev
        {
            eprintln!("JOB FAILED on plate {plate_id}");
            eprintln!("SliceError variant: {error:?}");
            eprintln!("Display: {error}");
            std::process::exit(3);
        }
    }
    eprintln!("ok");
    Ok(())
}
