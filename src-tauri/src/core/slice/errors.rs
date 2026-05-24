//! Typed slice errors (PR-3-3 part 2).
//!
//! Wraps libslic3r's flat-string error returns with a typed
//! `SliceError` that names the offending setting where possible.
//! The orchestrator (PR-3-2) feeds the FFI's raw error message
//! through [`classify_libslic3r_error`] before emitting the
//! `slice:job_failed` event so the UI can render a specific
//! diagnostic instead of just "slice failed".
//!
//! Owns the FR-SL-3 deliverable. Per the ticket, classification is
//! table-driven so adding a new pattern is one line.
//!
//! The pattern catalog is sourced from `external/OrcaSlicer/src/
//! libslic3r/PrintConfig.cpp` + observed errors from PR-0.5-* spike
//! runs. When a new error shape shows up in practice, add a row.

use serde::{Deserialize, Serialize};

/// Typed view of a libslic3r-reported slice failure. The variant
/// names the specific failure mode where we can tell from the
/// raw message; `Unknown` carries the original bytes for the UI
/// to display verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data")]
pub enum SliceError {
    /// A configuration value rejected by libslic3r's validation
    /// pass. `setting_key` is the libslic3r option name when we
    /// could extract it; otherwise empty so the UI can still
    /// surface `reason`.
    InvalidConfig {
        setting_key: String,
        reason: String,
        raw_message: String,
    },
    /// Mesh geometry libslic3r couldn't slice (degenerate, empty,
    /// non-manifold beyond its repair threshold).
    InvalidGeometry {
        reason: String,
        raw_message: String,
    },
    /// Object positioned (partly) outside the printer's build
    /// volume. `plate_id` carries the plate where the offending
    /// object sits, when the orchestrator can provide it.
    OutOfBounds {
        plate_id: Option<u32>,
        raw_message: String,
    },
    /// User cancelled the slice via `slice_cancel` (PR-3-2).
    Cancelled,
    /// Cascade resolved to values the safety gate flagged as
    /// dangerous to send to a printer: missing machine_start_gcode,
    /// missing change_filament_gcode for an AMS-capable printer,
    /// nozzle temp above the bound printer's max_temp, etc. Each
    /// entry in `issues` is a human-readable description; the UI
    /// surfaces them on the slice panel + blocks the send button.
    UnsafeCascade { issues: Vec<String> },
    /// Anything we couldn't classify. UI surfaces the raw message
    /// in a "see logs" toast.
    Unknown { raw_message: String },
}

impl std::fmt::Display for SliceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidConfig {
                setting_key,
                reason,
                ..
            } => {
                if setting_key.is_empty() {
                    write!(f, "invalid config: {reason}")
                } else {
                    write!(f, "invalid config ({setting_key}): {reason}")
                }
            }
            Self::InvalidGeometry { reason, .. } => write!(f, "invalid geometry: {reason}"),
            Self::OutOfBounds { plate_id, .. } => {
                if let Some(p) = plate_id {
                    write!(f, "object out of bounds on plate {p}")
                } else {
                    write!(f, "object out of bounds")
                }
            }
            Self::Cancelled => write!(f, "slice cancelled"),
            Self::UnsafeCascade { issues } => {
                write!(
                    f,
                    "cascade safety gate refused the slice ({} issue{}): {}",
                    issues.len(),
                    if issues.len() == 1 { "" } else { "s" },
                    issues.join("; "),
                )
            }
            Self::Unknown { raw_message } => write!(f, "{raw_message}"),
        }
    }
}

impl std::error::Error for SliceError {}

