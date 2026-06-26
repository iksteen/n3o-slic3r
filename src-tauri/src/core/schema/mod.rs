//! Typed Rust schema over libslic3r's option universe.
//!
//! Wraps `slic3r_ffi::option_defs()` into a richer, cached representation
//! that downstream Phase 1 modules consume:
//!
//! - **`cascade::loader`** reads the schema to validate predicate
//!   dimensions and `set.*` keys at load time, so cascades with typos fail
//!   with file:line errors rather than silently dropping settings at slice
//!   time.
//! - **`cascade_adapter`** expands the logical `bed_temp` key into the
//!   per-plate-type [`BED_TEMP_KEYS`] family (selected at slice time by
//!   `curr_bed_type`); other keys map 1:1.
//! - **`printer::profile`** cross-references schema keys when
//!   declaring which options a printer profile fixes vs leaves cascade-set.
//!
//! Cache is built lazily on first access. `slic3r_ffi::init()` must be
//! called before `load_schema()` — the Tauri app's `run()` does this in
//! its setup step, so any command-handler use is safe.
//!
//! See PRD §6.1 (FR-CAS-1..13) and `docs/dev/profiles.md` for the design.

pub mod capability;

pub use capability::{capability_for_key, CapabilityPredicate};

use slic3r_ffi::{option_defs, OptBucket, OptScope, OptType};
use std::collections::HashMap;
use std::sync::OnceLock;

/// A single libslic3r option, decorated with Phase-1-specific metadata.
///
/// This is the schema entry the resolver, adapter, and UI all consume.
/// Built from `slic3r_ffi::OptionDef`.
#[derive(Debug, Clone)]
pub struct OptionSchema {
    pub key: String,
    pub ty: OptType,
    pub scope: OptScope,
    /// Preset bucket (Printer / Filament / Process). `None` for metadata
    /// keys (`compatible_printers`, `inherits`) or non-FFF keys. The
    /// settings UI filters on this to keep printer/filament fields out of
    /// the process-bucket panel.
    pub bucket: Option<OptBucket>,
    /// True for any libslic3r vector option (Floats / Ints / Strings /
    /// Percents / FloatsOrPercents / Points / Bools / Enums). Derived
    /// from `ty.is_vector()`; surfaced here so callers don't need to
    /// import `OptType`'s method namespace just to check.
    pub is_vector: bool,
    pub label: Option<String>,
    pub category: Option<String>,
    /// `(enum_key, enum_label)` pairs, in declaration order. Empty for
    /// non-enum options.
    pub enum_values: Vec<(String, String)>,
    pub default_serialized: Option<String>,
}

/// True iff `key` is either a libslic3r option (per [`schema_by_key`])
/// or the cascade-side logical key `bed_temp` (which the adapter expands
/// into the per-plate-type [`BED_TEMP_KEYS`] family).
pub fn is_known_cascade_key(key: &str) -> bool {
    schema_by_key(key).is_some() || key == "bed_temp"
}

/// The 12 libslic3r keys in the `BedTempPerPlate` family, as declared
/// in `external/OrcaSlicer/src/libslic3r/PrintConfig.cpp` (all
/// `coInts`). 6 plate types × {steady, initial_layer}. Kept in sync
/// with the per-plate-type enum surface in libslic3r's `curr_bed_type`.
///
/// Note: some BBS-fork profile JSONs reference `smooth_plate_temp` /
/// `smooth_plate_temp_initial_layer` — those are OrcaSlicer-fork extras
/// not in libslic3r proper and end up in the cascade-adapter drop list
/// rather than this family.
pub const BED_TEMP_KEYS: &[&str] = &[
    "hot_plate_temp",
    "hot_plate_temp_initial_layer",
    "cool_plate_temp",
    "cool_plate_temp_initial_layer",
    "eng_plate_temp",
    "eng_plate_temp_initial_layer",
    "textured_plate_temp",
    "textured_plate_temp_initial_layer",
    "textured_cool_plate_temp",
    "textured_cool_plate_temp_initial_layer",
    "supertack_plate_temp",
    "supertack_plate_temp_initial_layer",
];

struct SchemaCache {
    options: Vec<OptionSchema>,
    by_key: HashMap<String, usize>,
}

static CACHE: OnceLock<SchemaCache> = OnceLock::new();

fn cache() -> &'static SchemaCache {
    CACHE.get_or_init(build_cache)
}

fn build_cache() -> SchemaCache {
    let defs = option_defs();
    let mut options = Vec::with_capacity(defs.len());
    let mut by_key = HashMap::with_capacity(defs.len());

    for (idx, def) in defs.into_iter().enumerate() {
        let enum_values = def
            .enum_values
            .iter()
            .cloned()
            .zip(
                def.enum_labels
                    .iter()
                    .cloned()
                    .chain(std::iter::repeat(String::new())),
            )
            .collect();
        let schema = OptionSchema {
            key: def.key.clone(),
            ty: def.ty,
            scope: def.scope,
            bucket: def.bucket,
            is_vector: def.ty.is_vector(),
            label: def.label,
            category: def.category,
            enum_values,
            default_serialized: def.default_serialized,
        };
        by_key.insert(def.key, idx);
        options.push(schema);
    }

    SchemaCache { options, by_key }
}

/// All registered libslic3r options as Phase-1 `OptionSchema` entries.
///
/// Stable order (matches `slic3r_ffi::option_defs()`). Safe to call from
/// any thread; cache initialization is `OnceLock`-guarded.
pub fn load_schema() -> &'static [OptionSchema] {
    &cache().options
}

