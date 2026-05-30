//! `adapt()`: resolved logical config → `slic3r_ffi::Config`.
//!
//! Consumes the resolver's `Resolved` (or `ResolvedOverrides`) plus a
//! `Context` plus a `Manifest`, writes every applicable key into a
//! fresh `slic3r_ffi::Config`. Returns the config + a list of dropped
//! / remapped keys for diagnostic reporting.
//!
//! Three transformation steps:
//!
//! 1. **Typo remap.** Orca-side typos (PR-0.5-2 finding) silently
//!    rewritten to their canonical spelling.
//! 2. **Drop list.** OrcaSlicer-only keys (PR-0.5-1 + PR-0.5-2
//!    findings) silently discarded — they have no libslic3r meaning.
//! 3. **Dimensional expansion.** Logical `bed_temp` writes the same
//!    value into all 12 per-plate-type keys; `curr_bed_type` is set
//!    from the active context's plate. This is the simplified
//!    expansion from PR-0.5-1; the production "resolve per
//!    hypothetical plate context" form is a forward task documented
//!    in `docs/profiles.md` and known limitations.
//!
//! Unknown-but-not-dropped keys (typos the manifest doesn't know
//! about) surface as `AdaptDropEntry::UnknownKey` so the caller can
//! decide whether to fail or warn.

use super::manifest::Manifest;
use crate::core::cascade::resolver::{Context, Resolved};
use crate::core::cascade::ResolvedOverrides;
use crate::core::schema::{schema_by_key, BED_TEMP_KEYS};
use serde::Serialize;
use slic3r_ffi::{Config, ErrorKind};

/// Outcome of `adapt`. The Config itself is `Send`-but-not-trivially-
/// serializable; the manifest of dropped/remapped/skipped entries
/// flows up for the trace + Tauri command response.
pub struct AdaptResult {
    pub config: Config,
    pub events: Vec<AdaptEvent>,
}

/// Diagnostic events emitted during `adapt`. Surfaces remaps,
/// drops, unknowns, and Config::set errors so the caller can render
/// a "X of Y resolved keys made it into libslic3r" summary.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind")]
pub enum AdaptEvent {
    /// Orca typo silently remapped to its canonical spelling.
    Remapped { from: String, to: String },
    /// OrcaSlicer-only key dropped per the manifest.
    Dropped { key: String },
    /// Key isn't in the libslic3r schema *and* isn't in the manifest
    /// drop list. Likely a typo the manifest doesn't know about.
    UnknownKey { key: String },
    /// `Config::set` rejected the value (parse error). The value
    /// itself is captured in the event for trace + UI.
    ParseValueError {
        key: String,
        value: String,
        message: String,
    },
    /// Dimensional expansion fired for `bed_temp` → N libslic3r keys.
    /// Surfaces the expansion target list once per resolve.
    BedTempExpanded {
        value: String,
        targets: Vec<&'static str>,
    },
    /// `curr_bed_type` set from context.
    CurrBedTypeSet {
        plate_type: String,
        libslic3r_value: String,
    },
}

/// Adapt errors are deliberately rare — most issues become events
/// rather than hard failures. `ConfigAlloc` covers FFI memory
/// problems.
#[derive(Debug)]
pub enum AdaptError {
    ConfigAlloc(slic3r_ffi::Error),
}

impl std::fmt::Display for AdaptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConfigAlloc(e) => write!(f, "slic3r_ffi::Config alloc failed: {e}"),
        }
    }
}

impl std::error::Error for AdaptError {}

/// Adapt a `Resolved` cascade output into a `slic3r_ffi::Config`.
pub fn adapt(
    resolved: &Resolved,
    ctx: &dyn Context,
    manifest: &Manifest,
) -> Result<AdaptResult, AdaptError> {
    let mut config = Config::new().map_err(AdaptError::ConfigAlloc)?;
    let mut events: Vec<AdaptEvent> = Vec::new();

    // Phase 1: typo remap + drop list + identity push, while
    // intercepting `bed_temp` for the dimensional expansion below.
    let mut bed_temp_value: Option<String> = None;
    for (key, rv) in resolved {
        if key == "bed_temp" {
            // Logical-only key — defer to dimensional expansion.
            bed_temp_value = Some(rv.value.clone());
            continue;
        }
        push_key(&mut config, key, &rv.value, manifest, &mut events);
    }

    // Phase 2: dimensional expansion of bed_temp + curr_bed_type.
    if let Some(value) = bed_temp_value {
        // Broadcast to all 12 BED_TEMP_KEYS — same value across plate
        // types. The production "resolve per hypothetical plate"
        // form is a follow-up (see docs/profiles.md "Translating to
        // libslic3r → Dimensional expansion"); the broadcast is
        // libslic3r-correct because the active `curr_bed_type`
        // selects which key the engine actually reads.
        for plate_key in BED_TEMP_KEYS {
            if let Err(e) = config.set(plate_key, &value) {
                if e.kind != ErrorKind::UnknownKey {
                    events.push(AdaptEvent::ParseValueError {
                        key: (*plate_key).into(),
                        value: value.clone(),
                        message: e.to_string(),
                    });
                }
            }
        }
        events.push(AdaptEvent::BedTempExpanded {
            value: value.clone(),
            targets: BED_TEMP_KEYS.to_vec(),
        });
    }

    if let Some(plate_type) = ctx.predicate_value("plate.type") {
        let libslic3r_value = libslic3r_curr_bed_type(plate_type);
        let _ = config.set("curr_bed_type", &libslic3r_value);
        events.push(AdaptEvent::CurrBedTypeSet {
            plate_type: plate_type.to_string(),
            libslic3r_value,
        });
    }

    Ok(AdaptResult { config, events })
}

