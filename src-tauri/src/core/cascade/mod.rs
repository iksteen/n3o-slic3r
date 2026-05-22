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
//! Owns FR-CAS-1 through FR-CAS-13 (PRD §6.1). The submodules:
//!
//! - **`types`** (PR-1-2): the typed cascade IR — `Cascade`, `Rule`,
//!   `Predicate`, `SourceLocation`. Sharable across resolver, adapter,
//!   trace tooling, and the Tauri command surface.
//! - **`loader`** (PR-1-2): TOML parser that desugars the three
//!   authoring forms (top-level keys, `[section.shorthand]`, `[[rule]]`)
//!   into the IR and load-validates against the PR-1-1 schema.
//!
//! Resolver, override tiers, and trace tooling land in subsequent
//! PR-1-3..-5 work.

pub mod commands;
pub mod loader;
pub mod overrides;
pub mod resolver;
pub mod trace;
pub mod types;
pub mod validate;

pub use commands::{
    cascade_context_dimensions, cascade_load, cascade_resolve, cascade_trace, CascadeHandle,
    CascadeRegistry, ContextJson, OverrideFileSpec, ResolvedEntryJson, ResolvedJson,
};
pub use loader::{load_cascade, CascadeLoadError};
pub use overrides::{
    load_override_file, parse_override_str, resolve_with_overrides, FlatOverrides,
    OverrideTier, OverrideTiers, OverrideTraceEntry, ResolvedOverrides, ResolvedWithTrace,
};
pub use resolver::{
    format_when, resolve, Context, MapContext, MatchingRule, Resolved, ResolvedValue,
};
pub use trace::{trace, Trace, TraceRule, TraceSource};
pub use types::{Cascade, Condition, ConditionValue, Predicate, Rule, SourceLocation};
pub use validate::{default_known_dimensions, validate_cascade, KnownDimensions};

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
