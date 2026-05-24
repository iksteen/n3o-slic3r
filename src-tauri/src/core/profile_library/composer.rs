//! Runtime cascade composer (PR-S-5).
//!
//! Composes a slice-time cascade from per-bucket vendor fragments + the
//! plate's process overrides + per-object overrides. The output is a
//! single [`Cascade`] in resolver-consumable form — layered so that
//! later sources override earlier ones (cascade resolver source-order
//! tie-break).
//!
//! Composition order (lowest precedence first):
//!
//!   1. Vendor printer fragment (machine envelopes, start_gcode, etc.)
//!   2. Vendor filament fragment for slot 0's bound filament
//!      (MVP — multi-slot vector assembly comes later)
//!   3. Vendor process fragment
//!   4. Plate `process_overrides` (per-plate strategy edits)
//!   5. Per-object overrides (already in the cascade adapter's path;
//!      this composer leaves them to the existing project_overrides
//!      mechanism in `build_slice_input`)
//!
//! See `docs/design/settings-model.md` §5.

use super::{load_fragment, Bucket};
use crate::core::cascade::types::{Cascade, Predicate, Rule, SourceLocation};
use crate::core::printer::PrinterInstance;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Errors from composing a slice-time cascade.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposeError {
    /// PrinterInstance's printer_fragment_slug doesn't match any
    /// bundled printer fragment.
    UnknownPrinterFragment(String),
    /// Filament slug (either from a bound slot or the instance default)
    /// doesn't match any bundled filament fragment.
    UnknownFilamentFragment(String),
    /// Process slug doesn't match any bundled process fragment.
    UnknownProcessFragment(String),
}

impl std::fmt::Display for ComposeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownPrinterFragment(s) => {
                write!(f, "no bundled printer fragment for slug `{s}`")
            }
            Self::UnknownFilamentFragment(s) => {
                write!(f, "no bundled filament fragment for slug `{s}`")
            }
            Self::UnknownProcessFragment(s) => {
                write!(f, "no bundled process fragment for slug `{s}`")
            }
        }
    }
}

impl std::error::Error for ComposeError {}

/// Compose a slice-time cascade for `instance`.
///
/// `plate_overrides` should be the plate's project-tier process
/// overrides — the cascade adapter will treat them as the
/// highest-precedence layer. Pass an empty map if the plate has no
/// per-plate overrides.
///
/// MVP simplification: composes against slot 0's bound filament only
/// (or `default_filament_fragment_slug` if slot 0 is unbound). When
/// multi-slot vector-key assembly lands, this expands to layer N
/// filament fragments per-slot and stitch their vector keys.
pub fn compose_cascade(
    instance: &PrinterInstance,
    plate_overrides: &BTreeMap<String, String>,
) -> Result<Cascade, ComposeError> {
    let mut rules: Vec<Rule> = Vec::new();

    // 1. Printer fragment.
    let printer = load_fragment(Bucket::Printer, &instance.printer_fragment_slug)
        .ok_or_else(|| ComposeError::UnknownPrinterFragment(instance.printer_fragment_slug.clone()))?;
    rules.extend(printer.rules);

    // 2. Filament fragment — slot 0's bound filament or the default.
    let filament_slug = instance
        .extruders
        .first()
        .and_then(|e| e.slots.first())
        .and_then(|s| s.filament_identity.as_deref())
        .unwrap_or(&instance.default_filament_fragment_slug);
    let filament = load_fragment(Bucket::Filament, filament_slug)
        .ok_or_else(|| ComposeError::UnknownFilamentFragment(filament_slug.to_owned()))?;
    rules.extend(filament.rules);

    // 3. Process fragment.
    let process = load_fragment(Bucket::Process, &instance.default_process_fragment_slug)
        .ok_or_else(|| {
            ComposeError::UnknownProcessFragment(instance.default_process_fragment_slug.clone())
        })?;
    rules.extend(process.rules);

    // 4. Plate-level process overrides — synthesized as one
    //    unconditional rule sourced from a virtual `<plate-overrides>`
    //    path so the trace UI can distinguish them.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::printer::lookup_instance;

    #[test]
    fn compose_bambi_yields_printer_filament_process_rules() {
        let bambi = lookup_instance("bambi").expect("bambi present");
        let cascade = compose_cascade(&bambi, &BTreeMap::new()).expect("compose");

        // Three vendor fragments → at least 3 rules (each fragment
        // contributes at minimum one default rule).
        assert!(
            cascade.rules.len() >= 3,
            "expected ≥ 3 rules, got {}",
            cascade.rules.len(),
        );

        // Sanity: the composed cascade carries representative keys
        // from every bucket.
        let all_keys: std::collections::BTreeSet<&String> = cascade
            .rules
            .iter()
            .flat_map(|r| r.set.keys())
            .collect();
        assert!(all_keys.contains(&"printable_height".to_owned()),
                "printer-bucket key missing");
        assert!(all_keys.contains(&"nozzle_temperature".to_owned()),
                "filament-bucket key missing");
        assert!(all_keys.contains(&"layer_height".to_owned()),
                "process-bucket key missing");
    }

    #[test]
    fn compose_snappy_yields_u1_envelope() {
        let snappy = lookup_instance("snappy").expect("snappy present");
        let cascade = compose_cascade(&snappy, &BTreeMap::new()).expect("compose");
        let all_keys: std::collections::BTreeSet<&String> = cascade
            .rules
            .iter()
            .flat_map(|r| r.set.keys())
            .collect();
        // Same key set shape — different values, but the cascade
        // structure is identical.
        assert!(all_keys.contains(&"printable_height".to_owned()));
        assert!(all_keys.contains(&"nozzle_temperature".to_owned()));
        assert!(all_keys.contains(&"layer_height".to_owned()));
    }

    #[test]
    fn plate_overrides_appended_as_last_rule() {
        let bambi = lookup_instance("bambi").expect("bambi present");
        let mut overrides = BTreeMap::new();
        overrides.insert("layer_height".to_owned(), "0.12".to_owned());
        let cascade = compose_cascade(&bambi, &overrides).expect("compose");

        let last = cascade.rules.last().expect("at least one rule");
        assert_eq!(last.set.get("layer_height").map(String::as_str), Some("0.12"));
        assert_eq!(last.source.path.to_string_lossy(), "<plate-overrides>");
    }

    #[test]
    fn missing_printer_fragment_errors() {
        let mut bambi = lookup_instance("bambi").expect("bambi present");
        bambi.printer_fragment_slug = "nope".into();
        let err = compose_cascade(&bambi, &BTreeMap::new()).unwrap_err();
        assert_eq!(err, ComposeError::UnknownPrinterFragment("nope".into()));
    }
}
