//! Pre-flight safety gate on the resolved cascade.
//!
//! Runs in the orchestrator after `cascade::resolve` and before
//! the adapter feeds libslic3r. Catches the class of bug where
//! the bundled cascade is missing a safety-critical macro field
//! (e.g. the Phase 1 demonstration cascade had no
//! `machine_start_gcode` — libslic3r would emit empty start-of-
//! print sequence, the printer would skip homing/leveling, and
//! the nozzle would crash into / extrude onto the bed).
//!
//! Gate semantics: collect every issue (don't fail fast), return
//! them all so the UI can render the complete picture instead of
//! the user fixing one issue at a time. The orchestrator
//! converts a non-empty `Vec` into [`SliceError::UnsafeCascade`].

use crate::core::cascade::Resolved;
use crate::core::printer::PrinterProfile;

/// Inspect the resolved cascade for unsafe-to-send conditions.
///
/// `Ok(())` means clear-for-slice. `Err(issues)` is a complete
/// list of every problem found — the UI is expected to surface
/// them all at once.
pub fn validate_resolved_cascade(
    resolved: &Resolved,
    printer: &PrinterProfile,
) -> Result<(), Vec<String>> {
    let mut issues: Vec<String> = Vec::new();

    // Empty start-gcode = no homing, no leveling, no prime ⇒
    // toolhead extrudes from wherever it happens to be sitting.
    // The worst single-field failure mode in the whole pipeline.
    require_non_empty(resolved, "machine_start_gcode", &mut issues);

    // Empty end-gcode = no park, no cool, no Z-lift. Cosmetic
    // failure mostly but still: don't ship without it.
    require_non_empty(resolved, "machine_end_gcode", &mut issues);

    // AMS / multi-toolhead printers need the filament-swap macro
    // or the printer just emits raw T<n> with no purge/flush/wipe.
    if printer.has_multiple_slots() {
        require_non_empty(resolved, "change_filament_gcode", &mut issues);
    }

    // Acceleration envelope — without it, libslic3r's defaults
    // can exceed the printer's mechanical limits → skipped steps,
    // layer shift, possible belt damage.
    require_positive_first(
        resolved,
        "machine_max_acceleration_extruding",
        &mut issues,
    );

    // Nozzle temperature ≤ the active toolhead's max_temp. We
    // pick the FIRST toolhead's max_temp as the global ceiling —
    // mixed-toolhead printers (U1) are out of MVP scope here;
    // when we add per-slot validation it goes through this same
    // function with an extra `slot` parameter.
    if let Some(max_temp) = printer.toolheads.first().map(|t| t.max_temp) {
        require_temp_within(
            resolved,
            "nozzle_temperature",
            max_temp,
            &mut issues,
        );
        require_temp_within(
            resolved,
            "nozzle_temperature_initial_layer",
            max_temp,
            &mut issues,
        );
    }

    if issues.is_empty() {
        Ok(())
    } else {
        Err(issues)
    }
}

fn require_non_empty(resolved: &Resolved, key: &str, issues: &mut Vec<String>) {
    match resolved.get(key) {
        None => issues.push(format!("cascade is missing `{key}`")),
        Some(rv) if rv.value.trim().is_empty() => {
            issues.push(format!("cascade resolved `{key}` to an empty string"))
        }
        _ => {}
    }
}

fn require_positive_first(resolved: &Resolved, key: &str, issues: &mut Vec<String>) {
    let Some(rv) = resolved.get(key) else {
        issues.push(format!("cascade is missing `{key}`"));
        return;
    };
    // libslic3r vector fields are comma-separated; take the
    // first value as representative.
    let first = rv.value.split(',').next().unwrap_or("").trim();
    match first.parse::<f64>() {
        Ok(v) if v > 0.0 => {}
        Ok(_) => issues.push(format!(
            "cascade resolved `{key}` to a non-positive value (`{first}`)"
        )),
        Err(_) => issues.push(format!(
            "cascade resolved `{key}` to a non-numeric value (`{first}`)"
        )),
    }
}