/// Classify a raw libslic3r error string into a typed `SliceError`.
///
/// Pattern catalog is intentionally lenient on matching — libslic3r
/// emits the same logical error with multiple phrasings depending on
/// the validation path. We pattern-match on the most specific
/// keywords first, fall through to less specific, and finally land
/// in `Unknown` with the raw message preserved.
///
/// Documented sources for each pattern (left as inline comments
/// so new contributors can confirm a pattern still matches when
/// upstream changes):
///
/// - `external/OrcaSlicer/src/libslic3r/PrintConfig.cpp` — option
///   validation `assign_value`/`set` paths.
/// - `external/OrcaSlicer/src/libslic3r/Print.cpp` — pre-slice
///   geometry + plate validation.
/// - `examples/spike{1,2,3}/` — observed error strings from the
///   PR-0.5 spike runs.
pub fn classify_libslic3r_error(raw: &str) -> SliceError {
    let lower = raw.to_lowercase();

    // Cancellation propagates through libslic3r as a generic
    // `Cancelled` exception. PR-3-2's orchestrator can short-
    // circuit before this classifier runs, but if a worker
    // thread returns the message from the FFI we still want to
    // recognize it.
    if lower.contains("slicing was cancelled")
        || lower.contains("slicing cancelled")
        || lower == "cancelled"
    {
        return SliceError::Cancelled;
    }

    // Out-of-bounds: "object out of print area", "model outside
    // the print bed", etc. The bed check runs early and is the
    // most common slice-time failure for new users.
    if lower.contains("out of print area")
        || lower.contains("out of bed")
        || lower.contains("outside the print bed")
        || lower.contains("outside the printable area")
    {
        return SliceError::OutOfBounds {
            plate_id: extract_plate_id(raw),
            raw_message: raw.to_owned(),
        };
    }

    // Invalid geometry: non-manifold, empty mesh, degenerate
    // triangle count, repair-fail. These all map to a single
    // typed variant — the UI distinguishes by surfacing `reason`.
    for needle in [
        "no layers were detected",
        "no extrusions were generated",
        "empty mesh",
        "no facets",
        "is not a manifold",
        "the model is not manifold",
        "couldn't repair some non-manifold",
        "degenerate facets",
        "the model has",
    ] {
        if lower.contains(needle) {
            return SliceError::InvalidGeometry {
                reason: first_sentence(raw),
                raw_message: raw.to_owned(),
            };
        }
    }

    // Invalid config: libslic3r's validation messages typically
    // reference the setting by libslic3r option key. Patterns the
    // catalog covers:
    //
    //   "Option `xxx` is invalid: …"
    //   "Setting yyy: …"
    //   "The value provided for parameter "zzz" is …"
    //   "Invalid value for option …"
    if let Some(key) = extract_setting_key(raw) {
        return SliceError::InvalidConfig {
            setting_key: key,
            reason: first_sentence(raw),
            raw_message: raw.to_owned(),
        };
    }
    // Catch-all "invalid config" without an attributable setting.
    if lower.contains("invalid value")
        || lower.contains("invalid configuration")
        || lower.contains("validation failed")
        || lower.contains("must be")
    {
        return SliceError::InvalidConfig {
            setting_key: String::new(),
            reason: first_sentence(raw),
            raw_message: raw.to_owned(),
        };
    }

    SliceError::Unknown {
        raw_message: raw.to_owned(),
    }
}

/// Pull the libslic3r setting key out of an error string. Patterns:
///
/// - `Option \`fill_density\` is invalid: …` → `"fill_density"`
/// - `The value provided for parameter "fill_density" is …` →
///   `"fill_density"`
/// - `Setting fill_density: must be in [0, 100]` → `"fill_density"`
///
/// Returns `None` when no pattern matches.
fn extract_setting_key(raw: &str) -> Option<String> {
    // Backtick-quoted form: `key` between backticks.
    if let Some(start) = raw.find('`') {
        let after = &raw[start + 1..];
        if let Some(end) = after.find('`') {
            let key = after[..end].trim();
            if !key.is_empty() && key_is_plausible(key) {
                return Some(key.to_owned());
            }
        }
    }
    // Double-quoted form: "key" between straight double quotes.
    if let Some(start) = raw.find('"') {
        let after = &raw[start + 1..];
        if let Some(end) = after.find('"') {
            let key = after[..end].trim();
            if !key.is_empty() && key_is_plausible(key) {
                return Some(key.to_owned());
            }
        }
    }
    // `Setting <key>:` form — leading bare identifier after
    // "Setting".
    if let Some(rest) = lower_prefix(raw, "setting ") {
        let after = rest.trim_start();
        let end = after
            .find(|c: char| c == ':' || c.is_whitespace())
            .unwrap_or(after.len());
        let key = &after[..end];
        if key_is_plausible(key) {
            return Some(key.to_owned());
        }
    }
    None
}

/// `true` when `s` looks like a libslic3r option key — lowercase
/// snake-case, no spaces, reasonable length. The check is permissive
/// (we'd rather over-extract than miss a real key) but rejects the
/// most common false positives (whole error messages, sentence
/// fragments).
fn key_is_plausible(s: &str) -> bool {
    if s.is_empty() || s.len() > 80 {
        return false;
    }
    s.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn first_sentence(s: &str) -> String {
    let trimmed = s.trim();
    // Split on `:` first (libslic3r often emits `Key: reason`),
    // then take everything after; fall back to first period.
    if let Some(idx) = trimmed.find(':') {
        let after = trimmed[idx + 1..].trim();
        if !after.is_empty() {
            return after
                .split_terminator('.')
                .next()
                .unwrap_or(after)
                .trim()
                .to_owned();
        }
    }
    trimmed
        .split_terminator('.')
        .next()
        .unwrap_or(trimmed)
        .trim()
        .to_owned()
}

fn lower_prefix<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    if s.len() < prefix.len() {
        return None;
    }
    let head = &s[..prefix.len()];
    if head.eq_ignore_ascii_case(prefix) {
        Some(&s[prefix.len()..])
    } else {
        None
    }
}

