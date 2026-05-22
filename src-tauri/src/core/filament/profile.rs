//! Filament profile descriptor.
//!
//! Phase 1 ships the minimum cascade-relevant fields: identity, base
//! type, vendor, color. Phase 7's filament-sync (PRD §6.8) extends
//! this with per-AMS-slot loaded state, temperature ranges, density,
//! etc.

use serde::{Deserialize, Serialize};

/// A loaded filament — what's in one slot of the active printer.
///
/// Surfaced in cascade predicates as:
/// - `filament.name`  → `identity` field
/// - `filament.type`  → `base_type` field (PLA, PETG, ABS, ...)
/// - `filament.color` (future) → `color` field
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilamentProfile {
    /// Specific identity, e.g. "Bambu PLA Basic Cyan".
    pub identity: String,
    /// Material family, e.g. "PLA", "PETG", "ABS". The most common
    /// predicate target ("when.filament.type = \"PLA\"").
    pub base_type: String,
    #[serde(default)]
    pub vendor: Option<String>,
    /// Hex color, e.g. "#C12E1F". Drives the renderer (Phase 2)
    /// and AMS slot display (Phase 7).
    #[serde(default)]
    pub color: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_json_with_optional_fields() {
        let f = FilamentProfile {
            identity: "Bambu PLA Basic Cyan".into(),
            base_type: "PLA".into(),
            vendor: Some("Bambu Lab".into()),
            color: Some("#0A2989".into()),
        };
        let j = serde_json::to_string(&f).expect("serialize");
        let parsed: FilamentProfile = serde_json::from_str(&j).expect("deserialize");
        assert_eq!(parsed.identity, "Bambu PLA Basic Cyan");
        assert_eq!(parsed.base_type, "PLA");
        assert_eq!(parsed.vendor.as_deref(), Some("Bambu Lab"));
    }

    #[test]
    fn minimal_filament_round_trips() {
        let f = FilamentProfile {
            identity: "Generic PLA".into(),
            base_type: "PLA".into(),
            vendor: None,
            color: None,
        };
        let j = serde_json::to_string(&f).expect("serialize");
        let parsed: FilamentProfile = serde_json::from_str(&j).expect("deserialize");
        assert!(parsed.vendor.is_none());
        assert!(parsed.color.is_none());
    }
}
