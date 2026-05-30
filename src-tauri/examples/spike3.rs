//! Spike PR-0.5-3: Bambu A1 mini 4-color AMS slice.
//!
//! Loads examples/spike3/fourcolor.3mf (the MakerWorld
//! "4 Colors Benchy AMS Test (v2)" by jansonne, CC BY-NC; see
//! examples/spike3/NOTICE.md for attribution) with its embedded
//! BambuStudio config, slices via slic3r_ffi, writes
//! /tmp/spike3.gcode.
//!
//! The 3MF embeds the AMS bindings (filament_settings_id,
//! filament_ids, flush_volumes_matrix, wipe_tower position, etc.)
//! so this driver doesn't apply a cascade overlay — the spike's
//! point is to characterize what libslic3r emits when fed a
//! Bambu-shaped 4-color config, not to re-validate the cascade
//! adapter (PR-0.5-1 already did that).
//!
//! Throwaway. Not production code.
//!
//! Run from the workspace root:
//!   cargo run -p n3o-slic3r --release --example spike3

use slic3r_ffi::{init, slice, Config, Model};
use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init(None, 3).map_err(|e| format!("libslic3r init: {e}"))?;

    let root = workspace_root();
    let model_path = root.join("examples/spike3/fourcolor.3mf");
    let out_path = PathBuf::from("/tmp/spike3.gcode");

    eprintln!("model: {}", model_path.display());
    eprintln!("out:   {}\n", out_path.display());

    let mut model = Model::new()?;
    let mut config = Config::new()?;
    model.load_with_config(&model_path, &mut config)?;
    eprintln!("loaded model + embedded BBS config");

    // Sanity-check key AMS bindings the 3MF should have populated. If
    // any of these come back None, libslic3r dropped them at load time
    // and the slice will not be 4-color.
    for key in &[
        "filament_settings_id",
        "filament_type",
        "filament_ids",
        "filament_colour",
        "flush_volumes_matrix",
        "flush_volumes_vector",
        "wipe_tower_x",
        "wipe_tower_y",
        "enable_prime_tower",
        "curr_bed_type",
        "nozzle_diameter",
    ] {
        let v = config.get(key).unwrap_or_else(|_| "<missing>".into());
        let display = if v.len() > 80 {
            format!("{}…", &v[..80])
        } else {
            v
        };
        eprintln!("  {key:30} = {display}");
    }

    eprintln!("\nslicing...");
    match slice(&model, &config, &out_path, |_, _| {}) {
        Ok(()) => {
            let bytes = std::fs::metadata(&out_path)?.len();
            eprintln!("ok — wrote {} ({} bytes)", out_path.display(), bytes);
            Ok(())
        }
        Err(e) => {
            eprintln!("slice failed: {e}");
            Err(e.into())
        }
    }
}
