//! OrcaSlicer / Bambu Studio `.3mf` **project** import.
//!
//! Reconstructs an n3o project from a foreign project file. Geometry and
//! per-object/plate structure come from the existing 3MF reader +
//! [`crate::core::threemf::bbs_meta`] (`model_settings.config`); this
//! module owns the **settings** half — `Metadata/project_settings.config`,
//! the flattened, resolved libslic3r config the project was authored with
//! (a JSON object of key → string | list-of-string).
//!
//! ## Key handling
//!
//! Each settings key is classified by its libslic3r bucket
//! ([`slic3r_ffi::bucket_of`] — the authoritative signal, not a curated
//! list):
//!
//! - **Process** + **Filament** keys are imported through the
//!   delta-vs-our-baseline path (a later step): resolve our own cascade
//!   for the bound printer/filament/process and keep only the keys where
//!   the foreign value differs, as project overrides.
//! - **Printer** (machine) keys are **dropped**. Machine config — bed
//!   shape, start/end G-code, kinematics, limits — is owned by the bound
//!   `PrinterInstance`, never adopted from a foreign project. This is
//!   load-bearing on a *fallback* bind (the project's printer isn't one
//!   we ship): adopting its machine settings onto our printer would be
//!   wrong and potentially unsafe.
//! - Keys with **no bucket** are identity/metadata — consumed as
//!   identity where we recognize them, otherwise reported as unmapped.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use serde_json::Value;
use slic3r_ffi::{bucket_of, OptBucket};

use crate::core::filament::FilamentProfile;
use crate::core::printer::{list_instances, lookup, PrinterInstance};
use crate::core::project::context::SlicingContext;
use crate::core::project::model::Project;
use crate::core::scene::build_plate::{self, BuildPlate};
use crate::core::scene::state::{MeshId, NewSceneObject};
use crate::core::scene::transform::Transform;
use crate::core::threemf::{load_3mf, ProjectObject};

/// The parsed `project_settings.config`: every key as-authored, plus
/// typed access to the identity keys the importer binds against.
#[derive(Debug, Clone)]
pub struct OrcaProjectSettings {
    /// Every key from the config; value is libslic3r's string or
    /// list-of-string JSON shape.
    pub settings: BTreeMap<String, Value>,
}

impl OrcaProjectSettings {
    /// Parse the raw `project_settings.config` bytes (a JSON object).
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let v: Value =
            serde_json::from_slice(bytes).map_err(|e| format!("project_settings.config: {e}"))?;
        let obj = v
            .as_object()
            .ok_or_else(|| "project_settings.config: not a JSON object".to_string())?;
        let settings = obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        Ok(Self { settings })
    }

    /// A scalar string: the JSON string, or the sole element of a
    /// one-element list (BBS authors some scalars as 1-lists).
    pub fn scalar(&self, key: &str) -> Option<String> {
        match self.settings.get(key)? {
            Value::String(s) => Some(s.clone()),
            Value::Array(a) if a.len() == 1 => a[0].as_str().map(str::to_owned),
            _ => None,
        }
    }

    /// A list value, each element stringified (a bare scalar reads as a
    /// 1-list).
    pub fn list(&self, key: &str) -> Option<Vec<String>> {
        match self.settings.get(key)? {
            Value::Array(a) => Some(
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_owned))
                    .collect(),
            ),
            Value::String(s) => Some(vec![s.clone()]),
            _ => None,
        }
    }

    /// Canonical string form for comparison + override storage: a scalar
    /// string, or a list joined with the libslic3r separator for the key's
    /// option type — `;` for string vectors (`filament_colour`,
    /// `filament_type`, …), `,` otherwise — via the same
    /// [`join_for_key`](crate::core::profile_library::composer::join_for_key)
    /// the composer uses. Getting this right is load-bearing: a comma-joined
    /// `filament_colour` parses as a *single* color in libslic3r, which then
    /// underflows the MMU color-painting segmentation (it sizes per-color
    /// arrays from `filament_colour.size()`) and crashes the slice.
    pub fn canonical(&self, key: &str) -> Option<String> {
        Some(match self.settings.get(key)? {
            Value::String(s) => s.clone(),
            Value::Array(a) => {
                let parts: Vec<String> = a
                    .iter()
                    .map(|x| match x {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    })
                    .collect();
                crate::core::profile_library::composer::join_for_key(key, &parts)
            }
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Null | Value::Object(_) => return None,
        })
    }

    // --- identity keys the importer binds against (not slice settings) ---

    /// The printer this project targets, e.g. "Bambu Lab A1 mini". The
    /// importer maps it to a bundled `PrinterInstance`, or falls back.
    pub fn printer_model(&self) -> Option<String> {
        self.scalar("printer_model")
    }

    /// The active build plate, e.g. "Supertack Plate".
    pub fn curr_bed_type(&self) -> Option<String> {
        self.scalar("curr_bed_type")
    }

    /// The process preset name, e.g. "0.20mm Standard @BBL A1M".
    pub fn print_settings_id(&self) -> Option<String> {
        self.scalar("print_settings_id")
    }

    /// Per-slot filament preset names, e.g. ["Bambu PLA Basic @BBL A1M", …].
    pub fn filament_settings_ids(&self) -> Vec<String> {
        self.list("filament_settings_id").unwrap_or_default()
    }

    /// The keys the project marks as changed from its system preset
    /// (`different_settings_to_system`). BBS stores this as a list whose
    /// slots are `;`-joined key strings, one per config category (process,
    /// then per-filament, …); we flatten the union across all slots.
    ///
    /// Returns:
    /// - `None` when the key is **absent** (or `null`): we can't tell what
    ///   the user changed, so the caller imports the full delta-vs-baseline.
    /// - `Some(set)` when **present** — even an empty set (every slot blank):
    ///   the project declares it changed *nothing*, so the caller imports no
    ///   overrides (just geometry + the adopted process).
    pub fn changed_from_system(&self) -> Option<std::collections::HashSet<String>> {
        let split = |s: &str| -> Vec<String> {
            s.split(';')
                .map(str::trim)
                .filter(|k| !k.is_empty())
                .map(str::to_owned)
                .collect()
        };
        match self.settings.get("different_settings_to_system")? {
            Value::Array(a) => Some(a.iter().filter_map(Value::as_str).flat_map(split).collect()),
            Value::String(s) => Some(split(s).into_iter().collect()),
            // null / unexpected shape → treat as absent (full import).
            _ => None,
        }
    }
}

/// Where each settings key lands when importing.
#[derive(Debug, Default)]
pub struct KeyPartition {
    /// Process-bucket keys → candidate project overrides (delta path).
    pub process: Vec<String>,
    /// Filament-bucket keys — **dropped** (owned by the bound slot; a
    /// material's filament identity comes from the slot it binds to, not the
    /// foreign project). Tracked for the import report's accounting.
    pub filament: Vec<String>,
    /// Printer/machine keys — **dropped** (owned by the bound printer).
    pub machine: Vec<String>,
    /// Keys with no libslic3r bucket that we don't consume as identity —
    /// reported as not-imported.
    pub unmapped: Vec<String>,
}

