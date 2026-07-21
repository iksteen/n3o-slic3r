//! Dump the real slicer option summaries (process/object, machine, extruder,
//! filament) for a printer, so the browser demo's settings panels show
//! authentic n3o settings instead of an empty list.
//!
//! Usage: cargo run -p n3o-slic3r --example dump_settings -- <out-dir> [printer-identity]

use n3o_slic3r_lib::core::printer::options::{
    slicer_extruder_options_for_printer, slicer_filament_options,
    slicer_machine_options_for_printer, slicer_options_for_printer,
};
use slic3r_ffi::init as ffi_init;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let out = args.next().ok_or("usage: dump_settings <out-dir> [identity]")?;
    let identity = args.next().unwrap_or_else(|| "bambu-lab-a1-mini".to_string());

    // The option summaries read the libslic3r schema; it must be initialized.
    ffi_init(None, 3).map_err(|e| format!("libslic3r init: {e}"))?;

    let printer = n3o_slic3r_lib::core::printer::lookup(&identity)
        .ok_or_else(|| format!("printer profile `{identity}` not found"))?;

    let dir = std::path::Path::new(&out);
    std::fs::create_dir_all(dir)?;

    write(dir, "settings-process.json", &slicer_options_for_printer(printer.clone(), None))?;
    write(dir, "settings-machine.json", &slicer_machine_options_for_printer(printer.clone(), None))?;
    write(dir, "settings-extruder.json", &slicer_extruder_options_for_printer(printer.clone(), None))?;
    write(dir, "settings-filament.json", &slicer_filament_options(None))?;

    // Resolved cascade values for a bound instance (needs --features
    // test-fixtures for the `bambi` A1 mini instance), so the demo panel shows a
    // configured printer's values, not bare compile-time defaults. `{key:
    // {value, source_layer}}` — the same shape `plate_cascade_resolve` returns.
    if let Some(instance) = n3o_slic3r_lib::core::printer::lookup_instance("bambi") {
        use n3o_slic3r_lib::core::cascade::overrides::{
            resolve_with_overrides, to_resolved, OverrideTiers,
        };
        use n3o_slic3r_lib::core::printer::SlotRef;
        use n3o_slic3r_lib::core::profile_library::{compose_cascade, with_quality_profile};
        use n3o_slic3r_lib::core::project::SlicingContext;
        use std::sync::Arc;

        let bed = n3o_slic3r_lib::core::scene::build_plate::lookup(&instance.bed.identity)
            .unwrap_or_else(|| n3o_slic3r_lib::core::scene::build_plate::BuildPlate {
                identity: instance.bed.identity.clone(),
                libslic3r_curr_bed_type: format!("{} Plate", instance.bed.identity),
            });
        let filament = n3o_slic3r_lib::core::filament::FilamentProfile {
            identity: "Generic PLA".into(),
            base_type: "PLA".into(),
            vendor: None,
            color: None,
        };
        let ctx = SlicingContext::new(Arc::new(printer.clone()), Arc::new(bed), vec![Arc::new(filament)]);
        let eff = with_quality_profile(&instance, None);
        let cascade = compose_cascade(&eff, &[Some(SlotRef { extruder: 0, slot: 0 })])?;
        let resolved = to_resolved(&resolve_with_overrides(&cascade, &OverrideTiers::default(), &ctx));
        let entries: std::collections::BTreeMap<String, serde_json::Value> = resolved
            .iter()
            .map(|(k, v)| (k.clone(), serde_json::json!({ "value": v.value, "source_layer": "profile" })))
            .collect();
        write(dir, "settings-resolved.json", &entries)?;
    } else {
        eprintln!("no `bambi` instance (run with --features test-fixtures) — skipping resolved dump");
    }
    Ok(())
}

fn write<T: serde::Serialize>(
    dir: &std::path::Path,
    name: &str,
    v: &T,
) -> Result<(), Box<dyn std::error::Error>> {
    let json = serde_json::to_string(v)?;
    std::fs::write(dir.join(name), &json)?;
    eprintln!("wrote {name} ({} bytes)", json.len());
    Ok(())
}
