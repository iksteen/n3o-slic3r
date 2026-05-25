//! Runtime cascade composer (PR-S-5a rework, hierarchical layout).
//!
//! Composes a slice-time cascade from the hierarchical vendor fragment
//! layout introduced by PR-S-4 (per-printer printer.toml + per-nozzle
//! scalar nozzle.toml + per-bed bed.toml + filament + process), plus
//! the plate-level process overrides.
//!
//! Composition order (lowest precedence first; later layers win in the
//! cascade resolver's source-order tie-break):
//!
//!   1. Printer fragment      (machine globals only; no per-extruder)
//!   2. Per-extruder nozzle fragments, scalar-to-vector assembled
//!   3. Bed fragment          (bed identity + curr_bed_type)
//!   4. Filament fragment     (single-slot MVP)
//!   5. Process fragment
//!   6. Plate process overrides
//!
//! The per-extruder vector assembly step zips each nozzle fragment's
//! scalars into vectors keyed at the extruder dimension. For an A1
//! mini (1 extruder) the vectors are length 1; for a U1 (4 extruders)
//! the vectors are length 4 with one entry per extruder. The composer
//! emits each vector key as a single synthesized cascade rule whose
//! value is the libslic3r-style semicolon-separated string the FFI
//! consumes.

use super::{
    load_bed_fragment, load_filament_fragment, load_nozzle_fragment,
    load_printer_fragment, load_process_fragment,
};
use crate::core::cascade::types::{Cascade, Predicate, Rule, SourceLocation};
use crate::core::printer::PrinterInstance;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Errors from composing a slice-time cascade.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposeError {
    UnknownPrinterFragment(String),
    UnknownNozzleFragment {
        printer_slug: String,
        sku: String,
    },
    UnknownBedFragment(String),
    UnknownFilamentFragment(String),
    UnknownProcessFragment(String),
    NoExtruders(String),
}

impl std::fmt::Display for ComposeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownPrinterFragment(s) => {
                write!(f, "no bundled printer fragment for slug `{s}`")
            }
            Self::UnknownNozzleFragment { printer_slug, sku } => {
                write!(f, "no bundled nozzle fragment for printer `{printer_slug}` SKU `{sku}`")
            }
            Self::UnknownBedFragment(s) => write!(f, "no bundled bed fragment for slug `{s}`"),
            Self::UnknownFilamentFragment(s) => {
                write!(f, "no bundled filament fragment for slug `{s}`")
            }
            Self::UnknownProcessFragment(s) => {
                write!(f, "no bundled process fragment for slug `{s}`")
            }
            Self::NoExtruders(id) => {
                write!(f, "PrinterInstance `{id}` has no extruders")
            }
        }
    }
}

impl std::error::Error for ComposeError {}