/// Keys we consume as identity / binding inputs (or pure file metadata),
/// so they're not double-counted as dropped or unmapped settings. These
/// are import-orchestration concerns, not libslic3r classifications.
const IDENTITY_KEYS: &[&str] = &[
    "printer_model",
    "printer_settings_id",
    "printer_variant",
    "print_settings_id",
    "filament_settings_id",
    "filament_ids",
    "different_settings_to_system",
    "version",
    "from",
    "name",
    "setting_id",
    // Consumed for bed selection (curr_bed_type()), not a slice override.
    "curr_bed_type",
];

/// Classify every key in `settings` by libslic3r bucket.
pub fn partition(settings: &OrcaProjectSettings) -> KeyPartition {
    let mut p = KeyPartition::default();
    for key in settings.settings.keys() {
        if IDENTITY_KEYS.contains(&key.as_str()) {
            continue;
        }
        match bucket_of(key) {
            Some(OptBucket::Process) => p.process.push(key.clone()),
            Some(OptBucket::Filament) => p.filament.push(key.clone()),
            Some(OptBucket::Printer) => p.machine.push(key.clone()),
            None => p.unmapped.push(key.clone()),
        }
    }
    p
}

// ---- printer / filament inference -----------------------------------

/// Strip OrcaSlicer's per-printer `@<suffix>` variant tag from a preset
/// name ("Bambu PLA Basic @BBL A1M" → "Bambu PLA Basic"). Our bundled
/// fragments are consolidated under the un-suffixed base name, so we
/// compare on the base.
fn base_preset_name(name: &str) -> &str {
    name.split(" @").next().unwrap_or(name).trim()
}

/// The bundled filament a project slot maps to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilamentMatch {
    /// The preset name as authored (suffix and all), for the report.
    pub requested: String,
    /// Bundled fragment identity (the `SlotBinding.filament_identity`
    /// wire form) if matched by base name; `None` if unmatched — the
    /// slot keeps the instance default and the report flags it.
    pub identity: Option<String>,
}

/// Map a BBS/Orca `filament_settings_id` ("Bambu PLA Basic @BBL A1M") to
/// a bundled fragment by base name (suffix stripped).
pub fn infer_filament(filament_settings_id: &str) -> FilamentMatch {
    let want = base_preset_name(filament_settings_id);
    let identity = crate::core::profile_library::list_filament_fragments()
        .iter()
        .find(|f| base_preset_name(&f.display_name) == want)
        .map(|f| f.identity.clone());
    FilamentMatch {
        requested: filament_settings_id.to_owned(),
        identity,
    }
}

// ---- delta vs our resolved baseline ---------------------------------

/// The settings to carry from a foreign project.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct OverrideOutcome {
    /// Keys whose foreign value differs from our resolved baseline —
    /// kept as overrides (canonical string form).
    pub overrides: BTreeMap<String, String>,
    /// Keys whose foreign value already matches our baseline — redundant,
    /// so dropped (reported as a count, not noise).
    pub redundant: Vec<String>,
    /// Process keys we couldn't read a value for (skipped).
    pub unreadable: Vec<String>,
    /// Enum keys whose foreign value isn't in our engine's value set —
    /// a fork-divergence (e.g. Bambu's `ironing_pattern = "zig-zag"`,
    /// which OrcaSlicer's own `handle_legacy` migrates to `rectilinear`;
    /// our set is `rectilinear`/`concentric`). Importing the raw value
    /// would inject an option libslic3r rejects, so it's dropped and
    /// reported (the cascade resolves these keys from our default).
    pub incompatible: Vec<String>,
}

/// Compute the overrides to import: for each **Process** key (machine
/// *and* filament keys are excluded — see below), keep it only when the
/// foreign value differs from `baseline` — our cascade resolved
/// (libslic3r key → value) for the bound printer / filament / process.
/// Keys that match the baseline are redundant and dropped, so the
/// imported project's override set stays minimal and the cascade
/// readable.
///
/// **Filament-bucket keys are not adopted.** In n3o a material's filament
/// identity — colour, type, temperatures, retraction, and the per-slot
/// `filament_*` vectors libslic3r fans — comes from the *slot* the
/// material binds to, not from the foreign project. Importing a foreign
/// project's filament settings would override the slot-owned values with
/// settings describing a different machine's filaments. It's the same
/// ownership rule that drops machine keys (owned by the bound
/// `PrinterInstance`), applied one bucket over (owned by the bound slot).
/// It also structurally removes a crash class: a foreign filament *vector*
/// (e.g. `filament_colour`) imported at the source project's length would
/// clash with the target's slot-fanned length and segfault libslic3r's MMU
/// painting (`filament_colour.size()` < `filament_diameter.size()`).
///
/// When in doubt the key is kept (an absent baseline entry counts as a
/// difference): a redundant override is harmless, a *missing* one would
/// silently lose the project's setting.
///
/// `enum_sets` maps an enum key to its valid value set (our engine's
/// `OptionDef.enum_values`). A foreign value outside that set — a
/// fork-divergence our libslic3r would reject — is dropped to
/// `incompatible` rather than imported; non-enum keys aren't in the map
/// and skip the check.
///
/// `only_keys`, when `Some`, restricts the import to exactly the project's
/// declared change list (`different_settings_to_system`) — every other
/// Process key resolves from the adopted process instead of landing as an
/// override. `None` imports the full delta (no change list).
pub fn compute_overrides(
    foreign: &OrcaProjectSettings,
    partition: &KeyPartition,
    baseline: &BTreeMap<String, String>,
    enum_sets: &BTreeMap<String, Vec<String>>,
    nonneg: &std::collections::HashSet<String>,
    only_keys: Option<&std::collections::HashSet<String>>,
) -> OverrideOutcome {
    let mut out = OverrideOutcome::default();
    for key in partition.process.iter() {
        // Intent-based import: skip anything the project didn't mark as
        // changed from its system preset.
        if only_keys.is_some_and(|only| !only.contains(key)) {
            continue;
        }
        let Some(value) = foreign.canonical(key) else {
            out.unreadable.push(key.clone());
            continue;
        };
        match baseline.get(key) {
            // A baseline match is "redundant" (drop it) only on the full-delta
            // path. When the project declares an explicit change list, the user
            // deliberately set this key, so keep it as an override even if it
            // currently equals our baseline — otherwise rebinding to a printer
            // whose default differs (snappy's max_bridge_length=10 -> bambi's 0)
            // silently discards the intent. The change list is BBS/Orca's record
            // of explicitly-changed settings; honoring it is the whole point.
            Some(base) if only_keys.is_none() && values_equal(base, &value) => {
                out.redundant.push(key.clone())
            }
            // Fork-divergent values our engine would reject: an enum value
            // outside its set, or a negative number for an option whose
            // range starts at >= 0 (e.g. Bambu's `tree_support_wall_count =
            // -1` / `raft_first_layer_expansion = -1` "auto" sentinels, where
            // Orca's min is 0). Dropped + reported, not injected.
            _ if !enum_value_known(&value, enum_sets.get(key)) => {
                out.incompatible.push(key.clone())
            }
            _ if negative_for_nonneg_option(&value, nonneg.contains(key)) => {
                out.incompatible.push(key.clone())
            }
            _ => {
                out.overrides.insert(key.clone(), value);
            }
        }
    }
    out
}