fn require_temp_within(
    resolved: &Resolved,
    key: &str,
    max_temp: f64,
    issues: &mut Vec<String>,
) {
    let Some(rv) = resolved.get(key) else {
        // Missing temp keys aren't safety-critical for the gate —
        // libslic3r enforces its own per-filament-profile range.
        return;
    };
    let first = rv.value.split(',').next().unwrap_or("").trim();
    let Ok(v) = first.parse::<f64>() else {
        return; // Non-numeric is libslic3r's problem to reject.
    };
    if v > max_temp {
        issues.push(format!(
            "cascade resolved `{key}` to {v}°C, above the printer's max of {max_temp}°C"
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::cascade::{MatchingRule, ResolvedValue};
    use crate::core::cascade::SourceLocation;
    use crate::core::printer::profile::{BoundingBox, Toolhead};
    use std::path::PathBuf;

    fn rv(value: &str) -> ResolvedValue {
        ResolvedValue {
            value: value.to_owned(),
            winning_rule: SourceLocation {
                path: PathBuf::from("test"),
                line: 1,
            },
            winning_specificity: 0,
            matching_rules: vec![MatchingRule {
                source: SourceLocation {
                    path: PathBuf::from("test"),
                    line: 1,
                },
                specificity: 0,
                value: value.to_owned(),
                when_summary: String::new(),
            }],
        }
    }

    fn a1_mini_profile() -> PrinterProfile {
        PrinterProfile {
            model: "Bambu A1 mini".into(),
            // AMS-fed: single toolhead but multi-material via AMS.
            // The change_filament_gcode requirement keys off this.
            ams_max: 1,
            supported_build_plates: vec!["Textured PEI".into()],
            toolheads: vec![Toolhead {
                default_nozzle_diameter: 0.4,
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

    #[test]
    fn safe_cascade_passes() {
        let mut r = Resolved::new();
        r.insert("machine_start_gcode".into(), rv("G28\nG29\n"));
        r.insert("machine_end_gcode".into(), rv("M104 S0\nM140 S0\n"));
        r.insert("change_filament_gcode".into(), rv("; AMS swap\n"));
        r.insert(
            "machine_max_acceleration_extruding".into(),
            rv("20000,20000"),
        );
        r.insert("nozzle_temperature".into(), rv("220"));
        r.insert("nozzle_temperature_initial_layer".into(), rv("220"));
        assert_eq!(validate_resolved_cascade(&r, &a1_mini_profile()), Ok(()));
    }

    #[test]
    fn missing_start_gcode_blocks_slice() {
        let mut r = Resolved::new();
        r.insert("machine_end_gcode".into(), rv("M104 S0\n"));
        r.insert("change_filament_gcode".into(), rv("; swap\n"));
        r.insert("machine_max_acceleration_extruding".into(), rv("20000"));
        r.insert("nozzle_temperature".into(), rv("220"));
        let err = validate_resolved_cascade(&r, &a1_mini_profile()).unwrap_err();
        assert!(
            err.iter().any(|s| s.contains("machine_start_gcode")),
            "expected start_gcode complaint, got {err:?}"
        );
    }

    #[test]
    fn empty_string_start_gcode_blocks_slice() {
        // The demonstration-cascade failure mode that triggered
        // this whole safety regen: the field IS set, just to an
        // empty string. Catch that as separately from "missing".
        let mut r = Resolved::new();
        r.insert("machine_start_gcode".into(), rv("   \n  "));
        r.insert("machine_end_gcode".into(), rv("M104 S0\n"));
        r.insert("change_filament_gcode".into(), rv("; swap\n"));
        r.insert("machine_max_acceleration_extruding".into(), rv("20000"));
        r.insert("nozzle_temperature".into(), rv("220"));
        let err = validate_resolved_cascade(&r, &a1_mini_profile()).unwrap_err();
        assert!(err.iter().any(|s| s.contains("empty string")));
    }

    #[test]
    fn change_filament_required_only_when_multi_material() {
        // Strip the AMS to make this a single-material A1 mini.
        let mut single_slot = a1_mini_profile();
        single_slot.ams_max = 0;
        let mut r = Resolved::new();
        r.insert("machine_start_gcode".into(), rv("G28\n"));
        r.insert("machine_end_gcode".into(), rv("M104 S0\n"));
        r.insert("machine_max_acceleration_extruding".into(), rv("20000"));
        r.insert("nozzle_temperature".into(), rv("220"));
        // No change_filament_gcode — should pass for single-material.
        assert_eq!(validate_resolved_cascade(&r, &single_slot), Ok(()));
        // But fail for AMS-capable printer.
        assert!(validate_resolved_cascade(&r, &a1_mini_profile()).is_err());
    }

    #[test]
    fn over_max_temp_blocks_slice() {
        let mut r = Resolved::new();
        r.insert("machine_start_gcode".into(), rv("G28\n"));
        r.insert("machine_end_gcode".into(), rv("M104 S0\n"));
        r.insert("change_filament_gcode".into(), rv("; swap\n"));
        r.insert("machine_max_acceleration_extruding".into(), rv("20000"));
        r.insert("nozzle_temperature".into(), rv("350")); // > 300 max
        r.insert("nozzle_temperature_initial_layer".into(), rv("220"));
        let err = validate_resolved_cascade(&r, &a1_mini_profile()).unwrap_err();
        assert!(err.iter().any(|s| s.contains("above the printer's max")));
    }

    #[test]
    fn zero_acceleration_blocks_slice() {
        let mut r = Resolved::new();
        r.insert("machine_start_gcode".into(), rv("G28\n"));
        r.insert("machine_end_gcode".into(), rv("M104 S0\n"));
        r.insert("change_filament_gcode".into(), rv("; swap\n"));
        r.insert("machine_max_acceleration_extruding".into(), rv("0,0"));
        r.insert("nozzle_temperature".into(), rv("220"));
        let err = validate_resolved_cascade(&r, &a1_mini_profile()).unwrap_err();
        assert!(err.iter().any(|s| s.contains("non-positive")));
    }

    #[test]
    fn multiple_issues_all_surface() {
        // No fail-fast — every issue lands in the issues list so
        // the UI can show the user the complete picture.
        let r = Resolved::new(); // empty — everything's missing
        let err = validate_resolved_cascade(&r, &a1_mini_profile()).unwrap_err();
        assert!(err.len() >= 4, "expected ≥4 issues, got {}: {err:?}", err.len());
    }
}
