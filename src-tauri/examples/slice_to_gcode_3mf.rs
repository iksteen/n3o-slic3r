//! Slice a .3mf locally, wrap as a Bambu `.gcode.3mf` bundle on
//! disk — no printer round-trip.
//!
//! Mirrors the same path our app's Send button takes:
//!   1. Load the input .3mf into libslic3r (embedded BBS config wins).
//!   2. Slice via slic3r_ffi → raw plate_1.gcode on disk.
//!   3. Wrap into `.gcode.3mf` via core::threemf::write_sliced_3mf
//!      (PR-3-10 writer, the same one driver_send_plate uses via
//!      `wrap_gcode_as_3mf`).
//!
//! Use to diff our slice output against a BBS-sliced .gcode.3mf
//! of the same model without uploading anything to the printer.
//!
//! Run from the workspace root:
//!   cargo run -p n3o-slic3r --release --example slice_to_gcode_3mf -- \
//!       <input.3mf> <output.gcode.3mf>

use n3o_slic3r_lib::core::threemf::{fixture_input, write_sliced_3mf};
use slic3r_ffi::{init, slice, Config, Model};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let input = PathBuf::from(args.next().ok_or(
        "usage: slice_to_gcode_3mf <input.3mf> <output.gcode.3mf>",
    )?);
    let output = PathBuf::from(args.next().ok_or(
        "usage: slice_to_gcode_3mf <input.3mf> <output.gcode.3mf>",
    )?);

    init(None, 3).map_err(|e| format!("libslic3r init: {e}"))?;

    eprintln!("input:  {}", input.display());
    eprintln!("output: {}", output.display());

    let mut model = Model::new()?;
    let mut config = Config::new()?;
    model.load_with_config(&input, &mut config)?;
    eprintln!("loaded model + embedded config");

    // Slice to a temp raw .gcode; we'll wrap it into a .gcode.3mf
    // bundle next. Same two-step our driver's send path does.
    let tmp_gcode = tempfile::Builder::new()
        .suffix(".gcode")
        .tempfile()?;
    eprintln!("slicing → {}", tmp_gcode.path().display());
    slice(&model, &config, tmp_gcode.path(), |_, _| {})?;

    let gcode_bytes = std::fs::read(tmp_gcode.path())?;
    eprintln!(
        "sliced ok ({} bytes)\nwrapping as .gcode.3mf …",
        gcode_bytes.len(),
    );

    // Mirror the wrap our driver does in
    // `core::driver::commands::wrap_gcode_as_3mf` — fixture_input
    // for a single plate, plate_id = 1.
    let input_struct = fixture_input(1, gcode_bytes);
    write_sliced_3mf(&input_struct, &output)
        .map_err(|e| format!("write .gcode.3mf: {e}"))?;

    let bundle_bytes = std::fs::metadata(&output)?.len();
    eprintln!(
        "ok — wrote {} ({} bytes)",
        output.display(),
        bundle_bytes,
    );
    Ok(())
}