/// Per-option validity from our engine's schema, used to drop foreign
/// values our libslic3r would reject (fork-divergence): the enum value
/// set (e.g. Bambu's `ironing_pattern = "zig-zag"`) and the set of numeric
/// options that disallow negatives (`min >= 0`), to catch Bambu's `-1`
/// "auto" sentinels (`tree_support_wall_count`, `raft_first_layer_expansion`).
/// Built in one pass over `option_defs`.
fn engine_validity_sets() -> (
    BTreeMap<String, Vec<String>>,
    std::collections::HashSet<String>,
) {
    let defs = slic3r_ffi::option_defs();
    let enum_sets: BTreeMap<String, Vec<String>> = defs
        .iter()
        .filter(|d| !d.enum_values.is_empty())
        .map(|d| (d.key.clone(), d.enum_values.clone()))
        .collect();
    let nonneg: std::collections::HashSet<String> = defs
        .iter()
        .filter(|d| {
            matches!(
                d.ty,
                slic3r_ffi::OptType::Int
                    | slic3r_ffi::OptType::Ints
                    | slic3r_ffi::OptType::Float
                    | slic3r_ffi::OptType::Floats
            ) && d.min >= 0.0
        })
        .map(|d| d.key.clone())
        .collect();
    (enum_sets, nonneg)
}

/// The object-applicable override map for "Add model + settings": the
/// foreign project's config restricted to keys libslic3r honors per
/// object (object/region scope), minus filament-index-valued keys,
/// diffed against `baseline` (the destination plate's effective config)
/// with the same normalization + fork-divergence validation as the
/// full project import. Keys equal to the baseline drop out — the
/// imported object should pin only what actually differs from the
/// plate it lands on.
pub fn object_applicable_overrides(
    settings: &OrcaProjectSettings,
    baseline: &BTreeMap<String, String>,
) -> OverrideOutcome {
    let part = partition(settings);
    let object_keys: Vec<String> = part
        .process
        .into_iter()
        .filter(|k| crate::core::schema::is_object_overridable(k))
        .filter(|k| !crate::core::schema::is_filament_index_key(k))
        .collect();
    let part = KeyPartition {
        process: object_keys,
        ..Default::default()
    };
    let (enum_sets, nonneg) = engine_validity_sets();
    compute_overrides(settings, &part, baseline, &enum_sets, &nonneg, None)
}

/// Whether every element of a (possibly per-extruder, comma-joined) enum
/// value is in our engine's value set. `valid = None` means the key isn't
/// an enum we track — nothing to validate, so it passes.
fn enum_value_known(value: &str, valid: Option<&Vec<String>>) -> bool {
    match valid {
        Some(set) => value.split(',').all(|el| set.iter().any(|v| v == el)),
        None => true,
    }
}

/// Whether a (possibly per-extruder, comma-joined) value is a negative
/// number for an option whose declared range starts at >= 0. `nonneg` is
/// true only for numeric options whose `min >= 0`; for those, a negative
/// element is a fork-divergent sentinel our engine can't represent (Bambu's
/// `-1` "auto" for `tree_support_wall_count` / `raft_first_layer_expansion`).
///
/// Deliberately narrow: it does NOT enforce a *positive* lower bound or the
/// upper bound. Those are GUI hints in PrintConfig.cpp, and valid sentinels
/// sit outside them — e.g. `wall_filament = 0` (the "inherit the object's
/// filament" value, below its GUI `min = 1`) must NOT be dropped.
fn negative_for_nonneg_option(value: &str, nonneg: bool) -> bool {
    nonneg
        && value
            .split(',')
            .any(|el| matches!(el.trim().parse::<f64>(), Ok(v) if v < -1e-9))
}

/// Collapse a comma-joined vector whose elements are all equal to its
/// single value ("35,35,35" → "35"). A uniform per-filament setting then
/// compares equal regardless of how many slots each side resolved — the
/// project's filament count vs. our instance's — so it isn't a false
/// delta. Non-uniform vectors and scalars are returned unchanged.
fn collapse_uniform(v: &str) -> &str {
    match v.split_once(',') {
        Some((first, rest)) if rest.split(',').all(|p| p == first) => first,
        _ => v,
    }
}

/// Whether two libslic3r values mean the same thing, tolerating the
/// representation gaps between Bambu's serialization and ours:
/// per-filament slot count (uniform-vector collapse) and numeric
/// formatting ("1" vs "1.0", "0.30" vs "0.3"). Compares element-wise so
/// non-uniform numeric vectors normalize too.
fn values_equal(a: &str, b: &str) -> bool {
    let (a, b) = (collapse_uniform(a), collapse_uniform(b));
    if a == b {
        return true;
    }
    let (pa, pb): (Vec<&str>, Vec<&str>) = (a.split(',').collect(), b.split(',').collect());
    pa.len() == pb.len()
        && pa.iter().zip(&pb).all(|(x, y)| {
            x == y
                || matches!(
                    (x.parse::<f64>(), y.parse::<f64>()),
                    (Ok(nx), Ok(ny)) if (nx - ny).abs() < 1e-9
                )
        })
}

// ---- orchestration --------------------------------------------------

/// Summary of an import, surfaced to the user so lossy mapping is never
/// silent.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ImportReport {
    pub objects: usize,
    pub plates: usize,
    /// The bound printer instance id (none only if the user has no
    /// instances at all).
    pub printer_instance: Option<String>,
    /// The bound instance's display name, for the report.
    pub printer_instance_name: Option<String>,
    /// The model the project asked for.
    pub printer_model: Option<String>,
    /// No instance matched the model → bound an existing one as a
    /// fallback. Machine settings are dropped regardless, so this is
    /// safe; the user can rebind.
    pub printer_fallback: bool,
    pub filaments_matched: usize,
    pub filaments_unmatched: usize,
    /// Process/Filament settings carried as overrides.
    pub settings_applied: usize,
    /// Settings that matched our baseline and were dropped as redundant.
    pub settings_redundant: usize,
    /// Enum settings whose foreign value our engine doesn't recognize
    /// (fork-divergence), dropped rather than imported as invalid.
    pub settings_incompatible: usize,
    /// Printer/machine settings dropped (owned by the bound printer).
    pub settings_machine_dropped: usize,
    /// Filament settings dropped (owned by the bound slot — colour, type,
    /// temperatures, retraction all come from the slot a material binds to,
    /// not the foreign project).
    pub settings_filament_dropped: usize,
    /// Keys with no home in our model.
    pub settings_unmapped: usize,
    /// `true` when the project declared a `different_settings_to_system`
    /// change list and we imported only those keys (everything else resolves
    /// from the adopted process); `false` for the full delta-vs-baseline.
    pub settings_from_change_list: bool,
}