/// Convenience wrapper for the override-aware resolved map. Drops the
/// override-tier metadata and reuses the cascade branch — adapting
/// doesn't distinguish "cascade winner" from "override winner", only
/// the effective value matters.
pub fn adapt_with_overrides(
    resolved: &ResolvedOverrides,
    ctx: &dyn Context,
    manifest: &Manifest,
) -> Result<AdaptResult, AdaptError> {
    // Translate to the simpler Resolved shape by copying the effective
    // value as-if it came from the cascade.
    let cascade_view: Resolved = resolved
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                crate::core::cascade::ResolvedValue {
                    value: v.value.clone(),
                    winning_rule: v.winning_rule.clone(),
                    winning_specificity: v.winning_specificity,
                    matching_rules: v.matching_rules.clone(),
                },
            )
        })
        .collect();
    adapt(&cascade_view, ctx, manifest)
}

/// Push one key into the Config, applying typo-remap + drop-list +
/// schema lookup + Config::set. Logs events for every transformation.
fn push_key(
    config: &mut Config,
    key: &str,
    value: &str,
    manifest: &Manifest,
    events: &mut Vec<AdaptEvent>,
) {
    let effective_key: String = if let Some(canonical) = manifest.typo_remap(key) {
        events.push(AdaptEvent::Remapped {
            from: key.to_string(),
            to: canonical.to_string(),
        });
        canonical.to_string()
    } else {
        key.to_string()
    };

    if manifest.is_dropped(&effective_key) {
        events.push(AdaptEvent::Dropped { key: effective_key });
        return;
    }

    if schema_by_key(&effective_key).is_none() {
        events.push(AdaptEvent::UnknownKey { key: effective_key });
        return;
    }

    match config.set(&effective_key, value) {
        Ok(()) => {}
        Err(e) if e.kind == ErrorKind::UnknownKey => {
            // Schema said yes but libslic3r said no — schema desync.
            // Should be rare; surface as UnknownKey for diagnosis.
            events.push(AdaptEvent::UnknownKey { key: effective_key });
        }
        Err(e) => events.push(AdaptEvent::ParseValueError {
            key: effective_key,
            value: value.to_string(),
            message: e.to_string(),
        }),
    }
}

