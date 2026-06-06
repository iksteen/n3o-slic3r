//! Schema-level validation pass over a parsed `Cascade`.
//!
//! Runs after `loader.rs` produces the IR. Surfaces three error
//! classes:
//!
//! - **Unknown set key** — `set.layer_hieght = "0.2"` doesn't match any
//!   libslic3r option. Includes a fuzzy suggestion when an edit-distance
//!   match is close.
//! - **Unknown predicate dimension** — `when.printr.model = "..."` uses
//!   a dotted dimension the active context layout doesn't declare.
//! - **Scope violation** — an option that's only meaningful at the
//!   object scope (e.g. `support_filament`) appears in a rule whose
//!   predicates can only constrain print-scope context.
//!
//! The validator is *advisory* — callers can choose to bypass it
//! (e.g. for UI live-editing where the user is mid-typing) and only
//! invoke at slice time. CLI loads always validate.
//!
//! The "unknown predicate dimension" check accepts a caller-supplied
//! list of valid dimensions. The real source of truth is the active
//! `Context`'s predicate-value key space, but the wire-up is
//! pending; [`default_known_dimensions`] is a stub fallback that
//! keeps tests + the validator usable in the meantime.

use super::types::{Cascade, ConditionValue};
use crate::core::cascade::loader::CascadeLoadError;
use crate::core::schema::{is_known_cascade_key, schema_by_key};

/// Predicate dimensions the cascade can use, scoped to a project's
/// active context layout. Tests + the default validator use
/// [`default_known_dimensions`] which covers the canonical set
/// (`printer.model`, `filament.type`, `plate.type`, etc.); the
/// real per-project dimension list will eventually be derived from
/// the active `Context`.
#[derive(Debug, Clone)]
pub struct KnownDimensions {
    pub dimensions: Vec<String>,
}

impl KnownDimensions {
    pub fn new<I, S>(iter: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            dimensions: iter.into_iter().map(Into::into).collect(),
        }
    }

    pub fn contains(&self, dim: &str) -> bool {
        self.dimensions.iter().any(|d| d == dim)
    }
}

/// Canonical predicate-dimension set the validator falls back to
/// when no caller-supplied list is passed. Matches the dotted keys
/// produced by the spike resolver: `printer.model`,
/// `filament.type`, `filament.name`, `plate.type`.
///
/// Stub: the real dimensions will be derived from the active
/// `Context`'s `predicate_value` key space once the wire-up
/// lands. The function name stays so callers don't churn.
pub fn default_known_dimensions() -> KnownDimensions {
    KnownDimensions::new([
        "printer.model",
        "filament.type",
        "filament.name",
        "plate.type",
    ])
}