/// Lookup a single option's schema by canonical key. Returns `None` for
/// keys libslic3r doesn't know about (typos, OrcaSlicer-only metadata).
pub fn schema_by_key(key: &str) -> Option<&'static OptionSchema> {
    cache().by_key.get(key).map(|&i| &cache().options[i])
}

/// Whether `key` can be set as a per-object override — i.e. libslic3r
/// honors it at object or region scope (`PrintObjectConfig` /
/// `PrintRegionConfig`). The single source of truth shared by the
/// slice-time gate (`object_overrides_for_slice`) and the Orca-import
/// reader; mirrors the frontend's `isObjectOverridable`. Unknown keys and
/// print/global-scope keys return `false` — libslic3r would ignore them
/// per object, so storing them as object overrides is an inert no-op.
pub fn is_object_overridable(key: &str) -> bool {
    schema_by_key(key).is_some_and(|s| s.scope.is_object() || s.scope.is_region())
}

/// Filter a per-object override map down to the keys libslic3r honors per
/// object (object/region scope), returned as a `BTreeMap` for deterministic
/// downstream ordering. The single gate shared by the slice-time builder and
/// the Orca-import reader so they keep exactly the same keys. Dropped keys
/// are logged by category against `object_label` (the object id):
///   - a real libslic3r option at the wrong scope → `warn` (the user likely
///     meant it; it just isn't per-object-settable),
///   - a key that isn't a libslic3r option at all → `debug` (expected:
///     foreign-3MF bookkeeping metadata like `matrix` / `source_*` / `module`
///     that the reader collects alongside real config).
pub fn gate_object_overrides<'a, I>(
    raw: I,
    object_label: u64,
) -> std::collections::BTreeMap<String, String>
where
    I: IntoIterator<Item = (&'a String, &'a String)>,
{
    let mut out = std::collections::BTreeMap::new();
    for (key, value) in raw {
        if is_object_overridable(key) {
            out.insert(key.clone(), value.clone());
        } else if schema_by_key(key).is_some() {
            tracing::warn!(
                object = object_label,
                key = %key,
                "dropping per-object override: not an object/region-scoped libslic3r option",
            );
        } else {
            tracing::debug!(
                object = object_label,
                key = %key,
                "ignoring non-option per-object metadata (foreign-3MF bookkeeping)",
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use slic3r_ffi::init;
    use std::sync::Once;

    static FFI_INIT: Once = Once::new();
    fn ensure_ffi() {
        FFI_INIT.call_once(|| {
            init(None, 3).expect("libslic3r init");
        });
    }

    #[test]
    fn count_matches_ffi() {
        ensure_ffi();
        let schema_count = load_schema().len();
        let ffi_count = option_defs().len();
        assert_eq!(
            schema_count, ffi_count,
            "schema must cover every libslic3r option (got {schema_count}, ffi has {ffi_count})"
        );
    }

    #[test]
    fn layer_height_is_scalar_float() {
        ensure_ffi();
        let s = schema_by_key("layer_height").expect("layer_height in schema");
        assert_eq!(s.ty, OptType::Float, "layer_height type");
        assert!(!s.is_vector, "layer_height is scalar");
        assert!(s.scope.is_object(), "layer_height is object-scoped");
    }

    #[test]
    fn nozzle_diameter_is_vector_float() {
        ensure_ffi();
        let s = schema_by_key("nozzle_diameter").expect("nozzle_diameter in schema");
        assert_eq!(s.ty, OptType::Floats, "nozzle_diameter type");
        assert!(s.is_vector, "nozzle_diameter is vector");
    }

    #[test]
    fn curr_bed_type_is_enum_with_expected_variants() {
        ensure_ffi();
        let s = schema_by_key("curr_bed_type").expect("curr_bed_type in schema");
        assert_eq!(s.ty, OptType::Enum, "curr_bed_type type");
        let keys: Vec<&str> = s.enum_values.iter().map(|(k, _)| k.as_str()).collect();
        // libslic3r's plate enum starts with these three; later variants
        // (SuperTack, Engineering, etc.) are present too.
        for expected in ["Cool Plate", "Textured PEI Plate", "Supertack Plate"] {
            assert!(
                keys.contains(&expected),
                "curr_bed_type missing variant {expected:?} (got {keys:?})"
            );
        }
    }

    #[test]
    fn hot_plate_temp_is_per_filament_ints_vector() {
        ensure_ffi();
        let s = schema_by_key("hot_plate_temp").expect("hot_plate_temp in schema");
        // Note: libslic3r declares the temperature families as coInts
        // (integer Celsius), not Floats. They're still vectors —
        // per-filament temperature.
        assert_eq!(s.ty, OptType::Ints, "hot_plate_temp is per-filament Ints");
        assert!(s.is_vector, "hot_plate_temp is vector");
    }

    #[test]
    fn all_bed_temp_keys_exist_in_schema() {
        ensure_ffi();
        for key in BED_TEMP_KEYS {
            schema_by_key(key).unwrap_or_else(|| {
                panic!("{key} not in libslic3r schema — BED_TEMP_KEYS out of sync")
            });
        }
    }

    #[test]
    fn schema_by_key_returns_none_for_typos() {
        ensure_ffi();
        assert!(
            schema_by_key("layer_hieght").is_none(),
            "typo lookup returns None"
        );
        assert!(schema_by_key("").is_none(), "empty key returns None");
    }
}