/// Translate our context's `plate.type` predicate value to libslic3r's
/// `curr_bed_type` enum value. Mapping mirrors the
/// `BuildPlate::libslic3r_curr_bed_type` design point from
/// `docs/profiles.md`. Unknown plate types pass through verbatim —
/// libslic3r will reject at slice time, which the trace tooling
/// surfaces.
fn libslic3r_curr_bed_type(plate_type: &str) -> String {
    match plate_type {
        "PEI" | "Textured PEI" => "Textured PEI Plate".to_string(),
        "Smooth PEI" => "Smooth PEI Plate".to_string(),
        "Cool" => "Cool Plate".to_string(),
        "Engineering" | "Eng" => "Engineering Plate".to_string(),
        "SuperTack" | "Supertack" => "Supertack Plate".to_string(),
        "Textured Cool" => "Textured Cool Plate".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::cascade::resolver::MapContext;
    use crate::core::cascade::resolver::ResolvedValue;
    use crate::core::cascade::types::SourceLocation;
    use slic3r_ffi::init;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::Once;

    static FFI_INIT: Once = Once::new();
    fn ensure_ffi() {
        FFI_INIT.call_once(|| {
            init(None, 3).expect("libslic3r init");
        });
    }

    fn rv(value: &str) -> ResolvedValue {
        ResolvedValue {
            value: value.to_string(),
            winning_rule: SourceLocation {
                path: PathBuf::from("test.toml"),
                line: 1,
            },
            winning_specificity: 0,
            matching_rules: Vec::new(),
        }
    }

    fn resolved_from<I, K, V>(iter: I) -> Resolved
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: AsRef<str>,
    {
        iter.into_iter()
            .map(|(k, v)| (k.into(), rv(v.as_ref())))
            .collect::<BTreeMap<_, _>>()
    }

    fn ctx_pei() -> MapContext {
        MapContext::with([("filament.type", "PLA"), ("plate.type", "Textured PEI")])
    }

    #[test]
    fn identity_keys_make_it_into_config() {
        ensure_ffi();
        let resolved = resolved_from([("layer_height", "0.2"), ("wall_loops", "2")]);
        let manifest = Manifest::build();
        let result = adapt(&resolved, &ctx_pei(), &manifest).unwrap();
        assert_eq!(result.config.get("layer_height").unwrap_or_default(), "0.2");
        assert_eq!(result.config.get("wall_loops").unwrap_or_default(), "2");
        // No unexpected events for these clean identity entries.
        assert!(
            result.events.iter().all(|e| matches!(
                e,
                AdaptEvent::CurrBedTypeSet { .. } | AdaptEvent::BedTempExpanded { .. }
            )),
            "unexpected events: {:#?}",
            result.events
        );
    }

    #[test]
    fn drop_list_keys_are_silently_filtered() {
        ensure_ffi();
        let resolved = resolved_from([
            ("layer_height", "0.2"),
            ("hotend_cooling_rate", "2"),     // dropped
            ("filament_scarf_height", "0.1"), // dropped
        ]);
        let manifest = Manifest::build();
        let result = adapt(&resolved, &ctx_pei(), &manifest).unwrap();
        let dropped: Vec<&str> = result
            .events
            .iter()
            .filter_map(|e| match e {
                AdaptEvent::Dropped { key } => Some(key.as_str()),
                _ => None,
            })
            .collect();
        assert!(dropped.contains(&"hotend_cooling_rate"));
        assert!(dropped.contains(&"filament_scarf_height"));
        // layer_height not in drop list — should have made it through.
        assert_eq!(result.config.get("layer_height").unwrap_or_default(), "0.2");
    }

    #[test]
    fn typo_remap_recovers_authors_intent() {
        ensure_ffi();
        let resolved = resolved_from([("inital_layer_height", "0.3")]);
        let manifest = Manifest::build();
        let result = adapt(&resolved, &ctx_pei(), &manifest).unwrap();
        let remapped: Vec<(&str, &str)> = result
            .events
            .iter()
            .filter_map(|e| match e {
                AdaptEvent::Remapped { from, to } => Some((from.as_str(), to.as_str())),
                _ => None,
            })
            .collect();
        assert_eq!(
            remapped,
            vec![("inital_layer_height", "initial_layer_height")]
        );
        assert_eq!(
            result
                .config
                .get("initial_layer_height")
                .unwrap_or_default(),
            "0.3"
        );
    }

    #[test]
    fn bed_temp_expands_across_plate_keys() {
        ensure_ffi();
        let resolved = resolved_from([("bed_temp", "65")]);
        let manifest = Manifest::build();
        let result = adapt(&resolved, &ctx_pei(), &manifest).unwrap();
        for plate_key in BED_TEMP_KEYS {
            assert_eq!(
                result.config.get(plate_key).unwrap_or_default(),
                "65",
                "{plate_key} should receive the broadcast"
            );
        }
        let expansion_event = result.events.iter().any(|e| {
            matches!(e,
            AdaptEvent::BedTempExpanded { value, .. } if value == "65")
        });
        assert!(expansion_event);
    }

    #[test]
    fn curr_bed_type_set_from_context() {
        ensure_ffi();
        let resolved = resolved_from([("layer_height", "0.2")]);
        let manifest = Manifest::build();
        let result = adapt(&resolved, &ctx_pei(), &manifest).unwrap();
        assert_eq!(
            result.config.get("curr_bed_type").unwrap_or_default(),
            "Textured PEI Plate"
        );
        let curr_bed_event = result.events.iter().any(|e| {
            matches!(e,
            AdaptEvent::CurrBedTypeSet { plate_type, libslic3r_value }
            if plate_type == "Textured PEI" && libslic3r_value == "Textured PEI Plate")
        });
        assert!(curr_bed_event);
    }

    #[test]
    fn unknown_key_surfaces_as_event() {
        ensure_ffi();
        let resolved = resolved_from([("totally_made_up_key", "x")]);
        let manifest = Manifest::build();
        let result = adapt(&resolved, &ctx_pei(), &manifest).unwrap();
        let unknown: Vec<&str> = result
            .events
            .iter()
            .filter_map(|e| match e {
                AdaptEvent::UnknownKey { key } => Some(key.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(unknown, vec!["totally_made_up_key"]);
    }
}
