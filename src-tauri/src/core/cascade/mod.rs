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
use slic3r_ffi::{option_defs, version, OptMode as FfiOptMode, OptScope as FfiOptScope};

use crate::core::printer::profile::PrinterProfile;
use crate::core::schema::{capability_for_key, CapabilityPredicate};

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

/// Wire-format mode for the FR-UI-2 Simple / Advanced / Expert
/// filter. Mirrors `slic3r_ffi::OptMode` but serializes as a stable
/// lowercase string so the TS side gets a tagged-string enum, not
/// a numeric variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OptMode {
    Simple,
    Advanced,
    Expert,
    Develop,
}

impl From<FfiOptMode> for OptMode {
    fn from(m: FfiOptMode) -> Self {
        match m {
            FfiOptMode::Simple => Self::Simple,
            FfiOptMode::Advanced => Self::Advanced,
            FfiOptMode::Expert => Self::Expert,
            FfiOptMode::Develop => Self::Develop,
        }
    }
}

/// Wire-format scope flags. The FFI's `OptScope` is a u32 bitmask
/// over libslic3r's config classes; the UI only cares about three
/// of them (project / object / region) for FR-3D-3's Object-tab
/// read-only gating. Serializes as a struct of bools so the
/// frontend doesn't need to know the underlying bit positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct OptScopeFlags {
    pub project: bool,
    pub object: bool,
    pub region: bool,
}

impl From<FfiOptScope> for OptScopeFlags {
    fn from(s: FfiOptScope) -> Self {
        Self {
            project: s.is_print(),
            object: s.is_object(),
            region: s.is_region(),
        }
    }
}

#[derive(Serialize)]
pub struct OptionSummary {
    pub key: String,
    pub ty: String,
    pub label: Option<String>,
    pub category: Option<String>,
    pub default_value: Option<String>,
    /// libslic3r tooltip text (FR-UI-6, PR-4-11's tooltip surface
    /// consumes this).
    pub tooltip: Option<String>,
    /// Simple / Advanced / Expert / Develop — drives the FR-UI-2
    /// mode filter in PR-4-3.
    pub mode: OptMode,
    /// Project / object / region scope bitmask — drives PR-4-9's
    /// Object-tab "project-scope setting" read-only badge.
    pub scope: OptScopeFlags,
    /// Printer-capability predicate that gates this option's
    /// visibility (PR-4-5 / FR-UI-7). `None` = always show. The
    /// generic `slicer_options` command always returns the
    /// predicate verbatim; `slicer_options_for_printer` returns
    /// the same data plus a pre-evaluated `hidden` flag against
    /// the supplied printer.
    pub capability: Option<CapabilityPredicate>,
}

fn summary_from_def(d: slic3r_ffi::OptionDef) -> OptionSummary {
    OptionSummary {
        capability: capability_for_key(&d.key),
        key: d.key,
        ty: format!("{:?}", d.ty),
        label: d.label,
        category: d.category,
        default_value: d.default_serialized,
        tooltip: d.tooltip,
        mode: d.mode.into(),
        scope: d.scope.into(),
    }
}

fn matches_filter(d: &slic3r_ffi::OptionDef, needle: &str) -> bool {
    needle.is_empty()
        || d.key.to_lowercase().contains(needle)
        || d.label.as_deref().is_some_and(|s| s.to_lowercase().contains(needle))
}

/// Settings-panel-visible options: Process bucket only (PR-S-2). Printer
/// + filament editing lives on other surfaces; metadata keys
/// (`compatible_printers`, `inherits`, …) and SLA-only keys have no
/// bucket and are also excluded.
fn is_panel_visible(d: &slic3r_ffi::OptionDef) -> bool {
    d.bucket == Some(slic3r_ffi::OptBucket::Process)
}

/// Filtered option introspection. The filter matches against the
/// canonical key and the display label, case-insensitively. Does
/// **not** evaluate capability predicates — callers that want a
/// printer-aware hide/show should use [`slicer_options_for_printer`].
#[tauri::command]
#[tracing::instrument]
pub fn slicer_options(filter: Option<String>) -> Vec<OptionSummary> {
    let needle = filter.unwrap_or_default().to_lowercase();
    let out: Vec<OptionSummary> = option_defs()
        .into_iter()
        .filter(is_panel_visible)
        .filter(|d| matches_filter(d, &needle))
        .map(summary_from_def)
        .collect();
    tracing::info!(matched = out.len(), "slicer_options");
    out
}

/// Per-option printer-aware view (PR-4-5 / FR-UI-7). Same shape as
/// [`OptionSummary`] plus a pre-evaluated `hidden` flag derived from
/// the option's [`CapabilityPredicate`] against the supplied
/// [`PrinterProfile`]. Frontend calls this once per printer-switch;
/// per-row capability evaluation in the hot render path is then a
/// single field read instead of a function call.
#[derive(Serialize)]
pub struct PrinterAwareOptionSummary {
    #[serde(flatten)]
    pub summary: OptionSummary,
    /// True when this option's capability predicate is **not**
    /// satisfied by the printer (i.e. the row should be hidden in
    /// `mode='hide'`, or badged "not applicable" in `mode='search'`).
    /// `false` when the predicate is satisfied OR when there's no
    /// predicate at all.
    pub hidden: bool,
}