/// Validate every rule in `cascade` against the libslic3r schema and
/// the supplied context dimensions. Returns all collected errors at
/// once so the caller can surface a complete list rather than
/// trickling one-at-a-time.
pub fn validate_cascade(
    cascade: &Cascade,
    known_dims: &KnownDimensions,
) -> Result<(), Vec<CascadeLoadError>> {
    let mut errors = Vec::new();

    for rule in &cascade.rules {
        for cond in &rule.when.conditions {
            if !known_dims.contains(&cond.dimension) {
                let suggestion = suggest_dimension(&cond.dimension, known_dims);
                let suggest_hint = suggestion
                    .map(|s| format!(" (did you mean `{s}`?)"))
                    .unwrap_or_default();
                errors.push(CascadeLoadError::InvalidShape {
                    location: rule.source.clone(),
                    message: format!(
                        "unknown predicate dimension `when.{}`{}",
                        cond.dimension, suggest_hint
                    ),
                });
            }
            // Sanity-check on the value shape: an empty array makes no
            // sense as set-membership and won't ever match anything.
            if let ConditionValue::Array(items) = &cond.value {
                if items.is_empty() {
                    errors.push(CascadeLoadError::InvalidShape {
                        location: rule.source.clone(),
                        message: format!(
                            "predicate `when.{}` has an empty array — \
                             use a scalar value or list at least one option",
                            cond.dimension
                        ),
                    });
                }
            }
        }

        for key in rule.set.keys() {
            if !is_known_cascade_key(key) {
                let suggestion = suggest_set_key(key);
                let suggest_hint = suggestion
                    .map(|s| format!(" (did you mean `{s}`?)"))
                    .unwrap_or_default();
                errors.push(CascadeLoadError::InvalidShape {
                    location: rule.source.clone(),
                    message: format!("unknown set key `set.{key}`{suggest_hint}"),
                });
            }
        }

        // Scope check: settings declared SLA-only in libslic3r are out
        // of scope for our FFF-only Phase 1 cascade. Object/region/print
        // scopes are all valid targets for any rule today (every rule
        // applies at the (filament, plate, object) intersection per
        // docs/dev/profiles.md), so the meaningful early check is just the
        // FFF-vs-SLA gate. Richer object/print scope distinctions can
        // land later as the override tiers gain more constraints.
        for key in rule.set.keys() {
            if let Some(schema) = schema_by_key(key) {
                if schema.scope.0 != 0 && schema.scope.is_sla() && !schema.scope.is_fff() {
                    errors.push(CascadeLoadError::InvalidShape {
                        location: rule.source.clone(),
                        message: format!(
                            "set key `set.{key}` is SLA-only \
                             (libslic3r scope bitmask {:#x}) — \
                             not meaningful for FFF cascades",
                            schema.scope.0
                        ),
                    });
                }
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Suggest a near-neighbor for an unknown set key. Returns `Some(key)`
/// when an existing schema key is within edit distance 2 of the
/// offending key; `None` if nothing close was found.
fn suggest_set_key(unknown: &str) -> Option<&'static str> {
    let schema = crate::core::schema::load_schema();
    let mut best: Option<(&'static str, usize)> = None;
    for entry in schema {
        let d = edit_distance(unknown, &entry.key);
        if d <= 2 && best.is_none_or(|(_, bd)| d < bd) {
            best = Some((entry.key.as_str(), d));
        }
    }
    best.map(|(k, _)| k)
}

/// Suggest a near-neighbor for an unknown predicate dimension. Same
/// edit-distance threshold (≤2) as `suggest_set_key`.
fn suggest_dimension<'a>(unknown: &str, dims: &'a KnownDimensions) -> Option<&'a str> {
    let mut best: Option<(&'a str, usize)> = None;
    for d in &dims.dimensions {
        let dist = edit_distance(unknown, d);
        if dist <= 2 && best.is_none_or(|(_, bd)| dist < bd) {
            best = Some((d.as_str(), dist));
        }
    }
    best.map(|(d, _)| d)
}

/// Plain Levenshtein on bytes. Adequate for short ASCII identifiers
/// like libslic3r option names. Memory: O(min(a, b)).
fn edit_distance(a: &str, b: &str) -> usize {
    let a_bytes = a.as_bytes();
    let b_bytes = b.as_bytes();
    let (a_bytes, b_bytes) = if a_bytes.len() < b_bytes.len() {
        (b_bytes, a_bytes)
    } else {
        (a_bytes, b_bytes)
    };
    let n = a_bytes.len();
    let m = b_bytes.len();
    if m == 0 {
        return n;
    }
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut curr = vec![0usize; m + 1];
    for i in 1..=n {
        curr[0] = i;
        for j in 1..=m {
            let cost = if a_bytes[i - 1] == b_bytes[j - 1] {
                0
            } else {
                1
            };
            curr[j] = (curr[j - 1] + 1).min(prev[j] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[m]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::cascade::loader::parse_cascade_str;
    use slic3r_ffi::init;
    use std::path::Path;
    use std::sync::Once;

    static FFI_INIT: Once = Once::new();
    fn ensure_ffi() {
        FFI_INIT.call_once(|| {
            init(None, 3).expect("libslic3r init");
        });
    }

    fn parse_and_validate(src: &str) -> Result<(), Vec<CascadeLoadError>> {
        ensure_ffi();
        let rules = parse_cascade_str(src, Path::new("test.toml")).expect("parse");
        let cascade = Cascade { rules };
        validate_cascade(&cascade, &default_known_dimensions())
    }

    #[test]
    fn canonical_cascade_validates() {
        let src = "\
layer_height = 0.2
nozzle_diameter = [\"0.4\"]

[[rule]]
when.filament.type = \"PLA\"
set.bed_temperature_formula = \"by_first_filament\"
";
        parse_and_validate(src).expect("canonical content passes validation");
    }

    #[test]
    fn unknown_set_key_is_caught_with_suggestion() {
        ensure_ffi();
        let src = "layer_hieght = 0.2\n";
        let errs = parse_and_validate(src).expect_err("typo should be caught");
        assert_eq!(errs.len(), 1);
        match &errs[0] {
            CascadeLoadError::InvalidShape { message, .. } => {
                assert!(message.contains("layer_hieght"), "names the typo");
                assert!(
                    message.contains("layer_height"),
                    "suggests the correct key: {message}"
                );
            }
            other => panic!("unexpected error {other:?}"),
        }
    }

    #[test]
    fn unknown_predicate_dimension_is_caught_with_suggestion() {
        let src = "[[rule]]\nwhen.filament.tipe = \"PLA\"\nset.layer_height = 0.2\n";
        let errs = parse_and_validate(src).expect_err("typo dim should be caught");
        let predicate_err = errs.iter().find(|e| match e {
            CascadeLoadError::InvalidShape { message, .. } => message.contains("predicate"),
            _ => false,
        });
        let predicate_err = predicate_err.expect("predicate-dimension error present");
        match predicate_err {
            CascadeLoadError::InvalidShape { message, .. } => {
                assert!(
                    message.contains("filament.tipe"),
                    "names the unknown dim: {message}"
                );
                assert!(
                    message.contains("filament.type"),
                    "suggests the corrected dim: {message}"
                );
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn empty_predicate_array_is_caught() {
        let src = "[[rule]]\nwhen.filament.type = []\nset.layer_height = 0.2\n";
        let errs = parse_and_validate(src).expect_err("empty array should be flagged");
        let array_err = errs.iter().find(|e| {
            matches!(e,
            CascadeLoadError::InvalidShape { message, .. } if message.contains("empty array"))
        });
        assert!(array_err.is_some(), "empty-array predicate is flagged");
    }

    #[test]
    fn multiple_errors_collected_at_once() {
        ensure_ffi();
        let src = "\
[[rule]]
when.filament.tipe = \"PLA\"
set.layer_hieght = 0.2
set.nozzl_diameter = \"0.4\"
";
        let errs = parse_and_validate(src).expect_err("multi-error should bubble");
        // 1 predicate + 2 set-key errors = 3
        assert_eq!(errs.len(), 3, "collected all 3 errors: {errs:#?}");
    }

    #[test]
    fn edit_distance_simple_cases() {
        assert_eq!(edit_distance("", ""), 0);
        assert_eq!(edit_distance("a", ""), 1);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("layer_height", "layer_hieght"), 2);
    }
}