fn extract_plate_id(raw: &str) -> Option<u32> {
    // `Plate 2: object out of …` style. Best-effort — most error
    // messages don't include this and the orchestrator overrides
    // with the plate it was slicing when it has the info.
    if let Some(rest) = lower_prefix(raw, "plate ") {
        let trimmed = rest.trim_start();
        let end = trimmed
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(trimmed.len());
        return trimmed[..end].parse().ok();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_invalid_config_with_backtick_key() {
        let raw = "Option `fill_density` is invalid: must be in range [0, 100]";
        let err = classify_libslic3r_error(raw);
        match err {
            SliceError::InvalidConfig {
                setting_key,
                reason,
                ..
            } => {
                assert_eq!(setting_key, "fill_density");
                assert!(reason.contains("must be"));
            }
            other => panic!("expected InvalidConfig, got {other:?}"),
        }
    }

    #[test]
    fn classifies_invalid_config_with_double_quoted_key() {
        let raw = "The value provided for parameter \"perimeter_speed\" is invalid";
        let err = classify_libslic3r_error(raw);
        match err {
            SliceError::InvalidConfig { setting_key, .. } => {
                assert_eq!(setting_key, "perimeter_speed");
            }
            other => panic!("expected InvalidConfig, got {other:?}"),
        }
    }

    #[test]
    fn classifies_invalid_config_with_setting_prefix() {
        let raw = "Setting fill_density: must be a positive number";
        let err = classify_libslic3r_error(raw);
        match err {
            SliceError::InvalidConfig { setting_key, .. } => {
                assert_eq!(setting_key, "fill_density");
            }
            other => panic!("expected InvalidConfig, got {other:?}"),
        }
    }

    #[test]
    fn classifies_invalid_geometry() {
        for raw in [
            "No layers were detected. Please check the configuration.",
            "The model is not manifold and cannot be sliced.",
            "empty mesh — nothing to slice",
        ] {
            let err = classify_libslic3r_error(raw);
            assert!(
                matches!(err, SliceError::InvalidGeometry { .. }),
                "expected InvalidGeometry for {raw:?}, got {err:?}"
            );
        }
    }

    #[test]
    fn classifies_out_of_bounds() {
        let raw = "Plate 2: object out of print area";
        let err = classify_libslic3r_error(raw);
        match err {
            SliceError::OutOfBounds { plate_id, .. } => {
                assert_eq!(plate_id, Some(2));
            }
            other => panic!("expected OutOfBounds, got {other:?}"),
        }
    }

    #[test]
    fn classifies_cancellation() {
        for raw in ["Slicing was cancelled", "slicing cancelled by user", "Cancelled"]
        {
            let err = classify_libslic3r_error(raw);
            assert_eq!(err, SliceError::Cancelled, "for input {raw:?}");
        }
    }

    #[test]
    fn classifies_unknown_when_nothing_matches() {
        let raw = "something completely unexpected from libslic3r";
        let err = classify_libslic3r_error(raw);
        match err {
            SliceError::Unknown { raw_message } => {
                assert_eq!(raw_message, raw);
            }
            other => panic!("expected Unknown, got {other:?}"),
        }
    }

    #[test]
    fn invalid_config_without_attributable_key_falls_through() {
        let raw = "Invalid configuration: extruders aren't homed";
        let err = classify_libslic3r_error(raw);
        match err {
            SliceError::InvalidConfig {
                setting_key,
                reason,
                ..
            } => {
                assert_eq!(setting_key, "");
                assert!(reason.contains("extruders"));
            }
            other => panic!("expected InvalidConfig (keyless), got {other:?}"),
        }
    }

    #[test]
    fn display_formats_typed_variants_readably() {
        let e = SliceError::InvalidConfig {
            setting_key: "fill_density".into(),
            reason: "must be in [0, 100]".into(),
            raw_message: "Option `fill_density` is invalid".into(),
        };
        assert_eq!(
            e.to_string(),
            "invalid config (fill_density): must be in [0, 100]"
        );
        let e = SliceError::Cancelled;
        assert_eq!(e.to_string(), "slice cancelled");
        let e = SliceError::OutOfBounds {
            plate_id: Some(3),
            raw_message: "".into(),
        };
        assert_eq!(e.to_string(), "object out of bounds on plate 3");
    }

    #[test]
    fn key_is_plausible_rejects_obvious_non_keys() {
        assert!(key_is_plausible("fill_density"));
        assert!(key_is_plausible("perimeter_speed"));
        assert!(!key_is_plausible(""));
        assert!(!key_is_plausible("CamelCase"));
        assert!(!key_is_plausible("has spaces"));
        assert!(!key_is_plausible("some, sentence"));
        // 81-char string — over the cap.
        assert!(!key_is_plausible(&"x".repeat(81)));
    }
}