/// Compose the slice-time cascade for `instance`.
///
/// `plate_overrides` becomes the highest-precedence layer (the
/// resolver's source-order tie-break makes later sources win).
pub fn compose_cascade(
    instance: &PrinterInstance,
    plate_overrides: &BTreeMap<String, String>,
) -> Result<Cascade, ComposeError> {
    if instance.extruders.is_empty() {
        return Err(ComposeError::NoExtruders(instance.id.clone()));
    }

    let mut rules: Vec<Rule> = Vec::new();

    // 1. Printer fragment — machine globals only (per-extruder keys
    //    are deliberately absent here; they come from step 2).
    let printer = load_printer_fragment(&instance.printer_fragment_slug)
        .ok_or_else(|| ComposeError::UnknownPrinterFragment(instance.printer_fragment_slug.clone()))?;
    rules.extend(printer.rules);

    // 2. Per-extruder nozzle fragments → vector assembly.
    //    Load one nozzle fragment per extruder using its
    //    `installed_nozzle.diameter_mm` as the SKU. Then merge their
    //    scalar values into per-key vectors and synthesize one cascade
    //    rule whose set contains those vector strings.
    let nozzle_vectors = assemble_nozzle_vectors(instance)?;
    if !nozzle_vectors.is_empty() {
        rules.push(Rule {
            when: Predicate::default(),
            set: nozzle_vectors,
            source: SourceLocation {
                path: PathBuf::from("<extruder-vector-assembly>"),
                line: 1,
            },
        });
    }

    // 3. Bed fragment — looked up by `(printer_slug, bed_identity)`
    //    where the identity matches libslic3r's `curr_bed_type` enum
    //    value verbatim.
    let bed = load_bed_fragment(&instance.printer_fragment_slug, &instance.bed.identity)
        .ok_or_else(|| ComposeError::UnknownBedFragment(format!(
            "{}/{}",
            instance.printer_fragment_slug,
            instance.bed.identity,
        )))?;
    rules.extend(bed.rules);

    // 4. Filament fragment — slot-0-bound filament or instance default.
    //    Multi-slot vector assembly for filament keys lands when the
    //    instance actually carries multiple bound slots; MVP uses
    //    slot 0 only.
    let filament_slug = instance
        .extruders
        .first()
        .and_then(|e| e.slots.first())
        .and_then(|s| s.filament_identity.as_deref())
        .unwrap_or(&instance.default_filament_fragment_slug);
    let filament = load_filament_fragment(filament_slug)
        .ok_or_else(|| ComposeError::UnknownFilamentFragment(filament_slug.to_owned()))?;
    rules.extend(filament.rules);

    // 5. Process fragment — printer-bound, looked up by
    //    `(printer_fragment_slug, default_process_fragment_slug)`.
    let process = load_process_fragment(
        &instance.printer_fragment_slug,
        &instance.default_process_fragment_slug,
    )
    .ok_or_else(|| ComposeError::UnknownProcessFragment(format!(
        "{}/{}",
        instance.printer_fragment_slug,
        instance.default_process_fragment_slug,
    )))?;
    rules.extend(process.rules);

    // 6. Plate overrides — virtual source so trace UI can name them.
    if !plate_overrides.is_empty() {
        rules.push(Rule {
            when: Predicate::default(),
            set: plate_overrides.clone(),
            source: SourceLocation {
                path: PathBuf::from("<plate-overrides>"),
                line: 1,
            },
        });
    }

    Ok(Cascade { rules })
}

/// Walk the instance's extruders, load each one's nozzle fragment,
/// and zip the scalar values into per-extruder vector strings keyed
/// by the per-extruder option keys. libslic3r consumes per-extruder
/// vectors as semicolon-separated strings in the cascade `set` map.
fn assemble_nozzle_vectors(
    instance: &PrinterInstance,
) -> Result<BTreeMap<String, String>, ComposeError> {
    // Load each extruder's nozzle scalars.
    let mut per_extruder: Vec<BTreeMap<String, String>> =
        Vec::with_capacity(instance.extruders.len());
    for extruder in &instance.extruders {
        let sku = nozzle_sku_string(&extruder.installed_nozzle);
        let cascade = load_nozzle_fragment(&instance.printer_fragment_slug, &sku).ok_or_else(
            || ComposeError::UnknownNozzleFragment {
                printer_slug: instance.printer_fragment_slug.clone(),
                sku: sku.clone(),
            },
        )?;
        // A nozzle fragment is a single unconditional default rule
        // (the converter never emits [[rule]] blocks).
        let scalars = cascade
            .rules
            .into_iter()
            .next()
            .map(|r| r.set)
            .unwrap_or_default();
        per_extruder.push(scalars);
    }

    // Union of all keys seen across extruders → vector key set.
    let mut all_keys: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for scalars in &per_extruder {
        for k in scalars.keys() {
            all_keys.insert(k.clone());
        }
    }

    // For each key, build a length-N vector where missing entries
    // fall back to the same key's value on extruder 0 (the printer's
    // canonical nozzle). Empty string if even extruder 0 doesn't
    // declare the key — the adapter will reject downstream if that's
    // a problem.
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    for key in all_keys {
        let values: Vec<String> = per_extruder
            .iter()
            .map(|scalars| {
                scalars
                    .get(&key)
                    .cloned()
                    .or_else(|| per_extruder.first().and_then(|p| p.get(&key).cloned()))
                    .unwrap_or_default()
            })
            .collect();
        // libslic3r's vector option format: semicolon-separated.
        out.insert(key, values.join(";"));
    }

    Ok(out)
}

