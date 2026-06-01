// cargo run --example slice -- <model_file> [<out.gcode>]
//
// Loads a model file (STL/3MF/OBJ/STEP), slices it with default FullPrintConfig
// settings (which target a generic 0.4mm-nozzle FDM printer), and writes G-code.
//
// This is a smoke test for the shim, not a fully-configured production slice.
// In a real consumer you would set printer-specific options (printable_area,
// nozzle_diameter, filament_diameter, etc.) before calling slice().

use slic3r_ffi::{init, slice, Config, Model};
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: {} <model_file> [<out.gcode>]", args[0]);
        std::process::exit(2);
    }
    let model_path = PathBuf::from(&args[1]);
    let out_path = args
        .get(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp/out.gcode"));

    // Resources dir is the OrcaSlicer resources/ tree. Only needed if you load
    // STEP files or use font embossing — STL/3MF works without it.
    let resources =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../external/OrcaSlicer/resources");
    let resources = if resources.exists() {
        Some(resources)
    } else {
        None
    };

    init(resources.as_deref(), 3).expect("init failed");

    let mut model = Model::new().expect("model alloc");
    let mut config = Config::new().expect("config alloc");
    // load_with_config seeds the config from the file's embedded settings
    // when present (3MF carries a full printer profile). STL/OBJ/STEP have
    // no embedded config and slice against FullPrintConfig defaults.
    model
        .load_with_config(&model_path, &mut config)
        .expect("model load");
    // Override anything afterwards if needed:
    //   config.set("layer_height", "0.2")?;

    println!("slicing {} -> {}", model_path.display(), out_path.display());
    match slice(&model, &config, &out_path, |percent, stage| {
        println!("  [{percent:>3}%] {stage}");
    }) {
        Ok(_) => println!("ok"),
        Err(e) => {
            eprintln!("slice failed: {e}");
            std::process::exit(1);
        }
    }
}
