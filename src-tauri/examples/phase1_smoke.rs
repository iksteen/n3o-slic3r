//! Phase 1 exit-criteria smoke (PR-1-11).
//!
//! Drives the full PR-1-1 .. PR-1-7 pipeline end-to-end. Loads the A1
//! mini printer + Textured PEI plate + Generic PLA filament from the
//! profile registries, parses the small demo cascade fixture,
//! validates against the schema, resolves PLA on Textured PEI, prints
//! the structured trace for `bed_temp`, applies a project-tier
//! override on top, and runs the adapter to produce a
//! `slic3r_ffi::Config`.
//!
//! No actual slice (yet). Production slicing wiring lands when PR-1-9
//! exposes the adapter through the Tauri command surface.
//!
//! Run from the workspace root:
//!   cargo run -p n3o-slic3r --release --example phase1_smoke

use n3o_slic3r_lib::core::cascade::{
    loader::parse_cascade_str, parse_override_str, resolve_with_overrides, trace, validate_cascade,
    Cascade, KnownDimensions, OverrideTiers,
};
use n3o_slic3r_lib::core::cascade_adapter::{adapt_with_overrides, Manifest};
use n3o_slic3r_lib::core::filament::registry as filament_registry;
use n3o_slic3r_lib::core::printer::registry as printer_registry;
use n3o_slic3r_lib::core::project::SlicingContext;
use n3o_slic3r_lib::core::scene::build_plate;
use slic3r_ffi::init;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init(None, 3).map_err(|e| format!("libslic3r init: {e}"))?;

    println!("=== Phase 1 exit-criteria smoke ===\n");

    println!("[1/6] Loading reference profiles via registries");
    let printer = printer_registry::lookup("bambu-lab-a1-mini")
        .ok_or("bambu-lab-a1-mini missing from registry")?;
    let plate = build_plate::lookup("Textured PEI Plate")
        .ok_or("Textured PEI Plate missing from registry")?;
    let filament = filament_registry::lookup("generic-pla")
        .ok_or("generic-pla missing from registry")?;
    println!(
        "  printer:  {} ({} slots, {} toolheads)",
        printer.model,
        printer.slot_count,
        printer.toolheads.len()
    );
    println!(
        "  plate:    {} → libslic3r curr_bed_type = {:?}",
        plate.identity, plate.libslic3r_curr_bed_type
    );
    println!("  filament: {} ({})", filament.identity, filament.base_type);

    println!("\n[2/6] Building SlicingContext");
    let ctx = SlicingContext::new(Arc::new(printer), Arc::new(plate), vec![Arc::new(filament)]);
    println!("  active_slot = {}", ctx.active_slot);

    println!("\n[3/6] Parsing + validating demo cascade");
    let cascade_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/bambu-lab-a1-mini-demo.toml");
    let src = std::fs::read_to_string(&cascade_path)?;
    let cascade = Cascade {
        rules: parse_cascade_str(&src, Path::new("bambu-lab-a1-mini-demo.toml"))?,
    };
    println!("  cascade: {} rules parsed", cascade.rules.len());
    let known_dims =
        KnownDimensions::new(["printer.model", "filament.type", "filament.name", "plate.type"]);
    validate_cascade(&cascade, &known_dims).map_err(|errs| {
        let summary = errs
            .iter()
            .map(|e| format!("  - {e}"))
            .collect::<Vec<_>>()
            .join("\n");
        format!("cascade validation failed:\n{summary}")
    })?;
    println!("  validation: OK");

    println!("\n[4/6] Resolving cascade against context");
    let overrides_empty = OverrideTiers::empty();
    let resolved = resolve_with_overrides(&cascade, &overrides_empty, &ctx);
    println!("  resolved keys: {}", resolved.len());
    for key in ["layer_height", "nozzle_temperature", "bed_temp"] {
        if let Some(v) = resolved.get(key) {
            println!(
                "    {key:>22} = {:<12} (spec={})",
                v.value, v.winning_specificity
            );
        }
    }

    println!("\n[5/6] Tracing bed_temp");
    let t = trace(&resolved, "bed_temp").expect("bed_temp traced");
    print!("{t}");

    println!("\n[6/6] Applying project override + running adapter");
    let project = parse_override_str("bed_temp = 50\n", Path::new("project.toml"))?;
    let overrides_with_project = OverrideTiers {
        user: vec![],
        project: vec![project],
        object: None,
    };
    let resolved_overridden = resolve_with_overrides(&cascade, &overrides_with_project, &ctx);
    let t2 = trace(&resolved_overridden, "bed_temp").expect("bed_temp traced with override");
    print!("{t2}");

    let manifest = Manifest::build();
    let adapt_result = adapt_with_overrides(&resolved_overridden, &ctx, &manifest)?;
    let mut dropped = 0usize;
    let mut remapped = 0usize;
    let mut unknown = 0usize;
    let mut parse_err = 0usize;
    let mut expanded_keys = 0usize;
    for event in &adapt_result.events {
        use n3o_slic3r_lib::core::cascade_adapter::AdaptEvent;
        match event {
            AdaptEvent::Dropped { .. } => dropped += 1,
            AdaptEvent::Remapped { .. } => remapped += 1,
            AdaptEvent::UnknownKey { .. } => unknown += 1,
            AdaptEvent::ParseValueError { .. } => parse_err += 1,
            AdaptEvent::BedTempExpanded { targets, .. } => expanded_keys += targets.len(),
            AdaptEvent::CurrBedTypeSet { .. } => {}
        }
    }
    let accepted = resolved_overridden.len() - dropped - unknown - parse_err;
    println!(
        "  adapter: {} accepted, {} dropped, {} remapped, {} unknown, {} parse-error, \
         {} keys filled by bed_temp expansion",
        accepted, dropped, remapped, unknown, parse_err, expanded_keys
    );

    let layer_height_in_config = adapt_result.config.get("layer_height").unwrap_or_default();
    let hot_plate_temp_in_config = adapt_result.config.get("hot_plate_temp").unwrap_or_default();
    let curr_bed_type_in_config = adapt_result.config.get("curr_bed_type").unwrap_or_default();
    println!(
        "  Config spot-check: layer_height={:?}, hot_plate_temp={:?}, curr_bed_type={:?}",
        layer_height_in_config, hot_plate_temp_in_config, curr_bed_type_in_config
    );

    println!("\n=== smoke OK ===");
    Ok(())
}
