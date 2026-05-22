//! Rule cascade resolver.
//!
//! Loads TOML rule files, validates predicates and set keys against the
//! libslic3r schema, accepts a context object, and returns resolved
//! settings with trace metadata (winning rule's file:line and
//! specificity, list of also-matching losers, override-tier source).
//! Two-phase resolution per `docs/profiles.md`: authored cascade with
//! specificity-and-source-order, then `!important`-style user / project /
//! object override tiers.
//!
//! Owns FR-CAS-1 through FR-CAS-13 (PRD §6.1). For Phase 0, only the
//! option-introspection commands (which the resolver and the UI both
//! need) live here; the resolver itself is Phase 1 work.

use serde::Serialize;
use slic3r_ffi::{option_defs, version};

#[derive(Serialize)]
pub struct SlicerInfo {
    pub version: String,
    pub option_count: usize,
}

/// Banner: libslic3r version + count of options registered.
/// Drives the UI's connection-confirmation header.
#[tauri::command]
#[tracing::instrument]
pub fn slicer_info() -> SlicerInfo {
    let info = SlicerInfo {
        version: version(),
        option_count: option_defs().len(),
    };
    tracing::info!(version = %info.version, options = info.option_count, "slicer_info");
    info
}

#[derive(Serialize)]
pub struct OptionSummary {
    pub key: String,
    pub ty: String,
    pub label: Option<String>,
    pub category: Option<String>,
    pub default_value: Option<String>,
}

/// Filtered option introspection. The filter matches against the
/// canonical key and the display label, case-insensitively.
#[tauri::command]
#[tracing::instrument]
pub fn slicer_options(filter: Option<String>) -> Vec<OptionSummary> {
    let needle = filter.unwrap_or_default().to_lowercase();
    let out: Vec<OptionSummary> = option_defs()
        .into_iter()
        .filter(|d| {
            if needle.is_empty() {
                true
            } else {
                d.key.to_lowercase().contains(&needle)
                    || d.label.as_deref().is_some_and(|s| s.to_lowercase().contains(&needle))
            }
        })
        .map(|d| OptionSummary {
            key: d.key,
            ty: format!("{:?}", d.ty),
            label: d.label,
            category: d.category,
            default_value: d.default_serialized,
        })
        .collect();
    tracing::info!(matched = out.len(), "slicer_options");
    out
}
