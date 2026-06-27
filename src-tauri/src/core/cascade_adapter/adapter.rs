//! `adapt()`: resolved logical config → `slic3r_ffi::Config`.
//!
//! Consumes the resolver's `Resolved` (or `ResolvedOverrides`) plus a
//! `Context`, writes every applicable key into a fresh
//! `slic3r_ffi::Config`. Returns the config + a list of per-key events
//! (skipped / expanded / parse-error) for diagnostic reporting.
//!
//! Transformations:
//!
//! - **Schema lookup.** Each key is set on the Config iff libslic3r's schema
//!   carries it. A key it doesn't carry is skipped (debug-logged, recorded as
//!   `AdaptEvent::UnknownKey`) — these are OrcaSlicer-fork extras the bundled
//!   cascades carry, or typos. Typos are normalized at *import*, not here
//!   (see `scripts/import_*.py`); the runtime adapter has no drop list or
//!   typo remap.
//! - **Dimensional expansion.** Logical `bed_temp` writes the same value into
//!   all 12 per-plate-type keys; `curr_bed_type` is set from the active
//!   context's plate. This is the simplified expansion; the "resolve per
//!   hypothetical plate context" form is a forward task documented in
//!   `docs/dev/profiles.md` and known limitations.

use crate::core::cascade::resolver::{Context, Resolved};
use crate::core::profile_library::split_for_key;
use crate::core::schema::{schema_by_key, BED_TEMP_KEYS};
use serde::Serialize;
use slic3r_ffi::{Config, ErrorKind, OptBucket};

/// Outcome of `adapt`. The Config itself is `Send`-but-not-trivially-
/// serializable; the list of skipped/expanded entries flows up for the
/// trace + Tauri command response.
pub struct AdaptResult {
    pub config: Config,
    pub events: Vec<AdaptEvent>,
}

/// Diagnostic events emitted during `adapt`. Surfaces skipped (unknown)
/// keys, dimensional expansions, and Config::set errors so the caller can
/// render a "X of Y resolved keys made it into libslic3r" summary.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind")]
pub enum AdaptEvent {
    /// Key isn't in the libslic3r schema, so it's skipped. Either an expected
    /// OrcaSlicer-fork extra (Bambu/Prusa firmware knobs the bundled cascades
    /// carry) or a typo (typos are folded at import; anything still unknown
    /// here is skipped, as libslic3r itself does).
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
    /// One or more per-filament vectors carry FEWER elements than the
    /// printer has filaments (`filament_diameter.size()` =
    /// `num_extruders`). libslic3r's MMU segmentation would index past the
    /// end and segfault. This is a config inconsistency — a stray override
    /// or a composer gap — surfaced rather than papered over with guessed
    /// values: the normal cascade fans every filament vector to the slot
    /// count, so this never fires for a well-formed config.
    FilamentVectorTooShort {
        /// `filament_diameter.size()` — the expected element count.
        expected: usize,
        /// `(key, found_len)` for each short vector, sorted by key.
        offenders: Vec<(String, usize)>,
    },
}

impl std::fmt::Display for AdaptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConfigAlloc(e) => write!(f, "slic3r_ffi::Config alloc failed: {e}"),
            Self::FilamentVectorTooShort {
                expected,
                offenders,
            } => {
                let list = offenders
                    .iter()
                    .map(|(k, n)| format!("{k} has {n}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(
                    f,
                    "filament setting(s) shorter than the printer's {expected} filament(s): \
                     {list}. This is an inconsistent filament configuration — most likely an \
                     override that doesn't match the printer's slot count.",
                )
            }
        }
    }
}

impl std::error::Error for AdaptError {}