/// Same shape as [`slicer_options`] but with each option's
/// capability predicate pre-evaluated against the supplied printer.
/// Filter behavior is identical to [`slicer_options`].
#[tauri::command]
#[tracing::instrument(skip(printer))]
pub fn slicer_options_for_printer(
    printer: PrinterProfile,
    filter: Option<String>,
) -> Vec<PrinterAwareOptionSummary> {
    let needle = filter.unwrap_or_default().to_lowercase();
    let out: Vec<PrinterAwareOptionSummary> = option_defs()
        .into_iter()
        .filter(is_panel_visible)
        .filter(|d| matches_filter(d, &needle))
        .map(|d| {
            let summary = summary_from_def(d);
            let hidden = summary
                .capability
                .map(|c| !c.satisfied_by(&printer))
                .unwrap_or(false);
            PrinterAwareOptionSummary { summary, hidden }
        })
        .collect();
    let hidden_count = out.iter().filter(|s| s.hidden).count();
    tracing::info!(
        matched = out.len(),
        hidden = hidden_count,
        printer = %printer.model,
        "slicer_options_for_printer",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::printer::profile::{BoundingBox, Toolhead};
    use slic3r_ffi::init as ffi_init;
    use std::sync::Once;

    static FFI: Once = Once::new();
    fn ensure_ffi() {
        FFI.call_once(|| {
            ffi_init(None, 3).expect("libslic3r init");
        });
    }

    fn a1_mini() -> PrinterProfile {
        PrinterProfile {
            model: "Bambu A1 mini".into(),
            slot_count: 4,
            supported_build_plates: vec!["Textured PEI".into()],
            toolheads: vec![Toolhead {
                nozzle_diameter: 0.4,
                hotend_type: "stainless_steel".into(),
                max_temp: 300.0,
                slot_indices: vec![0, 1, 2, 3],
            }],
            build_volume: BoundingBox {
                min: [0.0, 0.0, 0.0],
                max: [180.0, 180.0, 180.0],
            },
            exclusion_zones: vec![],
        }
    }

    fn synthetic_toolchanger() -> PrinterProfile {
        PrinterProfile {
            model: "Synthetic 2-toolhead".into(),
            slot_count: 2,
            supported_build_plates: vec!["Textured PEI".into()],
            toolheads: (0..2)
                .map(|i| Toolhead {
                    nozzle_diameter: 0.4,
                    hotend_type: "stainless_steel".into(),
                    max_temp: 300.0,
                    slot_indices: vec![i],
                })
                .collect(),
            build_volume: BoundingBox {
                min: [0.0, 0.0, 0.0],
                max: [200.0, 200.0, 200.0],
            },
            exclusion_zones: vec![],
        }
    }

    #[test]
    fn slicer_options_carries_mode_scope_tooltip_capability() {
        ensure_ffi();
        let opts = slicer_options(Some("layer_height".into()));
        // The schema is huge; layer_height is the canonical
        // representative the rest of the project keys off.
        let lh = opts
            .iter()
            .find(|o| o.key == "layer_height")
            .expect("layer_height present");
        assert_eq!(lh.mode, OptMode::Simple, "layer_height ships in Simple mode");
        // layer_height lives in PrintObjectConfig (per
        // external/OrcaSlicer/src/libslic3r/PrintConfig.hpp:936).
        assert!(lh.scope.object, "layer_height is an object-scope option");
        assert!(lh.tooltip.is_some(), "libslic3r ships a tooltip for layer_height");
        assert!(
            lh.capability.is_none(),
            "layer_height has no printer-capability gating",
        );

        // wall_filament is the canonical region-scope option per the
        // same PrintConfig.hpp.
        let wf_opts = slicer_options(Some("wall_filament".into()));
        let wf = wf_opts
            .iter()
            .find(|o| o.key == "wall_filament")
            .expect("wall_filament present");
        assert!(wf.scope.region, "wall_filament is a region-scope option");
    }

    #[test]
    fn a1_mini_shows_purge_tower_keys_via_printer_aware_view() {
        ensure_ffi();
        // PR-S-2 filters the panel to Process bucket only — printer-bucket
        // toolchanger geometry is no longer in scope here. The remaining
        // capability-gated process-bucket keys are the purge-tower /
        // prime-tower family, which AMS-style printers DO use.
        let opts = slicer_options_for_printer(a1_mini(), None);
        let purge = opts
            .iter()
            .find(|o| o.summary.key == "enable_prime_tower")
            .expect("enable_prime_tower present");
        assert_eq!(
            purge.summary.capability,
            Some(CapabilityPredicate::RequiresPurgeTower),
        );
        assert!(
            !purge.hidden,
            "purge-tower key should be visible on AMS-style A1 mini",
        );
    }

    #[test]
    fn synthetic_toolchanger_hides_purge_tower_keys() {
        ensure_ffi();
        let opts = slicer_options_for_printer(synthetic_toolchanger(), None);
        let purge = opts
            .iter()
            .find(|o| o.summary.key == "enable_prime_tower")
            .expect("enable_prime_tower present");
        assert!(purge.hidden, "purge-tower key should hide on toolchanger");
    }

    #[test]
    fn printer_aware_view_completes_within_render_budget() {
        ensure_ffi();
        // After PR-S-2 the panel surfaces Process-bucket options only
        // (~345 keys vs ~624 before bucket filtering). Per the FR-UI
        // 50 ms panel re-render budget, the full capability evaluation
        // pass must not dominate; debug allows 10× headroom.
        let start = std::time::Instant::now();
        let opts = slicer_options_for_printer(a1_mini(), None);
        let elapsed = start.elapsed();
        assert!(
            opts.len() >= 300,
            "expected ≥ 300 process-bucket options from libslic3r, got {}",
            opts.len(),
        );
        assert!(
            elapsed < std::time::Duration::from_millis(500),
            "slicer_options_for_printer took {elapsed:?} (debug budget 500 ms)",
        );
    }
}