/// Format a NozzleSku for fragment lookup. Currently just the
/// diameter as the SKU string (matches the converter's filename
/// convention). Future: incorporate material when we author
/// hotend-material-specific nozzle files.
fn nozzle_sku_string(nozzle: &crate::core::printer::NozzleSku) -> String {
    // Trim trailing zero on round diameters: 0.4 not 0.400000.
    let d = nozzle.diameter_mm;
    if (d - d.round()).abs() < 1e-6 {
        format!("{:.0}", d)
    } else {
        // One decimal place for 0.2/0.4/0.6/0.8 etc.
        format!("{:.1}", d)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::printer::lookup_instance;

    #[test]
    fn compose_bambi_yields_printer_nozzle_bed_filament_process_layers() {
        let bambi = lookup_instance("bambi").expect("bambi present");
        let cascade = compose_cascade(&bambi, &BTreeMap::new()).expect("compose");

        // Each layer contributes at least one rule. With 5 layers + nozzle
        // assembly + no plate overrides → ≥ 6 rules (printer, nozzle,
        // bed, filament, process).
        assert!(
            cascade.rules.len() >= 5,
            "expected ≥ 5 rules, got {}",
            cascade.rules.len(),
        );

        let all_keys: std::collections::BTreeSet<&String> =
            cascade.rules.iter().flat_map(|r| r.set.keys()).collect();
        assert!(all_keys.contains(&"printable_height".to_owned()),
                "printer-bucket key missing");
        assert!(all_keys.contains(&"nozzle_diameter".to_owned()),
                "per-extruder nozzle key missing from composition");
        assert!(all_keys.contains(&"curr_bed_type".to_owned()),
                "bed-fragment key missing");
        assert!(all_keys.contains(&"nozzle_temperature".to_owned()),
                "filament-bucket key missing");
        assert!(all_keys.contains(&"layer_height".to_owned()),
                "process-bucket key missing");
    }

    #[test]
    fn nozzle_vector_assembly_replicates_for_u1() {
        // Snappy has 4 extruders all bound to 0.4 SS — the assembled
        // nozzle_diameter must be "0.4;0.4;0.4;0.4".
        let snappy = lookup_instance("snappy").expect("snappy present");
        let cascade = compose_cascade(&snappy, &BTreeMap::new()).expect("compose");

        // Find the synthesized extruder-vector rule.
        let vector_rule = cascade
            .rules
            .iter()
            .find(|r| r.source.path.to_string_lossy() == "<extruder-vector-assembly>")
            .expect("extruder-vector rule present");
        let diameter = vector_rule
            .set
            .get("nozzle_diameter")
            .expect("nozzle_diameter assembled");
        assert_eq!(diameter, "0.4;0.4;0.4;0.4");
    }

    #[test]
    fn nozzle_vector_assembly_yields_single_value_for_a1_mini() {
        let bambi = lookup_instance("bambi").expect("bambi present");
        let cascade = compose_cascade(&bambi, &BTreeMap::new()).expect("compose");
        let vector_rule = cascade
            .rules
            .iter()
            .find(|r| r.source.path.to_string_lossy() == "<extruder-vector-assembly>")
            .expect("extruder-vector rule present");
        let diameter = vector_rule.set.get("nozzle_diameter").expect("diameter present");
        // A1 mini has 1 extruder → no semicolons in the vector string.
        assert_eq!(diameter, "0.4");
    }

    #[test]
    fn plate_overrides_appended_as_last_rule() {
        let bambi = lookup_instance("bambi").expect("bambi present");
        let mut overrides = BTreeMap::new();
        overrides.insert("layer_height".to_owned(), "0.12".to_owned());
        let cascade = compose_cascade(&bambi, &overrides).expect("compose");
        let last = cascade.rules.last().expect("rules");
        assert_eq!(last.set.get("layer_height").map(String::as_str), Some("0.12"));
        assert_eq!(last.source.path.to_string_lossy(), "<plate-overrides>");
    }

    #[test]
    fn missing_nozzle_fragment_errors_with_useful_message() {
        let mut bambi = lookup_instance("bambi").expect("bambi present");
        bambi.extruders[0].installed_nozzle.diameter_mm = 0.9; // not bundled
        let err = compose_cascade(&bambi, &BTreeMap::new()).unwrap_err();
        assert!(
            matches!(&err, ComposeError::UnknownNozzleFragment { sku, .. } if sku == "0.9"),
            "got {err:?}",
        );
    }

    #[test]
    fn missing_printer_fragment_errors() {
        let mut bambi = lookup_instance("bambi").expect("bambi present");
        bambi.printer_fragment_slug = "ghost".into();
        let err = compose_cascade(&bambi, &BTreeMap::new()).unwrap_err();
        assert_eq!(err, ComposeError::UnknownPrinterFragment("ghost".into()));
    }
}