/// Adapt a `Resolved` cascade output into a `slic3r_ffi::Config`.
pub fn adapt(
    resolved: &Resolved,
    ctx: &dyn Context,
) -> Result<AdaptResult, AdaptError> {
    // Invariant guard: every per-filament vector must carry exactly
    // `filament_diameter.size()` (= num_extruders) elements, or libslic3r's
    // MMU segmentation indexes out of bounds and segfaults. The normal
    // cascade fans them all to the slot count, so a short one means a real
    // inconsistency upstream — fail loudly with the offenders rather than
    // mask it with padded/guessed values.
    check_filament_vector_lengths(resolved)?;

    let mut config = Config::new().map_err(AdaptError::ConfigAlloc)?;
    let mut events: Vec<AdaptEvent> = Vec::new();

    // Phase 1: identity push (skipping keys not in the libslic3r schema),
    // while intercepting `bed_temp` for the dimensional expansion below.
    let mut bed_temp_value: Option<String> = None;
    for (key, rv) in resolved {
        if key == "bed_temp" {
            // Logical-only key — defer to dimensional expansion.
            bed_temp_value = Some(rv.value.clone());
            continue;
        }
        push_key(&mut config, key, &rv.value, &mut events);
    }

    // Phase 2: dimensional expansion of bed_temp + curr_bed_type.
    if let Some(value) = bed_temp_value {
        // Broadcast to all 12 BED_TEMP_KEYS — same value across plate
        // types. The production "resolve per hypothetical plate"
        // form is a follow-up (see docs/dev/profiles.md "Translating to
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

/// libslic3r's effective extruder count for the filament dimension is
/// `filament_diameter.size()` — that's literally what
/// `apply_mm_segmentation` reads as `num_extruders` and what every other
/// per-filament vector is indexed against. Anchoring to it (rather than a
/// composer-side slot count the adapter can't see) keeps the invariant in
/// libslic3r's own terms. `None` when `filament_diameter` is absent
/// (non-FFF, or nothing to anchor to → no normalization).
fn filament_vector_len(resolved: &Resolved) -> Option<usize> {
    let v = &resolved.get("filament_diameter")?.value;
    if v.is_empty() {
        return None;
    }
    Some(v.split(',').count())
}

/// A vector option libslic3r indexes per *filament* (0..num_filaments), so a
/// value shorter than the filament count OOB-reads in MMU segmentation.
///
/// The signal is the schema's Filament bucket + `is_vector` — derived, not a
/// curated list — with exactly one exception, `filament_colour`. It is a
/// genuine per-filament vector libslic3r reads from the *model*, but it's
/// commented out of `s_Preset_filament_options` upstream, so it belongs to no
/// preset and the FFI can't bucket it (there is no "per-filament" FFI flag
/// distinct from "per-physical-extruder", which would wrongly pull in
/// `nozzle_diameter` et al.). The composer special-cases it for the same
/// reason — see `assemble_filament_colours`.
fn is_per_filament_vector(key: &str, schema: &crate::core::schema::OptionSchema) -> bool {
    schema.is_vector
        && (matches!(schema.bucket, Some(OptBucket::Filament)) || key == "filament_colour")
}

/// Reject the resolved config if any [`is_per_filament_vector`] is SHORTER than
/// `filament_diameter` (libslic3r's `num_extruders`). A short vector makes
/// MMU segmentation read past its end — the segfault this guards against.
/// Element counts use the composer's cstyle-aware [`split_for_key`] so a `;`
/// inside a quoted string element (e.g. the `;`-comments in
/// `filament_start_gcode`) isn't miscounted.
///
/// Longer-than-`num_extruders` vectors are NOT an error: libslic3r ignores
/// the surplus filament indices (no OOB), so they pass through untouched.
/// `filament_diameter` absent → non-FFF, nothing to anchor to, no check.
fn check_filament_vector_lengths(resolved: &Resolved) -> Result<(), AdaptError> {
    let Some(expected) = filament_vector_len(resolved) else {
        return Ok(());
    };
    let mut offenders: Vec<(String, usize)> = Vec::new();
    for (key, rv) in resolved {
        let Some(schema) = schema_by_key(key) else {
            continue;
        };
        if is_per_filament_vector(key, schema) {
            let len = split_for_key(key, &rv.value).len();
            if len < expected {
                offenders.push((key.clone(), len));
            }
        }
    }
    if offenders.is_empty() {
        Ok(())
    } else {
        offenders.sort();
        Err(AdaptError::FilamentVectorTooShort {
            expected,
            offenders,
        })
    }
}

/// Push one key into the Config: schema lookup + `Config::set`. A key the
/// schema doesn't carry is skipped — it isn't a libslic3r option. Records an
/// event for every transformation.
fn push_key(
    config: &mut Config,
    key: &str,
    value: &str,
    events: &mut Vec<AdaptEvent>,
) {
    // Typo keys are folded to their canonical spelling at *import* time
    // (the importer scripts), so by the time data reaches the runtime
    // cascade it only ever carries canonical keys. A stray typo therefore
    // falls through to the schema check below and is dropped as an
    // unknown key — exactly what libslic3r/OrcaSlicer do with it. Runtime
    // remapping used to live here; it let a typo'd key in one filament
    // zero a *sibling* filament's value during per-filament vector
    // assembly (the typo minted a phantom vector that overwrote the real
    // one). Normalizing at import makes that structurally impossible.
    let effective_key = key.to_string();

    // A key libslic3r's schema doesn't carry can't be set — it's either an
    // expected OrcaSlicer-fork extra (the Bambu/Prusa firmware knobs the
    // bundled cascades carry) or a genuine typo. We don't distinguish the two
    // (nothing consumes the distinction): log at debug for diagnosis and skip.
    if schema_by_key(&effective_key).is_none() {
        tracing::debug!(key = %effective_key, "adapter: key not in libslic3r schema; skipping");
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
/// `docs/dev/profiles.md`. Unknown plate types pass through verbatim —
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
        let result = adapt(&resolved, &ctx_pei()).unwrap();
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
    fn fork_only_keys_are_skipped_as_unknown() {
        ensure_ffi();
        // OrcaSlicer-fork keys with no libslic3r equivalent are skipped (they
        // surface as UnknownKey events, discarded by the slice path) — they
        // never reach the engine config. Real keys pass through.
        let resolved = resolved_from([
            ("layer_height", "0.2"),
            ("hotend_cooling_rate", "2"),     // fork-only, not in schema
            ("filament_scarf_height", "0.1"), // fork-only, not in schema
        ]);
        let result = adapt(&resolved, &ctx_pei()).unwrap();
        let unknown: Vec<&str> = result
            .events
            .iter()
            .filter_map(|e| match e {
                AdaptEvent::UnknownKey { key } => Some(key.as_str()),
                _ => None,
            })
            .collect();
        assert!(unknown.contains(&"hotend_cooling_rate"));
        assert!(unknown.contains(&"filament_scarf_height"));
        // Neither fork key reached the engine config.
        assert!(result
            .config
            .get("hotend_cooling_rate")
            .unwrap_or_default()
            .is_empty());
        // A real key passes through.
        assert_eq!(result.config.get("layer_height").unwrap_or_default(), "0.2");
    }

    #[test]
    fn runtime_does_not_remap_typos_it_drops_them_as_unknown() {
        // Typo normalization moved to import time; the runtime adapter no
        // longer remaps. A typo that somehow reaches here is treated as an
        // unknown key and dropped (it does NOT silently become the
        // canonical key), matching libslic3r's own unknown-key handling.
        ensure_ffi();
        let resolved = resolved_from([("inital_layer_height", "0.3")]);
        let result = adapt(&resolved, &ctx_pei()).unwrap();
        assert_ne!(
            result
                .config
                .get("initial_layer_height")
                .unwrap_or_default(),
            "0.3",
            "the typo value must not be remapped onto the canonical key",
        );
        assert!(
            result.events.iter().any(|e| matches!(
                e,
                AdaptEvent::UnknownKey { key } if key == "inital_layer_height"
            )),
            "the typo should be surfaced as an unknown-key drop",
        );
    }

    #[test]
    fn bed_temp_expands_across_plate_keys() {
        ensure_ffi();
        let resolved = resolved_from([("bed_temp", "65")]);
        let result = adapt(&resolved, &ctx_pei()).unwrap();
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
        let result = adapt(&resolved, &ctx_pei()).unwrap();
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
        let result = adapt(&resolved, &ctx_pei()).unwrap();
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

    #[test]
    fn short_filament_vector_fails_the_adapt_rather_than_padding() {
        ensure_ffi();
        // filament_diameter fans to 4 (a 4-slot toolchanger); something
        // clobbered filament_colour down to 2. That's a real inconsistency
        // that would segfault libslic3r's MMU segmentation — surface it, don't
        // mask it with guessed values.
        let resolved = resolved_from([
            ("filament_diameter", "1.75,1.75,1.75,1.75"),
            ("filament_colour", "#FFFFFF;#DE4343"),
        ]);
        let err = match adapt(&resolved, &ctx_pei()) {
            Err(e) => e,
            Ok(_) => panic!("a short filament vector must fail the adapt"),
        };
        match &err {
            AdaptError::FilamentVectorTooShort {
                expected,
                offenders,
            } => {
                assert_eq!(*expected, 4);
                assert_eq!(*offenders, vec![("filament_colour".to_string(), 2)]);
            }
            other => panic!("expected FilamentVectorTooShort, got {other:?}"),
        }
        // The message names the offender + the expected count.
        let msg = err.to_string();
        assert!(msg.contains("filament_colour has 2"), "got {msg:?}");
        assert!(msg.contains('4'), "got {msg:?}");
    }

    #[test]
    fn longer_than_num_extruders_filament_vector_passes_through() {
        ensure_ffi();
        // A surplus filament index is harmless — libslic3r ignores it (no
        // OOB), so a longer-than-num_extruders vector is NOT an error.
        let resolved = resolved_from([
            ("filament_diameter", "1.75,1.75"),
            ("filament_colour", "#AAAAAA;#BBBBBB;#CCCCCC;#DDDDDD"),
        ]);
        let result = adapt(&resolved, &ctx_pei()).expect("longer is allowed");
        assert_eq!(
            result.config.get("filament_colour").unwrap_or_default(),
            "#AAAAAA;#BBBBBB;#CCCCCC;#DDDDDD",
        );
    }

    #[test]
    fn matching_length_filament_vectors_adapt_cleanly() {
        ensure_ffi();
        let resolved = resolved_from([
            ("filament_diameter", "1.75,1.75"),
            ("filament_colour", "#FFFFFF;#DE4343"),
        ]);
        let result = adapt(&resolved, &ctx_pei()).expect("matching lengths adapt");
        assert_eq!(
            result.config.get("filament_colour").unwrap_or_default(),
            "#FFFFFF;#DE4343",
        );
    }

    #[test]
    fn gcode_string_vector_with_embedded_semicolons_is_not_miscounted() {
        ensure_ffi();
        // filament_start_gcode is a `;`-joined string vector whose values
        // embed `;` (gcode comments). Counting must be cstyle-aware or this
        // single-element value reads as "short" and spuriously fails.
        let gcode = split_for_key(
            "filament_start_gcode",
            "\"; one\\nM104 S200 ; t\"", // one quoted element with inner `;`
        );
        assert_eq!(gcode.len(), 1, "embedded `;` must not split the element");
    }
}
