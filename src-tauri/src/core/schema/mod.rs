//! Typed Rust schema over libslic3r's option universe.
//!
//! Wraps `slic3r_ffi::option_defs()` into a richer, cached representation
//! that downstream Phase 1 modules consume:
//!
//! - **`cascade::loader`** (PR-1-2) reads the schema to validate predicate
//!   dimensions and `set.*` keys at load time, so cascades with typos fail
//!   with file:line errors rather than silently dropping settings at slice
//!   time.
//! - **`cascade_adapter`** (PR-1-6) reads `dimensional` to decide whether a
//!   resolved logical key needs expansion across a dimension (bed temp →
//!   14 per-plate-type keys) or maps 1:1.
//! - **`printer::profile`** (PR-1-7) cross-references schema keys when
//!   declaring which options a printer profile fixes vs leaves cascade-set.
//!
//! Cache is built lazily on first access. `slic3r_ffi::init()` must be
//! called before `load_schema()` — the Tauri app's `run()` does this in
//! its setup step, so any command-handler use is safe.
//!
//! See PRD §6.1 (FR-CAS-1..13) and `docs/profiles.md` for the design.

pub mod capability;

pub use capability::{capability_for_key, CapabilityPredicate};

use slic3r_ffi::{option_defs, OptScope, OptType};
use std::collections::HashMap;
use std::sync::OnceLock;

/// A single libslic3r option, decorated with Phase-1-specific metadata.
///
/// This is the schema entry the resolver, adapter, and UI all consume.
/// Built from `slic3r_ffi::OptionDef` plus a static binding table that
/// marks dimensional axes (PR-1-6 territory).
#[derive(Debug, Clone)]
pub struct OptionSchema {
    pub key: String,
    pub ty: OptType,
    pub scope: OptScope,
    /// True for any libslic3r vector option (Floats / Ints / Strings /
    /// Percents / FloatsOrPercents / Points / Bools / Enums). Derived
    /// from `ty.is_vector()`; surfaced here so callers don't need to
    /// import `OptType`'s method namespace just to check.
    pub is_vector: bool,
    /// Non-`None` when this option is part of a dimensional expansion the
    /// adapter performs (e.g. `hot_plate_temp` is one of the
    /// `BedTempPerPlate` family). The adapter consults this to decide
    /// whether to apply expansion or write the value verbatim.
    pub dimensional: Option<DimensionalKind>,
    pub label: Option<String>,
    pub category: Option<String>,
    /// `(enum_key, enum_label)` pairs, in declaration order. Empty for
    /// non-enum options.
    pub enum_values: Vec<(String, String)>,
    pub default_serialized: Option<String>,
}

/// Marker for an option's membership in one of libslic3r's dimensional
/// expansion families.
///
/// A "dimensional" option is one where the cascade carries a *single*
/// logical key (e.g. `bed_temp`) but libslic3r consumes a *family* of
/// keys (one per dimension value). The adapter resolves the cascade
/// against each dimension value and writes the corresponding libslic3r
/// key. See `docs/profiles.md` "Translating to libslic3r → Dimensional
/// expansion" for the worked bed-temp example.
///
/// Variants are added as the adapter (PR-1-6) surfaces them. Start small
/// — every variant means more adapter code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DimensionalKind {
    /// Per-plate-type bed temperature, selected at slice time by
    /// `curr_bed_type`. 14 libslic3r keys (7 plate types × {steady,
    /// initial_layer}). Cascade authors a single logical `bed_temp` per
    /// `(filament, plate)` context; the adapter expands across plates.
    BedTempPerPlate,
}

/// Cascade-side logical keys — names the *resolver* understands that
/// the *libslic3r option universe* doesn't. The cascade adapter
/// (PR-1-6) expands each logical key into one or more libslic3r keys.
///
/// Currently just `bed_temp` → `BedTempPerPlate` family. Validators
/// (PR-1-2) check against this list as a sibling to `schema_by_key`
/// so cascades that author `set.bed_temp = "65"` pass validation
/// while typos like `set.bd_temp` still fail.
pub const LOGICAL_KEYS: &[&str] = &["bed_temp"];

/// True iff `key` is either a libslic3r option (per
/// [`schema_by_key`]) or a recognized cascade-side logical key (per
/// [`LOGICAL_KEYS`]).
pub fn is_known_cascade_key(key: &str) -> bool {
    schema_by_key(key).is_some() || LOGICAL_KEYS.contains(&key)
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
    CACHE.get_or_init(|| build_cache())
}

fn build_cache() -> SchemaCache {
    let defs = option_defs();
    let mut options = Vec::with_capacity(defs.len());
    let mut by_key = HashMap::with_capacity(defs.len());

    for (idx, def) in defs.into_iter().enumerate() {
        let dimensional = dimensional_for_key(&def.key);
        let enum_values = def
            .enum_values
            .iter()
            .cloned()
            .zip(def.enum_labels.iter().cloned().chain(std::iter::repeat(String::new())))
            .collect();
        let schema = OptionSchema {
            key: def.key.clone(),
            ty: def.ty,
            scope: def.scope,
            is_vector: def.ty.is_vector(),
            dimensional,
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

fn dimensional_for_key(key: &str) -> Option<DimensionalKind> {
    if BED_TEMP_KEYS.contains(&key) {
        Some(DimensionalKind::BedTempPerPlate)
    } else {
        None
    }
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
        assert!(s.dimensional.is_none(), "layer_height is not dimensional");
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
    fn hot_plate_temp_is_bed_temp_dimensional_vector() {
        ensure_ffi();
        let s = schema_by_key("hot_plate_temp").expect("hot_plate_temp in schema");
        // Note: libslic3r declares the temperature families as coInts
        // (integer Celsius), not Floats. They're still vectors —
        // per-filament temperature.
        assert_eq!(s.ty, OptType::Ints, "hot_plate_temp is per-filament Ints");
        assert!(s.is_vector, "hot_plate_temp is vector");
        assert_eq!(
            s.dimensional,
            Some(DimensionalKind::BedTempPerPlate),
            "hot_plate_temp must be tagged BedTempPerPlate"
        );
    }

    #[test]
    fn all_bed_temp_keys_are_tagged_dimensional() {
        ensure_ffi();
        for key in BED_TEMP_KEYS {
            let s = schema_by_key(key)
                .unwrap_or_else(|| panic!("{key} not in libslic3r schema — BED_TEMP_KEYS out of sync"));
            assert_eq!(
                s.dimensional,
                Some(DimensionalKind::BedTempPerPlate),
                "{key} must be tagged BedTempPerPlate"
            );
        }
    }

    #[test]
    fn schema_by_key_returns_none_for_typos() {
        ensure_ffi();
        assert!(schema_by_key("layer_hieght").is_none(), "typo lookup returns None");
        assert!(schema_by_key("").is_none(), "empty key returns None");
    }
}