/// Find an *existing* user `PrinterInstance` whose printer model matches
/// `model`; else the first instance, flagged as a fallback. **Never
/// creates an instance.** `None` only when the user has no instances.
fn bind_instance(model: Option<&str>) -> Option<(PrinterInstance, bool)> {
    let instances = list_instances();
    if let Some(model) = model {
        if let Some(i) = instances.iter().find(|i| {
            lookup(&i.vendor_profile_ref)
                .map(|p| p.model == model)
                .unwrap_or(false)
        }) {
            return Some((i.clone(), false));
        }
    }
    instances.into_iter().next().map(|i| (i, true))
}

/// Map a foreign project's `print_settings_id` (e.g.
/// `"0.20mm Strength @BBL A1M"`) to one of the printer's bundled process
/// slugs (`"0.20mm-strength"`), when we ship a matching fragment. The
/// importer adopts this as the plate's `quality_profile`, so the plate
/// resolves + slices against the project's actual process rather than the
/// instance default — and its preset values aren't mistaken for overrides.
/// `None` when the project names no process or we ship no match.
fn mapped_process(printer_fragment_slug: &str, settings: &OrcaProjectSettings) -> Option<String> {
    let id = settings.print_settings_id()?;
    // "<layer> <variant> @<machine>" → "<layer>-<variant>".
    let base = id.split(" @").next().unwrap_or(&id).trim();
    let slug = base.to_lowercase().replace(' ', "-");
    crate::core::profile_library::bundled_process_slugs_for_printer(printer_fragment_slug)
        .iter()
        .any(|s| *s == slug)
        .then_some(slug)
}

/// Our cascade resolved (libslic3r key → value) for the bound instance +
/// the project's bed + filament + `quality_profile` — the baseline to
/// delta the project's settings against, so only genuinely-different keys
/// become overrides. `quality_profile` is the plate's adopted process (see
/// [`mapped_process`]); composing against it is what makes a project's
/// preset values resolve natively instead of as a pile of false overrides.
/// Best-effort: any failure yields an empty map, which keeps every
/// Process/Filament key (correct, just non-minimal).
fn resolve_baseline(
    instance: &PrinterInstance,
    settings: &OrcaProjectSettings,
    quality_profile: Option<&str>,
) -> BTreeMap<String, String> {
    let Some(printer) = lookup(&instance.vendor_profile_ref) else {
        return BTreeMap::new();
    };
    let effective = crate::core::profile_library::with_quality_profile(instance, quality_profile);
    let Ok(cascade) = crate::core::profile_library::compose_cascade(&effective, &[]) else {
        return BTreeMap::new();
    };
    // The project's bed (its `when.plate.type` is the libslic3r bed
    // identity verbatim); fall back to the instance's bed.
    let bed = settings
        .curr_bed_type()
        .unwrap_or_else(|| instance.bed.identity.clone());
    let plate = build_plate::lookup(&bed).unwrap_or(BuildPlate {
        identity: bed.clone(),
        libslic3r_curr_bed_type: bed,
    });
    // Filament context drives `when.filament.type`; mirror the project's
    // per-slot material families.
    let types = settings.list("filament_type").unwrap_or_default();
    let mk = |base_type: String| {
        Arc::new(FilamentProfile {
            identity: "imported".into(),
            base_type,
            vendor: None,
            color: None,
        })
    };
    let filaments: Vec<Arc<FilamentProfile>> = if types.is_empty() {
        vec![mk("PLA".into())]
    } else {
        types.into_iter().map(mk).collect()
    };
    let ctx = SlicingContext::new(Arc::new(printer), Arc::new(plate), filaments);
    // Seed with libslic3r's per-option defaults so keys our fragments
    // never set (whose value is just the engine default at slice time)
    // don't read as deltas — then overlay our cascade-resolved values.
    let mut baseline: BTreeMap<String, String> = slic3r_ffi::option_defs()
        .into_iter()
        .filter_map(|d| d.default_serialized.map(|v| (d.key, v)))
        .collect();
    for (k, v) in crate::core::cascade::resolve(&cascade, &ctx) {
        baseline.insert(k, v.value);
    }
    baseline
}

/// OrcaSlicer's plate-grid column count for `plate_count` plates —
/// `compute_colum_count` from upstream `PartPlate.hpp`.
fn plate_columns(plate_count: usize) -> usize {
    let value = (plate_count as f64).sqrt();
    let rounded = value.round();
    if value > rounded {
        rounded as usize + 1
    } else {
        rounded as usize
    }
}

/// World-space origin of plate `index` (0-based) in OrcaSlicer's plate
/// grid: `(col·width·1.2, −row·depth·1.2)` with `LOGICAL_PART_PLATE_GAP
/// = 1/5` (upstream `PartPlateList::compute_shape_position`). Build-item
/// transforms are authored in this multi-plate world frame, so
/// subtracting the plate's origin recovers the plate-local position n3o
/// renders per plate.
fn plate_origin(index: usize, cols: usize, width: f64, depth: f64) -> (f64, f64) {
    const GAP: f64 = 1.0 / 5.0;
    let row = index / cols;
    let col = index % cols;
    (
        col as f64 * width * (1.0 + GAP),
        -(row as f64) * depth * (1.0 + GAP),
    )
}

/// Source bed `(width, depth)` from the project's `printable_area`
/// polygon (`"0x0"`, `"180x0"`, …). `None` when absent/unparsable — the
/// caller then skips the plate-origin de-offset (objects keep world
/// coords, which only matters for multi-plate projects).
fn source_bed_size(settings: &OrcaProjectSettings) -> Option<(f64, f64)> {
    let pts = settings.list("printable_area")?;
    let (mut xs, mut ys) = (Vec::new(), Vec::new());
    for p in pts {
        let (x, y) = p.split_once('x')?;
        xs.push(x.trim().parse::<f64>().ok()?);
        ys.push(y.trim().parse::<f64>().ok()?);
    }
    if xs.is_empty() {
        return None;
    }
    let span = |v: &[f64]| {
        v.iter().cloned().fold(f64::MIN, f64::max) - v.iter().cloned().fold(f64::MAX, f64::min)
    };
    let (w, d) = (span(&xs), span(&ys));
    (w > 0.0 && d > 0.0).then_some((w, d))
}

/// Resolve the active plate's bound `PrinterInstance` from the registry, or
/// `None` when unbound. The importer is a boundary builder (like the command
/// layer), so it resolves the registry and passes the instance into the pure
/// model mutations (`register_object`, `ensure_material_bound_on_active`).
fn active_plate_instance(project: &Project) -> Option<crate::core::printer::PrinterInstance> {
    crate::core::project::session::resolve_plate_instance(project.active_plate())
}

fn register_obj(
    project: &mut Project,
    mesh_ids: &[MeshId],
    obj: &ProjectObject,
    plate_origin: (f64, f64),
    instance: Option<&crate::core::printer::PrinterInstance>,
) {
    // Shift the object out of OrcaSlicer's multi-plate world frame into
    // its plate-local frame (no-op for plate 0, whose origin is (0,0)).
    let transform = if plate_origin == (0.0, 0.0) {
        obj.transform
    } else {
        Transform::translation(glam::Vec3::new(
            -plate_origin.0 as f32,
            -plate_origin.1 as f32,
            0.0,
        ))
        .compose(obj.transform)
    };
    let id = project.register_object(
        NewSceneObject {
            mesh: mesh_ids[obj.mesh_idx],
            transform,
            name: obj.name.clone(),
            visible: true,
            extruder_id: obj.extruder_id,
            group: obj.group,
        },
        instance,
    );
    // Per-object overrides from the imported project's model_settings.config.
    project.apply_imported_object_overrides(id, &obj.overrides);
}

/// Import an OrcaSlicer / Bambu Studio `.3mf` **project** into a fresh
/// n3o `Project`. Geometry + per-object/plate structure come from the
/// existing 3MF reader (which applies `model_settings.config`); the
/// project's `project_settings.config` settings are carried as overrides
/// (Process only — machine keys owned by the bound printer and filament
/// keys owned by the bound slot are dropped). The plate(s) bind to
/// an existing matching `PrinterInstance` (fallback flagged, never
/// created).
///
/// Returns the built project + a report. Note: settings currently use an
/// **empty baseline**, so every Process/Filament key is kept (correct,
/// but non-minimal). Supplying the cascade-resolved baseline to drop
/// redundant keys is the follow-up — `compute_overrides` already takes
/// the baseline.
pub fn import(path: &Path) -> Result<(Project, ImportReport), String> {
    let loaded = load_3mf(path).map_err(|e| e.to_string())?;
    let crate::core::threemf::Project3mf {
        meshes,
        objects,
        plate_assignments,
        embedded_settings,
        file_metadata,
        ..
    } = loaded;

    let raw = embedded_settings
        .ok_or_else(|| "no project_settings.config — not a foreign project".to_string())?;
    let settings = OrcaProjectSettings::parse(raw.as_bytes())?;
    let part = partition(&settings);

    // Bind an existing instance by model (fallback flagged; never create).
    let model = settings.printer_model();
    let bind = bind_instance(model.as_deref());
    let instance_id = bind.as_ref().map(|(i, _)| i.id.clone());
    let instance_name = bind.as_ref().map(|(i, _)| i.display_name.clone());
    // The project's process preset, mapped to a bundled slug the bound
    // printer ships. Adopted as each plate's `quality_profile` so the
    // plate resolves + slices against the project's actual process.
    let process_slug = bind
        .as_ref()
        .and_then(|(i, _)| mapped_process(&i.printer_fragment_slug, &settings));

    // Fresh project (one default plate). Bind plate 1 before registering
    // objects so register_object's material→slot auto-bind sees the
    // instance.
    let mut project = Project::new();
    project.file_metadata = file_metadata;
    // The save target (`.n3o` alongside the foreign file) is the caller's
    // concern — `load_or_import` seeds it into `SessionRuntime.source_path`.
    if let Some(id) = &instance_id {
        // Rebind plate 0 to the matched (possibly non-default) printer; the
        // bed derives from the binding when this project is wrapped in a
        // `Session` (`load_or_import` → `Session::new` reconciles).
        project.plates[0].set_printer(Some(id.clone()));
    }

    // Plates: foreign plate ids in order; create the ones past plate 1.
    let mut plate_map = std::collections::HashMap::new();
    plate_map.insert(1u32, project.plates[0].id);
    let mut foreign_plate_ids: Vec<u32> = plate_assignments.keys().copied().collect();
    foreign_plate_ids.sort_unstable();
    for fid in &foreign_plate_ids {
        if *fid != 1 {
            let (pid, _) = project.add_plate(instance_id.clone());
            plate_map.insert(*fid, pid);
        }
    }

    // Meshes are scene-wide; register them once and keep the id order.
    let mesh_ids: Vec<MeshId> = meshes
        .into_iter()
        .map(|m| project.register_mesh(m))
        .collect();

    // Objects → their plate (set_active_plate steers register_object).
    // Build-item transforms are in OrcaSlicer's multi-plate world frame;
    // de-offset each plate's objects to plate-local using the source bed
    // size + OrcaSlicer's plate-grid layout.
    let bed = source_bed_size(&settings);
    let cols = plate_columns(foreign_plate_ids.len().max(1));
    let mut object_count = 0usize;
    if foreign_plate_ids.is_empty() {
        let instance = active_plate_instance(&project);
        for obj in &objects {
            register_obj(&mut project, &mesh_ids, obj, (0.0, 0.0), instance.as_ref());
            object_count += 1;
        }
    } else {
        for fid in &foreign_plate_ids {
            project
                .set_active_plate(plate_map[fid])
                .map_err(|e| format!("set active plate: {e:?}"))?;
            let instance = active_plate_instance(&project);
            // Plate index is the 1-based foreign id minus one.
            let origin = bed
                .map(|(w, d)| plate_origin((*fid as usize).saturating_sub(1), cols, w, d))
                .unwrap_or((0.0, 0.0));
            for &idx in &plate_assignments[fid] {
                register_obj(
                    &mut project,
                    &mesh_ids,
                    &objects[idx],
                    origin,
                    instance.as_ref(),
                );
                object_count += 1;
            }
        }
        let first = project.plates[0].id;
        let _ = project.set_active_plate(first);
    }

    // Painted (MMU color) filaments. A model whose 2nd+ filament is applied
    // by face `paint_color` rather than a per-object `extruder` carries no
    // object with `extruder = N`, so `material_count` would miss it and the
    // slice would fan a single filament — libslic3r then has nothing to
    // segment the painted faces to. Bind the project's declared filaments as
    // plate materials on any plate that has a painted object, so the cascade
    // fans them and AMS / toolhead routing covers them. (Per-plate precision
    // — exactly which filament indices a plate's paint references — arrives
    // with the paint decoder; for now adopt the project's filament count,
    // which is what the source slicer shows.)
    let filament_count = settings.filament_settings_ids().len().min(u8::MAX as usize) as u8;
    if filament_count > 1 {
        let painted_plates: Vec<_> = project
            .plates
            .iter()
            .filter(|pl| project.plate_has_painted_object(pl))
            .map(|pl| pl.id)
            .collect();
        for pid in painted_plates {
            let _ = project.set_active_plate(pid);
            let instance = active_plate_instance(&project);
            for material in 1..=filament_count {
                project.ensure_material_bound_on_active(material, instance.as_ref());
            }
        }
        let first = project.plates[0].id;
        let _ = project.set_active_plate(first);
    }

    // Settings → per-plate **project_overrides**, minimized against our
    // cascade baseline for the bound printer: keys that already match our
    // default drop out, leaving the project's genuine deltas (machine
    // already excluded by `partition`). These are the *project's* settings
    // (they came from its file), so they belong on the plate's
    // project tier — not Project.user_overrides (the user's everywhere
    // tier) — which is also where the settings panel already shows them.
    // (project_settings.config is the project's single active config; we
    // don't yet read per-plate plate_N.json, so every plate gets it.)
    let baseline = match bind.as_ref() {
        Some((instance, _)) => resolve_baseline(instance, &settings, process_slug.as_deref()),
        None => BTreeMap::new(),
    };
    let (enum_sets, nonneg) = engine_validity_sets();
    // Intent-based import: when the project declares which keys it changed
    // from its system preset, import only those (everything else resolves
    // from the adopted process). Absent/null → full delta.
    let changed = settings.changed_from_system();
    let outcome = compute_overrides(
        &settings,
        &part,
        &baseline,
        &enum_sets,
        &nonneg,
        changed.as_ref(),
    );
    for plate in project.plates.iter_mut() {
        plate.quality_profile = process_slug.clone();
        for (k, v) in &outcome.overrides {
            plate.project_overrides.insert(k.clone(), v.clone());
        }
    }

    // Filament match counts (report only — no instance mutation).
    let (mut matched, mut unmatched) = (0usize, 0usize);
    for id in settings.filament_settings_ids() {
        if infer_filament(&id).identity.is_some() {
            matched += 1;
        } else {
            unmatched += 1;
        }
    }

    let report = ImportReport {
        objects: object_count,
        plates: project.plates.len(),
        printer_instance: instance_id,
        printer_instance_name: instance_name,
        printer_model: model,
        printer_fallback: bind.map(|(_, fb)| fb).unwrap_or(false),
        filaments_matched: matched,
        filaments_unmatched: unmatched,
        settings_applied: outcome.overrides.len(),
        settings_redundant: outcome.redundant.len(),
        settings_incompatible: outcome.incompatible.len(),
        settings_machine_dropped: part.machine.len(),
        settings_filament_dropped: part.filament.len(),
        settings_unmapped: part.unmapped.len(),
        settings_from_change_list: changed.is_some(),
    };
    Ok((project, report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::threemf::read_3mf_extra_entry;
    use std::path::PathBuf;

    fn fourcolor_project_settings() -> OrcaProjectSettings {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("examples/spike3/fourcolor.3mf");
        let bytes = read_3mf_extra_entry(&path, "Metadata/project_settings.config")
            .expect("read entry")
            .expect("project_settings.config present in fourcolor.3mf");
        OrcaProjectSettings::parse(&bytes).expect("parse project_settings.config")
    }

    #[test]
    fn parses_identity_keys_from_a_real_bbs_project() {
        let s = fourcolor_project_settings();
        assert_eq!(s.printer_model().as_deref(), Some("Bambu Lab A1 mini"));
        assert_eq!(s.curr_bed_type().as_deref(), Some("Supertack Plate"));
        assert_eq!(
            s.print_settings_id().as_deref(),
            Some("0.20mm Standard @BBL A1M")
        );
        // Four-color print → four filament slots, all the same preset.
        let fils = s.filament_settings_ids();
        assert_eq!(fils.len(), 4);
        assert!(fils.iter().all(|f| f == "Bambu PLA Basic @BBL A1M"));
    }

    #[test]
    fn partition_routes_buckets_and_drops_machine_keys() {
        let s = fourcolor_project_settings();
        let p = partition(&s);

        // The config is large; every bucket should be populated.
        assert!(!p.process.is_empty(), "expected process keys");
        assert!(!p.filament.is_empty(), "expected filament keys");
        assert!(
            !p.machine.is_empty(),
            "expected machine (printer-bucket) keys"
        );

        // Spot-check the classification against known keys.
        assert!(p.process.contains(&"layer_height".to_string()));
        assert!(p.filament.contains(&"filament_type".to_string()));
        // A machine key must land in `machine` (dropped), never in the
        // imported buckets.
        assert!(p.machine.contains(&"machine_start_gcode".to_string()));
        assert!(!p.process.contains(&"machine_start_gcode".to_string()));
        assert!(!p.filament.contains(&"machine_start_gcode".to_string()));

        // The buckets are disjoint partitions of the (non-identity) keys.
        let total = p.process.len() + p.filament.len() + p.machine.len() + p.unmapped.len();
        let non_identity = s
            .settings
            .keys()
            .filter(|k| !IDENTITY_KEYS.contains(&k.as_str()))
            .count();
        assert_eq!(total, non_identity);
    }

    #[test]
    fn infers_filament_by_base_name_stripping_the_variant_suffix() {
        // The project's "@BBL A1M" variant maps to our consolidated
        // "Bambu PLA Basic" fragment by base name.
        let m = infer_filament("Bambu PLA Basic @BBL A1M");
        assert!(
            m.identity.is_some(),
            "expected a bundled match for Bambu PLA Basic, got {m:?}",
        );
        // An unknown filament stays unbound (slot keeps the default).
        let u = infer_filament("Nonexistent Filament @XYZ");
        assert!(u.identity.is_none());
    }

    #[test]
    fn canonical_scalar_and_list_forms() {
        // Schema (for the per-OptType separator) needs the FFI option table.
        let _ = slic3r_ffi::init(None, 3);
        let s = fourcolor_project_settings();
        // layer_height is a scalar string — no separator.
        let lh = s.canonical("layer_height").expect("layer_height present");
        assert!(
            !lh.contains(',') && !lh.contains(';'),
            "scalar should not be joined: {lh}"
        );
        // filament_settings_id is a 4-element *string* vector → libslic3r
        // joins those with `;`, not `,`.
        let f = s.canonical("filament_settings_id").expect("present");
        assert_eq!(
            f.matches(';').count(),
            3,
            "four string-vector elements → three semicolons: {f}"
        );
        // Regression guard for the MMU-painting crash: filament_colour is a
        // string vector, so a comma join would make libslic3r read it as a
        // single color and segfault in color-painting segmentation. It must
        // be `;`-joined.
        let c = s
            .canonical("filament_colour")
            .expect("filament_colour present");
        assert!(
            c.contains(';') && !c.contains(','),
            "filament_colour must be semicolon-joined: {c}"
        );
    }

    #[test]
    fn object_applicable_overrides_gates_scope_filament_keys_and_diffs() {
        // Scope gating + filament-index drop consult the FFI option table.
        let _ = slic3r_ffi::init(None, 3);
        let s = OrcaProjectSettings {
            settings: BTreeMap::from([
                // Object-scope, differs from baseline → kept.
                ("enable_support".into(), serde_json::json!("1")),
                // Region-scope, matches baseline → redundant, dropped.
                ("wall_loops".into(), serde_json::json!("3")),
                // Print-scope → not object-applicable, dropped.
                ("skirt_loops".into(), serde_json::json!("2")),
                // Object-scope but a filament index → dropped on principle.
                ("support_filament".into(), serde_json::json!("2")),
            ]),
        };
        let baseline = BTreeMap::from([
            ("enable_support".to_string(), "0".to_string()),
            ("wall_loops".to_string(), "3".to_string()),
        ]);

        let out = object_applicable_overrides(&s, &baseline);
        assert_eq!(
            out.overrides,
            BTreeMap::from([("enable_support".to_string(), "1".to_string())]),
            "only the differing object-applicable key survives",
        );
        assert!(out.redundant.contains(&"wall_loops".to_string()));
    }

    #[test]
    fn compute_overrides_keeps_differences_drops_redundant_and_machine() {
        let s = fourcolor_project_settings();
        let p = partition(&s);

        // Baseline pins one process key to the foreign value (→ redundant);
        // every other proc/fil key is absent (→ counts as a difference).
        let pinned = p.process.first().expect("a process key").clone();
        let mut baseline = BTreeMap::new();
        baseline.insert(pinned.clone(), s.canonical(&pinned).unwrap());

        // No enum/negative validation in this unit (FFI not initialized
        // here); empty sets leave every value valid. `None` = full delta
        // (no change list).
        let out = compute_overrides(
            &s,
            &p,
            &baseline,
            &BTreeMap::new(),
            &std::collections::HashSet::new(),
            None,
        );

        // The pinned key matched the baseline → redundant, not overridden.
        assert!(out.redundant.contains(&pinned));
        assert!(!out.overrides.contains_key(&pinned));
        // The rest differ → overrides.
        assert!(!out.overrides.is_empty());
        // A machine key is never an override (partition dropped it).
        assert!(!out.overrides.contains_key("machine_start_gcode"));
        // A filament key is never an override either — filament identity is
        // owned by the bound slot, so the whole bucket is dropped.
        for fk in &p.filament {
            assert!(
                !out.overrides.contains_key(fk),
                "filament key {fk} should not be imported as an override",
            );
        }
        // Every Process key is accounted for (filament keys are dropped
        // wholesale, so they don't appear in any outcome bucket).
        let seen = out.overrides.len()
            + out.redundant.len()
            + out.unreadable.len()
            + out.incompatible.len();
        assert_eq!(seen, p.process.len());
    }

    #[test]
    fn imports_a_real_bambu_studio_project_end_to_end() {
        // FFI up so resolve_baseline can seed libslic3r defaults (the
        // production path); without it the baseline lacks defaults and
        // default-valued keys read as false deltas.
        let _ = slic3r_ffi::init(None, 3);
        // Isolate the global instance registry: this reads it via import()'s
        // printer match, so it must serialize with registry-mutating tests
        // (e.g. the non-default-bed test) and start from the bundled set.
        let _g = crate::core::printer::instance_registry::RegistryGuard::acquire();
        // The project lead's real A1 mini Bambu Studio project.
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/3mf/case-bambu-studio.3mf");
        let (project, report) = import(&path).expect("import case-bambu-studio.3mf");

        // Geometry landed: objects on plates, count agrees with the report.
        assert!(
            report.objects >= 1,
            "expected objects, got {}",
            report.objects
        );
        let placed: usize = project.plates.iter().map(|pl| pl.scene.objects.len()).sum();
        assert_eq!(
            placed, report.objects,
            "report.objects must match placed objects"
        );

        // (The save target — `.n3o` alongside the foreign file — is seeded
        // into `SessionRuntime.source_path` by `load_or_import`, not here.)

        // Bound to an existing A1 mini instance (the bundled `bambi`
        // fixture) by exact model match — not a fallback, never created.
        assert_eq!(report.printer_model.as_deref(), Some("Bambu Lab A1 mini"));
        assert!(
            report.printer_instance.is_some(),
            "should bind an existing instance"
        );
        assert!(!report.printer_fallback, "A1 mini matches `bambi` exactly");

        // Settings carried as project overrides; machine settings dropped.
        assert!(report.settings_applied > 0, "expected imported settings");
        assert!(
            report.settings_machine_dropped > 0,
            "expected dropped machine settings"
        );
        // Settings land on the plate's project tier (where they came
        // from + where the panel shows them), not the user-everywhere tier.
        assert!(project.user_overrides.is_empty());
        let plate_ov = &project.plates[0].project_overrides;
        assert_eq!(report.settings_applied, plate_ov.len());
        assert!(!plate_ov.contains_key("machine_start_gcode"));
        assert!(!plate_ov.contains_key("nozzle_diameter"));

        // Intent-based import: case.3mf declares `different_settings_to_system`
        // (the support keys the user changed from the Strength preset), so we
        // import ONLY those — everything else resolves from the adopted
        // process. (The full delta-vs-baseline path is covered by the unit
        // tests below; it's not exercised here since the change list exists.)
        assert!(
            report.settings_from_change_list,
            "case.3mf has a change list → intent-based import",
        );
        // The genuine support tweak the user made imports...
        assert_eq!(
            plate_ov.get("support_top_z_distance").map(String::as_str),
            Some("0.3"),
            "a key in the change list should import",
        );
        // ...and nothing outside the change list lands as an override: not the
        // process defaults (outer_wall_speed, interlocking_depth,
        // cool_plate_temp), not the invalid Bambu-default sentinels the user
        // never touched (ironing_pattern=zig-zag, tree_support_wall_count=-1),
        // not the filament selectors. All resolve from the adopted process.
        for absent in [
            "outer_wall_speed",
            "interlocking_depth",
            "cool_plate_temp",
            "ironing_pattern",
            "tree_support_wall_count",
            "wall_filament",
        ] {
            assert!(
                !plate_ov.contains_key(absent),
                "{absent} is not in the change list — must not import",
            );
        }
        // The override set is exactly the declared, engine-compatible change-list
        // keys — kept even when a value equals our Strength baseline (an
        // explicitly-changed setting must survive a later printer rebind), and
        // never more than the 5 declared.
        assert!(
            plate_ov.len() <= 5,
            "intent-based import should be minimal; got {}",
            plate_ov.len(),
        );

        // The plate adopts the project's process preset regardless of the
        // change list (from print_settings_id) — "0.20mm Strength" → our
        // `0.20mm-strength` slug, so it both shows and slices the preset.
        assert_eq!(
            project.plates[0].quality_profile.as_deref(),
            Some("0.20mm-strength"),
            "the imported plate should adopt the project's process preset",
        );
    }

    #[test]
    fn changed_from_system_distinguishes_absent_present_empty_null() {
        use serde_json::json;
        let with = |v: serde_json::Value| OrcaProjectSettings {
            settings: [("different_settings_to_system".to_string(), v)]
                .into_iter()
                .collect(),
        };
        // Absent → None (full import).
        assert!(OrcaProjectSettings {
            settings: BTreeMap::new()
        }
        .changed_from_system()
        .is_none());
        // Present, BBS-structured (keys in slot 0, other slots blank) →
        // flattened union.
        let set = with(json!(["a;b;c", "", "", ""]))
            .changed_from_system()
            .expect("present");
        assert_eq!(set, ["a", "b", "c"].iter().map(|s| s.to_string()).collect());
        // Present but empty (every slot blank) → Some(empty): import nothing.
        assert_eq!(
            with(json!(["", "", ""]))
                .changed_from_system()
                .expect("present")
                .len(),
            0
        );
        // null → None (treated as absent — our call).
        assert!(with(json!(null)).changed_from_system().is_none());
    }

    #[test]
    fn compute_overrides_validates_invalid_values_and_honors_the_change_list() {
        use serde_json::json;
        use std::collections::HashSet;
        let s = OrcaProjectSettings {
            settings: [
                ("ironing_pattern", json!("zig-zag")),    // enum not in our set
                ("tree_support_wall_count", json!("-1")), // negative, min>=0
                ("wall_filament", json!("0")),            // valid 0 sentinel
                ("outer_wall_speed", json!("99")),        // ordinary delta
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
        };
        let part = KeyPartition {
            process: [
                "ironing_pattern",
                "tree_support_wall_count",
                "wall_filament",
                "outer_wall_speed",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
            ..Default::default()
        };
        let baseline = BTreeMap::new(); // empty → everything differs
        let enum_sets: BTreeMap<String, Vec<String>> = [(
            "ironing_pattern".to_string(),
            vec!["rectilinear".to_string(), "concentric".to_string()],
        )]
        .into_iter()
        .collect();
        let nonneg: HashSet<String> = ["tree_support_wall_count".to_string()]
            .into_iter()
            .collect();

        // Full delta (no change list): invalid values drop to incompatible,
        // valid ones import (incl. the wall_filament=0 sentinel).
        let full = compute_overrides(&s, &part, &baseline, &enum_sets, &nonneg, None);
        assert!(full.incompatible.contains(&"ironing_pattern".to_string()));
        assert!(full
            .incompatible
            .contains(&"tree_support_wall_count".to_string()));
        assert_eq!(
            full.overrides.get("wall_filament").map(String::as_str),
            Some("0")
        );
        assert_eq!(
            full.overrides.get("outer_wall_speed").map(String::as_str),
            Some("99")
        );

        // Change list = {outer_wall_speed}: only that key is considered; the
        // others are skipped entirely (not even validated).
        let only: HashSet<String> = ["outer_wall_speed".to_string()].into_iter().collect();
        let intent = compute_overrides(&s, &part, &baseline, &enum_sets, &nonneg, Some(&only));
        assert_eq!(
            intent.overrides.keys().cloned().collect::<Vec<_>>(),
            vec!["outer_wall_speed".to_string()]
        );
        assert!(
            intent.incompatible.is_empty(),
            "keys outside the change list aren't validated"
        );
    }

    #[test]
    fn change_list_keeps_an_explicit_override_even_when_it_equals_the_baseline() {
        use serde_json::json;
        use std::collections::HashSet;
        // The project explicitly set max_bridge_length=10; it happens to equal
        // our current baseline (the bound printer's default is also 10). On the
        // full-delta path that's "redundant" and dropped — but when the project
        // *declares* the key changed, it must be kept, so rebinding to a printer
        // with a different default (bambi's 0) doesn't silently lose the intent.
        let s = OrcaProjectSettings {
            settings: [("max_bridge_length", json!("10"))]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        };
        let part = KeyPartition {
            process: ["max_bridge_length"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            ..Default::default()
        };
        let baseline: BTreeMap<String, String> =
            [("max_bridge_length".to_string(), "10".to_string())]
                .into_iter()
                .collect();
        let enum_sets = BTreeMap::new();
        let nonneg = HashSet::new();

        // No change list: equal-to-baseline is redundant and dropped.
        let full = compute_overrides(&s, &part, &baseline, &enum_sets, &nonneg, None);
        assert!(full.overrides.is_empty());
        assert!(full.redundant.contains(&"max_bridge_length".to_string()));

        // Declared in the change list: kept despite matching the baseline.
        let only: HashSet<String> = ["max_bridge_length".to_string()].into_iter().collect();
        let intent = compute_overrides(&s, &part, &baseline, &enum_sets, &nonneg, Some(&only));
        assert_eq!(
            intent
                .overrides
                .get("max_bridge_length")
                .map(String::as_str),
            Some("10"),
            "an explicitly-changed key must survive even when it equals the baseline",
        );
        assert!(intent.redundant.is_empty());
    }

    #[test]
    fn plate_grid_matches_orcaslicer_layout() {
        // compute_colum_count: round(sqrt), +1 when sqrt exceeds the round.
        assert_eq!(plate_columns(1), 1);
        assert_eq!(plate_columns(2), 2); // sqrt 1.41 > round 1 → 2
        assert_eq!(plate_columns(4), 2);
        assert_eq!(plate_columns(5), 3); // sqrt 2.24 > round 2 → 3
        assert_eq!(plate_columns(9), 3);

        // 180mm bed → stride 180·1.2 = 216. Plate 0 at the origin; plate 1
        // (col 1) shifts +216 in x; a second-row plate shifts −216 in y.
        let near = |a: (f64, f64), b: (f64, f64)| {
            assert!(
                (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6,
                "{a:?} vs {b:?}"
            );
        };
        near(plate_origin(0, 2, 180.0, 180.0), (0.0, 0.0));
        near(plate_origin(1, 2, 180.0, 180.0), (216.0, 0.0));
        near(plate_origin(3, 3, 180.0, 180.0), (0.0, -216.0));
    }

    #[test]
    fn source_bed_size_from_printable_area() {
        let mut settings = BTreeMap::new();
        settings.insert(
            "printable_area".to_string(),
            serde_json::json!(["0x0", "180x0", "180x180", "0x180"]),
        );
        let s = OrcaProjectSettings { settings };
        assert_eq!(source_bed_size(&s), Some((180.0, 180.0)));

        // Missing key → None (caller skips the de-offset).
        assert_eq!(
            source_bed_size(&OrcaProjectSettings {
                settings: BTreeMap::new()
            }),
            None
        );
    }

    #[test]
    fn import_recomputes_plate_bed_for_a_non_default_matched_printer() {
        use crate::core::printer::instance_registry::RegistryGuard;
        use crate::core::printer::{create_instance, delete_instance};
        use crate::core::scene::bed::bed_for_printer;

        // Bundled order is [bambi (A1, default), snappy (U1)]. Make a
        // *non-default* printer the match target: drop the default A1, then
        // recreate it so it lands after the U1. Plate 0 is then seeded with the
        // U1's bed (the default), while the A1 project below matches the A1.
        let _g = RegistryGuard::acquire();
        delete_instance("bambi").expect("drop default A1");
        let a1 = create_instance("bambu-lab-a1-mini", "A1 (non-default)".into(), 1)
            .expect("recreate A1 as a non-default instance");

        // fourcolor.3mf is a Bambu Lab A1 mini project (see the identity test).
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("examples/spike3/fourcolor.3mf");
        let (project, _report) = import(&path).expect("import A1 project");

        // Plate 0 binds the matched A1...
        assert_eq!(
            project.plates[0].printer_instance_id(),
            Some(a1.id.as_str()),
        );
        // ...and its bed geometry follows the A1, not the default U1. (Before
        // the fix it stayed at Project::new()'s default-instance bed seed.)
        let a1_profile = lookup(&a1.vendor_profile_ref).expect("A1 profile");
        let expected = bed_for_printer(&a1_profile);
        let plate_bed = crate::core::project::session::derive_bed(&project.plates[0])
            .expect("plate 0 bed populated");
        assert_eq!(
            (plate_bed.extents.min, plate_bed.extents.max),
            (expected.extents.min, expected.extents.max),
            "plate 0 bed must follow the bound A1, not the default U1",
        );
    }
}
