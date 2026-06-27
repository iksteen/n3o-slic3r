//! Runtime cascade composer.
//!
//! Composes a slice-time cascade from the hierarchical vendor fragment
//! layout (per-printer model.toml + per-nozzle scalar nozzle.toml +
//! per-bed bed.toml + filament + process).
//!
//! Composition order. Authored layers 1–5 are lowest-precedence first and
//! win among themselves by the resolver's source-order tie-break; layer 5b
//! (the instance's saved machine overrides) is `!important` so it wins over
//! the printer fragment's machine globals:
//!
//!   1. Printer fragment      (machine globals only; no per-extruder)
//!   2. Bed fragment          (bed identity + curr_bed_type)
//!   3. Per-extruder nozzle fragments, scalar-to-vector assembled
//!   4. Filament fragment     (single-slot MVP)
//!   5. Process fragment
//!   5b. Machine overrides    (per-instance saved config — `Rule::important`)
//!
//! The user/project/object override tiers are applied as a second phase by
//! `cascade::resolve_with_overrides`, not baked here.
//!
//! The per-extruder vector assembly step zips each nozzle fragment's
//! scalars into vectors keyed at the extruder dimension. For an A1
//! mini (1 extruder) the vectors are length 1; for a U1 (4 extruders)
//! the vectors are length 4 with one entry per extruder. The composer
//! emits each vector key as a single synthesized cascade rule whose
//! value is the libslic3r-style semicolon-separated string the FFI
//! consumes.

use super::{
    load_bed_fragment, load_filament_fragment, load_nozzle_fragment, load_printer_fragment,
    load_process_fragment,
};
use crate::core::cascade::resolver::{resolve, MapContext};
use crate::core::cascade::types::{Cascade, Predicate, Rule, SourceLocation};
use crate::core::filament;
use crate::core::printer::{PrinterInstance, SlotBinding, SlotRef};
use crate::core::schema::schema_by_key;
use slic3r_ffi::OptType;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Per-filament-position view used by the composer to fan vector
/// keys (filament_diameter, filament_settings_id, filament_colour,
/// filament_map, …). One entry per libslic3r filament slot — same
/// length as the cascade's filament dimension.
///
/// `slot_ref` is the `PrinterInstance` slot this filament is bound
/// to (`None` for an unbound filament position); the composer
/// resolves it to the actual `SlotBinding` and uses that for the
/// fragment slug, color, and extruder index.
struct FilamentEntry<'a> {
    /// The slot binding to source this filament's identity/color
    /// from. `None` means "unbound" — the composer falls back to
    /// the instance's `default_filament_fragment_slug` for the
    /// slug, `extruder=0` for `filament_map`, and the default
    /// color.
    slot: Option<&'a SlotBinding>,
    /// 0-based extruder index this filament should feed from (for
    /// `filament_map`). When `slot.is_some()`, this is the
    /// extruder the slot lives on; when `None`, defaults to 0.
    extruder_index: u8,
}

/// Resolve a list of `Option<SlotRef>` material bindings against
/// the printer instance, producing the per-filament view the
/// composer fan-outs consume. Out-of-range slot refs (a stale
/// binding from a project file authored against a now-shrunken
/// instance) collapse to "unbound" rather than erroring.
fn resolve_layout<'a>(
    instance: &'a PrinterInstance,
    material_layout: &[Option<SlotRef>],
) -> Vec<FilamentEntry<'a>> {
    material_layout
        .iter()
        .map(|maybe_slot_ref| match maybe_slot_ref {
            Some(slot_ref) => instance
                .extruders
                .get(slot_ref.extruder as usize)
                .and_then(|ext| ext.slots.get(slot_ref.slot as usize))
                .map(|slot| FilamentEntry {
                    slot: Some(slot),
                    extruder_index: slot_ref.extruder,
                })
                .unwrap_or(FilamentEntry {
                    slot: None,
                    extruder_index: 0,
                }),
            None => FilamentEntry {
                slot: None,
                extruder_index: 0,
            },
        })
        .collect()
}

/// Legacy fallback when no material layout is provided — fan out
/// one filament per `PrinterInstance` slot in extruder-major flat
/// order. Preserves the old "one filament per slot" semantics used
/// by non-slice callers (cascade trace UI) and unit tests that
/// don't care about plate state.
fn slot_layout(instance: &PrinterInstance) -> Vec<FilamentEntry<'_>> {
    let mut out = Vec::new();
    for (e_idx, extruder) in instance.extruders.iter().enumerate() {
        for slot in &extruder.slots {
            out.push(FilamentEntry {
                slot: Some(slot),
                extruder_index: e_idx as u8,
            });
        }
    }
    out
}

/// Errors from composing a slice-time cascade.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposeError {
    UnknownPrinterFragment(String),
    UnknownNozzleFragment { printer_slug: String, sku: String },
    UnknownBedFragment(String),
    UnknownFilamentFragment(String),
    UnknownProcessFragment(String),
    NoExtruders(String),
}

impl std::fmt::Display for ComposeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownPrinterFragment(s) => {
                write!(f, "no bundled printer fragment for slug `{s}`")
            }
            Self::UnknownNozzleFragment { printer_slug, sku } => {
                write!(
                    f,
                    "no bundled nozzle fragment for printer `{printer_slug}` SKU `{sku}`"
                )
            }
            Self::UnknownBedFragment(s) => write!(f, "no bundled bed fragment for slug `{s}`"),
            Self::UnknownFilamentFragment(s) => {
                write!(f, "no bundled filament fragment for slug `{s}`")
            }
            Self::UnknownProcessFragment(s) => {
                write!(f, "no bundled process fragment for slug `{s}`")
            }
            Self::NoExtruders(id) => {
                write!(f, "PrinterInstance `{id}` has no extruders")
            }
        }
    }
}

impl std::error::Error for ComposeError {}

/// View an instance as if its process were `qp` — the per-plate
/// quality-profile override. Returns the instance unchanged
/// (`Borrowed`) when `qp` is `None` or already equal; otherwise a
/// `Owned` clone with `quality_profile` swapped. The composer reads
/// `instance.quality_profile` to pick the process fragment, so callers
/// (slice, panel resolve, import baseline) pass the plate's effective
/// profile through this rather than the composer growing a parameter.
pub fn with_quality_profile<'a>(
    instance: &'a PrinterInstance,
    qp: Option<&str>,
) -> std::borrow::Cow<'a, PrinterInstance> {
    match qp {
        Some(qp) if qp != instance.quality_profile => {
            let mut owned = instance.clone();
            owned.quality_profile = qp.to_owned();
            std::borrow::Cow::Owned(owned)
        }
        _ => std::borrow::Cow::Borrowed(instance),
    }
}

/// Compose the slice-time cascade for `instance`.
///
/// `material_layout` is the per-material filament view: one entry
/// per libslic3r filament slot, where each entry's `Option<SlotRef>`
/// names the `PrinterInstance` slot that material is bound to.
/// The cascade's filament dimension (the length of the
/// `filament_diameter` / `filament_colour` / `filament_map` /
/// `flush_volumes_matrix` vectors) is sized from this slice's
/// length. Pass an empty slice to fall back to the legacy "one
/// filament per `PrinterInstance` slot" view used by non-slice
/// callers (cascade trace UI, schema preview).
///
/// `plate_overrides` becomes the highest-precedence layer: appended as an
/// `important` (override-tier) rule, so the resolver ranks it above every
/// authored rule regardless of specificity.
pub fn compose_cascade(
    instance: &PrinterInstance,
    material_layout: &[Option<SlotRef>],
) -> Result<Cascade, ComposeError> {
    if instance.extruders.is_empty() {
        return Err(ComposeError::NoExtruders(instance.id.clone()));
    }
    let filaments: Vec<FilamentEntry<'_>> = if material_layout.is_empty() {
        slot_layout(instance)
    } else {
        resolve_layout(instance, material_layout)
    };

    let mut rules: Vec<Rule> = Vec::new();

    // 0. Synthesized flush-purge defaults.
    //
    //    libslic3r's gcode pass validates
    //    `flush_volumes_matrix.size() == filament_count² × heads_count`
    //    (see GCode.cpp::append_full_config) and throws
    //    "Flush volumes matrix do not match to the correct size!"
    //    on mismatch. The hardcoded default in PrintConfig.cpp is a
    //    4×4 matrix (16 entries) — fine for 1..=4 filaments on a
    //    single head, broken for everything else.
    //
    //    Synthesize sane defaults sized to the instance topology
    //    (N² × H, off-diagonal 280, diagonal 0; plus length-H
    //    flush_multiplier of 1.0). Placed *before* the printer
    //    fragment so a vendor-shipped matrix (snappy's 84-off-diag
    //    snapmaker default) still wins on source-order tie-break.
    let flush_defaults = assemble_flush_defaults(instance, filaments.len());
    if !flush_defaults.is_empty() {
        rules.push(Rule {
            when: Predicate::default(),
            set: flush_defaults,
            source: SourceLocation {
                path: PathBuf::from("<flush-defaults>"),
                line: 1,
            },
            important: false,
        });
    }

    // 1. Printer fragment — machine globals only (per-extruder keys
    //    are deliberately absent here; they come from step 2).
    let printer = load_printer_fragment(&instance.printer_fragment_slug).ok_or_else(|| {
        ComposeError::UnknownPrinterFragment(instance.printer_fragment_slug.clone())
    })?;
    // Global predicate dimensions filament fragments may key on, needed
    // by step 4's per-slot resolve. `printer_model` is the machine
    // cascade's `printer_model` scalar (the same value `when.printer.model
    // = …` predicates match on — see registry hydration); `plate_type`
    // is the loaded bed's identity (= `when.plate.type`). Captured here,
    // before `printer.rules` is moved into `rules` below, so the per-slot
    // filament context is complete and printer/plate-keyed filament rules
    // actually fire at compose time.
    let printer_model = printer
        .rules
        .iter()
        .find_map(|r| r.set.get("printer_model").cloned())
        .unwrap_or_default();
    let plate_type = instance.bed.identity.clone();
    rules.extend(printer.rules);

    // 2. Bed fragment — looked up by `(printer_slug, bed_identity)`
    //    where the identity matches libslic3r's `curr_bed_type` enum
    //    value verbatim. Composed *before* the per-extruder nozzle layer
    //    so nozzle (the more specific hardware) wins the source-order
    //    tie-break. They share no keys today (bed sets only
    //    `curr_bed_type`), so this is a no-op for resolution; the order is
    //    what the cascade ladder's precedence display reflects.
    let bed = load_bed_fragment(&instance.printer_fragment_slug, &instance.bed.identity)
        .ok_or_else(|| {
            ComposeError::UnknownBedFragment(format!(
                "{}/{}",
                instance.printer_fragment_slug, instance.bed.identity,
            ))
        })?;
    rules.extend(bed.rules);

    // 3. Per-extruder nozzle fragments → vector assembly.
    //    Load one nozzle fragment per extruder using its
    //    `installed_nozzle.diameter` as the SKU. Then merge their
    //    scalar values into per-key vectors and synthesize one cascade
    //    rule whose set contains those vector strings.
    let nozzle_vectors = assemble_nozzle_vectors(instance)?;
    if !nozzle_vectors.is_empty() {
        rules.push(Rule {
            when: Predicate::default(),
            set: nozzle_vectors,
            source: SourceLocation {
                path: PathBuf::from("<extruder-vector-assembly>"),
                line: 1,
            },
            important: false,
        });
    }

    // 3b. Filament-map topology.
    //
    //    libslic3r's `GCodeProcessor::update_slice_warnings` indexes
    //    `m_filament_maps` by filament index — if the map is shorter
    //    than the highest used filament index, you get out-of-bounds
    //    UB (SIGSEGV in release). The default is `{1}` (length-1), so
    //    any multi-filament print blows up unless someone (in Orca's
    //    case, the PartPlate GUI) writes the right value.
    //
    //    We author it from the instance topology: for each slot (in
    //    flat extruder-major order, matching the filament dimension
    //    libslic3r expects), the value is the 1-based extruder index
    //    that slot belongs to. We also force
    //    `filament_map_mode = "Manual"` so libslic3r treats the value
    //    as authoritative rather than auto-rebalancing.
    let topology = assemble_filament_topology(&filaments);
    if !topology.is_empty() {
        rules.push(Rule {
            when: Predicate::default(),
            set: topology,
            source: SourceLocation {
                path: PathBuf::from("<filament-topology>"),
                line: 1,
            },
            important: false,
        });
    }

    // 4. Per-slot filament fragments → vector assembly.
    //    Walk slots in extruder-major flat order (matching the
    //    `filament_map` dimension libslic3r expects), load each
    //    slot's filament fragment, and zip the scalar values into
    //    per-key vector strings. Single-filament instances collapse
    //    to length-1 vectors which libslic3r accepts as scalars.
    //    Without this fan-out the toolchanger (U1) and AMS-fed
    //    (Bambi) printers both end up emitting `filament: 1` even
    //    with N slots bound — no tool changes in the output gcode.
    let filament_vectors =
        assemble_filament_vectors(instance, &filaments, &printer_model, &plate_type)?;
    if !filament_vectors.is_empty() {
        rules.push(Rule {
            when: Predicate::default(),
            set: filament_vectors,
            source: SourceLocation {
                path: PathBuf::from("<filament-vector-assembly>"),
                line: 1,
            },
            important: false,
        });
    }

    // 4b. Per-slot color synthesis.
    //
    //    `filament_colour` must be length-N matching `filament_diameter`:
    //    `apply_mm_segmentation` in PrintObjectSlice.cpp uses
    //    `filament_diameter.size()` as the extruder count, while
    //    `multi_material_segmentation_by_painting` sizes its inner
    //    per-layer vectors by `filament_colour.size() + 1`. A mismatch
    //    causes an out-of-bounds read on `segmentation[layer_id][i]` for
    //    `i >= filament_colour.size() + 1`, surfacing as a SIGSEGV deep
    //    in `get_extents` with a garbage-length expolygons vector.
    //
    //    The bundled filament fragments don't carry `filament_colour` at
    //    all, so the filament-tier fan-out can't produce the right
    //    length on its own. Synthesize from `PrinterInstance.slots[]
    //    .color`, falling back to "#F2754E" for unbound slots — the
    //    color value doesn't affect slicing correctness, only the vector
    //    length does.
    let colours = assemble_filament_colours(&filaments);
    if !colours.is_empty() {
        rules.push(Rule {
            when: Predicate::default(),
            set: colours,
            source: SourceLocation {
                path: PathBuf::from("<filament-colour-synthesis>"),
                line: 1,
            },
            important: false,
        });
    }

    // 5. Process fragment — printer-bound, looked up by
    //    `(printer_fragment_slug, quality_profile)`.
    let process = load_process_fragment(&instance.printer_fragment_slug, &instance.quality_profile)
        .ok_or_else(|| {
            ComposeError::UnknownProcessFragment(format!(
                "{}/{}",
                instance.printer_fragment_slug, instance.quality_profile,
            ))
        })?;
    rules.extend(process.rules);

    // 5b. Machine-setting overrides — the per-printer-instance tier for
    //     Printer-bucket keys, set from the printer panel and stored in the
    //     instance's `config_overrides` (alongside the `plugin.*` entries,
    //     which are filtered out here — they drive the plugin gate, not the
    //     engine config). `!important` so they win over the printer
    //     fragment's machine globals; pushed before plate overrides so an
    //     explicit plate override still wins on a (rare, cross-bucket) key
    //     collision.
    let machine_overrides: BTreeMap<String, String> = instance
        .config_overrides
        .iter()
        .filter(|(k, _)| !k.starts_with("plugin."))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    if !machine_overrides.is_empty() {
        rules.push(Rule {
            when: Predicate::default(),
            set: machine_overrides,
            source: SourceLocation {
                path: PathBuf::from("<machine-overrides>"),
                line: 1,
            },
            important: true,
        });
    }

    // The user/project/object override tiers are NOT baked here — they're
    // applied as the second phase by `cascade::resolve_with_overrides`, so
    // the trace can attribute each value to its winning tier and the panel
    // can offer "reset to cascade". Machine overrides (5b) stay baked: they
    // are the instance's own saved config, part of its authored cascade.
    Ok(Cascade { rules })
}

/// libslic3r's compiled-in default for every option, keyed by name.
/// Built once per vector assembly so a fragment that doesn't set a
/// per-filament / per-extruder key is padded with the *engine* default
/// at its vector position — never a sibling fragment's value. Empty when
/// the FFI isn't initialized (non-slice paths); the per-key fallback then
/// degrades to an empty string, same as a key libslic3r has no default for.
fn engine_default_map() -> std::collections::HashMap<String, String> {
    slic3r_ffi::option_defs()
        .into_iter()
        .filter_map(|d| d.default_serialized.map(|v| (d.key, v)))
        .collect()
}

/// Zip per-fragment scalar maps into one length-N vector string for
/// `key`. The load-bearing invariant: a fragment that doesn't set the key
/// gets `key`'s **engine default** at its position (the default's first
/// element when it itself serializes as a vector) — so one fragment's
/// value can never leak into another fragment's slot. (Padding with the
/// first fragment's value used to: a key set only by filament 0 bled into
/// every later filament that left it default.)
fn zip_vector(
    key: &str,
    per_fragment: &[BTreeMap<String, String>],
    engine_defaults: &std::collections::HashMap<String, String>,
) -> String {
    let default = engine_defaults
        .get(key)
        .map(|d| split_for_key(key, d).into_iter().next().unwrap_or_default())
        .unwrap_or_default();
    let values: Vec<String> = per_fragment
        .iter()
        .map(|m| m.get(key).cloned().unwrap_or_else(|| default.clone()))
        .collect();
    join_for_key(key, &values)
}

/// Walk the instance's extruders, load each one's nozzle fragment,
/// and zip the scalar values into per-extruder vector strings keyed
/// by the per-extruder option keys. libslic3r consumes per-extruder
/// vectors as semicolon-separated strings in the cascade `set` map.
fn assemble_nozzle_vectors(
    instance: &PrinterInstance,
) -> Result<BTreeMap<String, String>, ComposeError> {
    // Load each extruder's nozzle scalars.
    let mut per_extruder: Vec<BTreeMap<String, String>> =
        Vec::with_capacity(instance.extruders.len());
    for extruder in &instance.extruders {
        let sku = nozzle_sku_string(&extruder.installed_nozzle);
        let cascade =
            load_nozzle_fragment(&instance.printer_fragment_slug, &sku).ok_or_else(|| {
                ComposeError::UnknownNozzleFragment {
                    printer_slug: instance.printer_fragment_slug.clone(),
                    sku: sku.clone(),
                }
            })?;
        // A nozzle fragment is a single unconditional default rule
        // (the converter never emits [[rule]] blocks).
        let scalars = cascade
            .rules
            .into_iter()
            .next()
            .map(|r| r.set)
            .unwrap_or_default();
        per_extruder.push(scalars);
    }

    // Union of all keys seen across extruders → vector key set.
    let mut all_keys: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for scalars in &per_extruder {
        for k in scalars.keys() {
            all_keys.insert(k.clone());
        }
    }

    // For each key, build a length-N vector. A missing extruder entry is
    // padded with the engine default for that key (see `zip_vector`), so
    // one extruder's nozzle profile can't leak into another's slot.
    let engine_defaults = engine_default_map();
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    for key in all_keys {
        let joined = zip_vector(&key, &per_extruder, &engine_defaults);
        out.insert(key, joined);
    }

    Ok(out)
}

/// Walk the cascade's filament layout, load each one's filament
/// fragment, and zip the scalar values into per-key vector strings.
/// Mirrors [`assemble_nozzle_vectors`] but for filament-side keys
/// (`filament_diameter`, `filament_colour`, `filament_type`,
/// `filament_settings_id`, …). Filament position N in the vector
/// pairs with `filament_map[N]`'s extruder.
///
/// Entries without a bound slot — or whose slot has no
/// `filament_identity` — fall back to the instance's
/// `default_filament_fragment_slug`. Per-key vector positions a fragment
/// doesn't set are filled with that key's libslic3r **engine default**
/// (via `zip_vector`), keeping the vector length uniform without leaking
/// one fragment's value into another's slot.
///
/// Each fragment is resolved against a slot-scoped context (per
/// `docs/dev/settings-model.md` §5 "Per-slot vector-key assembly")
/// before vector-assembly: conditional `[[rule]]` blocks inside
/// fragments — e.g. `when.filament.type = "PETG" set.fan_speed = 30`
/// — match against THIS slot's bound filament, so the vector entry
/// reflects per-slot conditional behavior rather than just the
/// fragment's default rule.

fn assemble_filament_vectors(
    instance: &PrinterInstance,
    filaments: &[FilamentEntry<'_>],
    printer_model: &str,
    plate_type: &str,
) -> Result<BTreeMap<String, String>, ComposeError> {
    let mut per_filament: Vec<BTreeMap<String, String>> = Vec::new();
    for entry in filaments {
        let identity = entry
            .slot
            .and_then(|s| s.filament_identity.as_deref())
            .unwrap_or(&instance.default_filament_fragment_slug);
        // A slot may bind a bundled fragment slug OR a user-library
        // filament id; the latter resolves to its base fragment + the
        // user's filament-bucket overrides.
        let (slug, overrides) = resolve_filament_ref(identity);
        let cascade = load_filament_fragment(&slug)
            .ok_or_else(|| ComposeError::UnknownFilamentFragment(slug.clone()))?;
        // Resolve against this slot's *complete* context — the global
        // dimensions (printer.model, plate.type) plus this slot's
        // filament.* — so conditional rules in the fragment match
        // whether they key on `when.filament.*` OR `when.printer.model`
        // / `when.plate.type`. Falls through to the unconditional default
        // rule when no filament profile is registered for this slug.
        let slot_ctx = slot_filament_context(&slug, printer_model, plate_type);
        let mut scalars: BTreeMap<String, String> = resolve(&cascade, &slot_ctx)
            .into_iter()
            .map(|(k, v)| (k, v.value))
            .collect();
        // User overrides win over the base fragment's resolved scalars.
        scalars.extend(overrides);
        per_filament.push(scalars);
    }

    if per_filament.is_empty() {
        return Ok(BTreeMap::new());
    }

    // Union of all keys seen across filament entries.
    let mut all_keys: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for scalars in &per_filament {
        for k in scalars.keys() {
            all_keys.insert(k.clone());
        }
    }

    // Build length-N vector strings per key. A filament that doesn't set a
    // key is padded with that key's engine default (see `zip_vector`) — so
    // a value (or quirk) in one filament's profile can never reach
    // another filament's slot.
    let engine_defaults = engine_default_map();
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    for key in all_keys {
        let joined = zip_vector(&key, &per_filament, &engine_defaults);
        out.insert(key, joined);
    }

    Ok(out)
}

/// Default off-diagonal purge volume (mm³) — matches libslic3r's
/// PrintConfig.cpp default for `flush_volumes_matrix`. Per-vendor
/// overrides (snappy ships 84) replace this via the printer fragment.
const DEFAULT_FLUSH_OFF_DIAG_MM3: f32 = 280.0;

/// Synthesize `flush_volumes_matrix` (N² × H entries) and
/// `flush_multiplier` (length H of 1.0) sized to the cascade's
/// filament count. N = filament count (the libslic3r filament
/// dimension, derived from the materials list at slice time or
/// from the instance's slot count otherwise), H = extruder count
/// (= "heads" in libslic3r terminology — `flush_multiplier.size()`).
///
/// The matrix layout per libslic3r: H consecutive N×N blocks, each
/// flat row-major. Off-diagonal entries are
/// [`DEFAULT_FLUSH_OFF_DIAG_MM3`]; diagonal entries (same → same
/// filament) are zero. Returned as a comma-joined coFloats string.
fn assemble_flush_defaults(
    instance: &PrinterInstance,
    filament_count: usize,
) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let heads_count = instance.extruders.len();
    if filament_count == 0 || heads_count == 0 {
        return out;
    }
    let mut matrix: Vec<String> = Vec::with_capacity(filament_count * filament_count * heads_count);
    for _ in 0..heads_count {
        for row in 0..filament_count {
            for col in 0..filament_count {
                let v = if row == col {
                    0.0
                } else {
                    DEFAULT_FLUSH_OFF_DIAG_MM3
                };
                matrix.push(format!("{v}"));
            }
        }
    }
    out.insert("flush_volumes_matrix".to_owned(), matrix.join(","));
    let multipliers: Vec<String> = (0..heads_count).map(|_| "1".to_owned()).collect();
    out.insert("flush_multiplier".to_owned(), multipliers.join(","));
    out
}

/// Build the slot-scoped resolver context for a single filament slug.
/// Carries the global dimensions (`printer.model`, `plate.type`) plus
/// this slot's `filament.*` predicates, mirroring `SlicingContext`'s
/// `Context` impl so conditional rules inside a filament fragment fire
/// per-slot whether they key on filament, printer, or plate.
///
/// The global dimensions are essential: the bundled vendor filament
/// fragments carry per-printer overrides as `[[rule]]
/// when.printer.model = "Snapmaker U1"` blocks (e.g. the U1's bed temp +
/// nozzle temp). Resolving with only `filament.*` predicates — as an
/// earlier version did — silently dropped every such rule, leaving the
/// U1 with baseline filament values (and a cold bed). The per-slot
/// fan-out means we resolve N times (one per bound slot) to build the
/// per-slot vectors libslic3r wants; each resolve must see the full
/// context, not just the filament part.
///
/// Unknown slugs (e.g. an instance bound to a filament identity that
/// isn't in the registry yet) still get the global dimensions — the
/// fragment's unconditional default rule and any printer/plate-keyed
/// rules match; only `filament.*`-keyed conditionals silently don't.
fn slot_filament_context(slug: &str, printer_model: &str, plate_type: &str) -> MapContext {
    let mut ctx = MapContext::new();
    if !printer_model.is_empty() {
        ctx.set("printer.model", printer_model);
    }
    if !plate_type.is_empty() {
        ctx.set("plate.type", plate_type);
    }
    if let Some(profile) = filament::lookup(slug) {
        ctx.set("filament.type", profile.base_type);
        ctx.set("filament.name", profile.identity);
        if let Some(vendor) = profile.vendor {
            ctx.set("filament.vendor", vendor);
        }
        if let Some(color) = profile.color {
            ctx.set("filament.color", color);
        }
    }
    ctx
}

/// Resolve a slot's `filament_identity` (always a bundled slug) into the
/// fragment slug to load plus the user overrides to fold on top. When the
/// user has edited this filament in place, their override profile (keyed by
/// the same slug) supplies the overrides; otherwise there are none.
fn resolve_filament_ref(identity: &str) -> (String, BTreeMap<String, String>) {
    let overrides = filament::library::lookup(identity)
        .map(|uf| uf.overrides)
        .unwrap_or_default();
    (identity.to_owned(), overrides)
}

/// Resolve a bundled filament fragment's scalar values with no
/// printer/plate context — the *base* values the filament settings editor
/// shows beneath any user override. Printer/plate-conditional fragment
/// rules (the U1 bed-temp block) aren't reflected here; the editor shows
/// the unconditional + filament-typed base, which is what a per-filament
/// override is authored against.
pub fn resolve_base_scalars(slug: &str) -> BTreeMap<String, String> {
    let Some(cascade) = load_filament_fragment(slug) else {
        return BTreeMap::new();
    };
    let ctx = slot_filament_context(slug, "", "");
    resolve(&cascade, &ctx)
        .into_iter()
        .map(|(k, v)| (k, v.value))
        .collect()
}

/// Default per-slot color for unbound slots — Orca's bundled
/// bambu-pla-basic default. The value is cosmetic; the *length* of
/// the assembled vector is the load-bearing part for slicing
/// correctness (see `apply_mm_segmentation` index OOB).
const DEFAULT_FILAMENT_COLOUR: &str = "#F2754E";

/// Synthesize the `filament_colour` vector from the cascade's
/// filament layout. Each entry pulls the color from its bound
/// `PrinterInstance` slot, or [`DEFAULT_FILAMENT_COLOUR`] when the
/// slot is unbound / has no color set.
fn assemble_filament_colours(filaments: &[FilamentEntry<'_>]) -> BTreeMap<String, String> {
    let values: Vec<String> = filaments
        .iter()
        .map(|entry| {
            entry
                .slot
                .and_then(|s| s.color.clone())
                .unwrap_or_else(|| DEFAULT_FILAMENT_COLOUR.to_owned())
        })
        .collect();
    let mut out = BTreeMap::new();
    if !values.is_empty() {
        out.insert(
            "filament_colour".to_owned(),
            join_for_key("filament_colour", &values),
        );
    }
    out
}

/// Synthesize `filament_map` + `filament_map_mode` from the
/// cascade's filament layout. Each entry contributes the 1-based
/// extruder index it should feed from. Empty when the layout is
/// empty (the caller skips the rule in that case).
fn assemble_filament_topology(filaments: &[FilamentEntry<'_>]) -> BTreeMap<String, String> {
    let mut filament_map: Vec<String> = Vec::new();
    for entry in filaments {
        filament_map.push((entry.extruder_index as usize + 1).to_string());
    }
    let mut out = BTreeMap::new();
    if !filament_map.is_empty() {
        out.insert("filament_map".to_owned(), filament_map.join(","));
        out.insert("filament_map_mode".to_owned(), "Manual".to_owned());
    }
    out
}

/// Join a list of per-extruder/per-slot scalar values into the
/// vector-string libslic3r's `ConfigOptionVector::deserialize` expects.
///
/// Almost every libslic3r vector option uses `,` as the separator
/// (coFloats, coInts, coBools, coPercents, coEnums, coFloatsOrPercents,
/// coPoints) — that's what `ConfigOptionVector::deserialize` and
/// `ConfigOptionEnumsGeneric::deserialize` parse. `coStrings` is the
/// exception: it expects `;` as the separator, with cstyle escaping
/// (`"…"` quoting + `\"`/`\\`/`\r`/`\n` escapes) applied to entries
/// containing whitespace or quotes. See
/// `external/OrcaSlicer/src/libslic3r/Config.cpp::escape_strings_cstyle`.
///
/// When the schema isn't available (FFI not initialized in some unit
/// tests) or the key isn't in libslic3r's option universe, fall back
/// to comma-join — matches the pre-existing behavior so tests stay
/// deterministic.
pub fn join_for_key(key: &str, values: &[String]) -> String {
    let ty = schema_by_key(key).map(|s| s.ty);
    if matches!(ty, Some(OptType::Strings)) {
        values
            .iter()
            .map(|v| escape_string_cstyle(v))
            .collect::<Vec<_>>()
            .join(";")
    } else {
        values.join(",")
    }
}

/// Inverse of [`join_for_key`]: split a serialized vector value back into
/// its elements. For string vectors this is cstyle-aware — a `;` inside a
/// quoted element (e.g. the `;`-comments in `filament_start_gcode`) is NOT
/// a separator — so naive `split(';')` would miscount. Non-string vectors
/// split on `,`. An empty value yields an empty vec.
///
/// `split_for_key(k, &join_for_key(k, parts))` round-trips `parts` (modulo
/// libslic3r's escaping normalization).
pub fn split_for_key(key: &str, value: &str) -> Vec<String> {
    let ty = schema_by_key(key).map(|s| s.ty);
    if matches!(ty, Some(OptType::Strings)) {
        unescape_strings_cstyle(value)
    } else if value.is_empty() {
        Vec::new()
    } else {
        value.split(',').map(str::to_string).collect()
    }
}

/// Rust port of libslic3r's `unescape_strings_cstyle` (Config.cpp). Splits
/// a `;`-delimited list into elements, honoring `"…"` quoting and the
/// `\"`/`\\`/`\r`/`\n` escapes [`escape_string_cstyle`] emits, so a `;`
/// inside a quoted value stays part of that value. The exact inverse of
/// the escape path.
fn unescape_strings_cstyle(s: &str) -> Vec<String> {
    if s.is_empty() {
        return Vec::new();
    }
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut out = Vec::new();
    let mut i = 0;
    loop {
        let mut current = String::new();
        if i < n && chars[i] == '"' {
            // Quoted element: consume until the unescaped closing quote.
            i += 1;
            while i < n && chars[i] != '"' {
                if chars[i] == '\\' && i + 1 < n {
                    i += 1;
                    match chars[i] {
                        'n' => current.push('\n'),
                        'r' => current.push('\r'),
                        c => current.push(c),
                    }
                } else {
                    current.push(chars[i]);
                }
                i += 1;
            }
            if i < n {
                i += 1; // skip closing quote
            }
        } else {
            // Bare element: up to the next `;`.
            while i < n && chars[i] != ';' {
                current.push(chars[i]);
                i += 1;
            }
        }
        out.push(current);
        if i >= n {
            break;
        }
        i += 1; // skip the `;` separator
    }
    out
}

/// Rust port of libslic3r's `escape_string_cstyle` (Config.cpp:49). A
/// value is quoted iff it contains a character that would otherwise
/// confuse the `;`-delimited parser: space, tab, backslash, quote, CR,
/// LF. Inside the quotes, `"` and `\` are backslash-escaped; CR/LF
/// become the literal `\r`/`\n` digraphs. Non-quoting values pass
/// through verbatim.
fn escape_string_cstyle(s: &str) -> String {
    let needs_quote = s
        .bytes()
        .any(|b| matches!(b, b' ' | b'\t' | b'\\' | b'"' | b'\r' | b'\n'));
    if !needs_quote {
        return s.to_owned();
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' | '"' => {
                out.push('\\');
                out.push(c);
            }
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// Format a NozzleSku for fragment lookup. Returns the diameter
/// string verbatim (matches the on-disk `nozzles/<diameter>.toml`
/// filename convention). Future: incorporate material when we
/// author hotend-material-specific nozzle files.
fn nozzle_sku_string(nozzle: &crate::core::printer::NozzleSku) -> String {
    nozzle.diameter.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::printer::instance_registry::RegistryGuard;
    use crate::core::printer::lookup_instance;

    #[test]
    fn escape_string_cstyle_passes_simple_values_through() {
        assert_eq!(escape_string_cstyle("PLA"), "PLA");
        assert_eq!(escape_string_cstyle("#F2754E"), "#F2754E");
        assert_eq!(escape_string_cstyle(""), "");
    }

    #[test]
    fn escape_string_cstyle_quotes_whitespace_and_specials() {
        // Space, tab, backslash, quote, CR, LF all trigger quoting; quote and
        // backslash get backslash-escaped inside; CR/LF become \r / \n.
        assert_eq!(escape_string_cstyle("Generic PLA"), "\"Generic PLA\"");
        assert_eq!(escape_string_cstyle("a\tb"), "\"a\tb\"");
        assert_eq!(escape_string_cstyle("a\"b"), "\"a\\\"b\"");
        assert_eq!(escape_string_cstyle("a\\b"), "\"a\\\\b\"");
        assert_eq!(escape_string_cstyle("a\nb"), "\"a\\nb\"");
        assert_eq!(escape_string_cstyle("a\rb"), "\"a\\rb\"");
    }

    #[test]
    fn unescape_strings_cstyle_inverts_the_escape() {
        // Bare values, quoted-with-space, embedded `;`, and the escape
        // digraphs all round-trip back to the original element list. Built
        // via escape_string_cstyle directly so the test needs no schema.
        let cases: Vec<Vec<String>> = vec![
            vec!["PLA".into(), "PETG".into()],
            vec!["#FFFFFF".into(), "#DE4343".into()],
            vec!["Generic PLA".into(), "eSUN".into()],
            // The filament_start_gcode shape: a `;`-comment inside one element.
            vec!["; outer\nM104 S200".into(), "; inner\nM104 S210".into()],
            vec!["a\"b".into(), "c\\d".into()],
        ];
        for parts in cases {
            let joined = parts
                .iter()
                .map(|v| escape_string_cstyle(v))
                .collect::<Vec<_>>()
                .join(";");
            assert_eq!(
                unescape_strings_cstyle(&joined),
                parts,
                "round-trip failed for {parts:?} (joined: {joined:?})",
            );
        }
    }

    #[test]
    fn split_for_key_counts_quoted_gcode_as_one_element() {
        let _ = slic3r_ffi::init(None, 3); // split_for_key consults the schema
                                           // A single gcode element with `;`-comments must count as ONE element,
                                           // not split on the embedded `;` — the bug that corrupted
                                           // filament_start_gcode during length normalization.
        let one = join_for_key(
            "filament_start_gcode",
            &["; start\nM104 S200 ; set temp".into()],
        );
        assert_eq!(split_for_key("filament_start_gcode", &one).len(), 1);
        // Numeric vectors split on `,`.
        assert_eq!(
            split_for_key("filament_diameter", "1.75,1.75,1.75"),
            vec!["1.75", "1.75", "1.75"],
        );
        // Empty value → no elements.
        assert!(split_for_key("filament_diameter", "").is_empty());
    }

    #[test]
    fn compose_bambi_yields_printer_nozzle_bed_filament_process_layers() {
        let _registry = RegistryGuard::acquire();
        let bambi = lookup_instance("bambi").expect("bambi present");
        let cascade = compose_cascade(&bambi, &[]).expect("compose");

        // Each layer contributes at least one rule. With 5 layers + nozzle
        // assembly + no plate overrides → ≥ 6 rules (printer, nozzle,
        // bed, filament, process).
        assert!(
            cascade.rules.len() >= 5,
            "expected ≥ 5 rules, got {}",
            cascade.rules.len(),
        );

        let all_keys: std::collections::BTreeSet<&String> =
            cascade.rules.iter().flat_map(|r| r.set.keys()).collect();
        assert!(
            all_keys.contains(&"printable_height".to_owned()),
            "printer-bucket key missing"
        );
        assert!(
            all_keys.contains(&"nozzle_diameter".to_owned()),
            "per-extruder nozzle key missing from composition"
        );
        assert!(
            all_keys.contains(&"curr_bed_type".to_owned()),
            "bed-fragment key missing"
        );
        assert!(
            all_keys.contains(&"nozzle_temperature".to_owned()),
            "filament-bucket key missing"
        );
        assert!(
            all_keys.contains(&"layer_height".to_owned()),
            "process-bucket key missing"
        );
    }

    #[test]
    fn filament_vector_typo_key_does_not_zero_first_filament() {
        // Regression: filament 0 (generic-pla-silk) sits ahead of a
        // filament (generic-pla) that historically carried the Orca typo
        // `nozzle_temperature_intial_layer`. The typo is now folded to
        // canonical at import, so the bundled fragments carry one
        // spelling and the per-filament vector can't be zeroed by a
        // sibling. Pre-fix this produced a 0 °C first layer on filament 0.
        let _registry = RegistryGuard::acquire();
        let _ = slic3r_ffi::init(None, 3);
        let mut bambi = lookup_instance("bambi").expect("bambi present");
        bambi.extruders[0].slots[0].filament_identity = Some("generic-pla-silk".into());
        bambi.extruders[0].slots[1].filament_identity = Some("generic-pla".into());
        let cascade = compose_cascade(&bambi, &[]).expect("compose");
        let vec_rule = cascade
            .rules
            .iter()
            .find(|r| r.source.path.to_str() == Some("<filament-vector-assembly>"))
            .expect("filament vector rule present");
        let joined = vec_rule
            .set
            .get("nozzle_temperature_initial_layer")
            .expect("initial-layer key present in the assembled vector");
        let first = joined.split(',').next().unwrap_or("");
        assert!(
            first != "0" && !first.is_empty(),
            "filament 0 first-layer temp must carry silk's real value, got vector {joined:?}",
        );
        assert!(
            vec_rule.set.get("nozzle_temperature_intial_layer").is_none(),
            "the typo key must not survive as a separate vector",
        );
    }

    #[test]
    fn zip_vector_pads_missing_with_engine_default_not_a_sibling() {
        // The general cross-filament guard: filament 0 sets a key, filament 1
        // doesn't. Filament 1's vector slot must be the ENGINE default, never
        // filament 0's value — otherwise one filament's profile leaks into
        // another's slot.
        let _ = slic3r_ffi::init(None, 3);
        let f0: BTreeMap<String, String> = [(
            "nozzle_temperature_initial_layer".to_string(),
            "200".to_string(),
        )]
        .into_iter()
        .collect();
        let f1: BTreeMap<String, String> = BTreeMap::new();
        let defaults: std::collections::HashMap<String, String> = [(
            "nozzle_temperature_initial_layer".to_string(),
            "220".to_string(),
        )]
        .into_iter()
        .collect();
        let v = zip_vector(
            "nozzle_temperature_initial_layer",
            &[f0, f1],
            &defaults,
        );
        assert_eq!(
            v, "200,220",
            "the unset filament must get the engine default (220), not filament 0's 200",
        );
    }

    #[test]
    fn user_filament_override_reaches_assembled_vector() {
        // A slot bound to a bundled slug whose filament the user has edited
        // in place resolves to the fragment + the user's override; the
        // override must land at that slot's position in the assembled
        // per-filament vector.
        let _registry = RegistryGuard::acquire();
        let _ = slic3r_ffi::init(None, 3);
        filament::library::set_override("generic-pla", "nozzle_temperature".into(), Some("250".into()))
            .expect("generic-pla is bundled");

        let mut bambi = lookup_instance("bambi").expect("bambi present");
        bambi.extruders[0].slots[0].filament_identity = Some("generic-pla".into());
        let cascade = compose_cascade(&bambi, &[]).expect("compose");
        let fil = cascade
            .rules
            .iter()
            .find(|r| r.source.path.to_str() == Some("<filament-vector-assembly>"))
            .expect("filament-vector rule present");
        let joined = fil
            .set
            .get("nozzle_temperature")
            .expect("nozzle_temperature assembled");
        assert_eq!(
            joined.split(',').next(),
            Some("250"),
            "slot 0's edited-filament override must win, got vector {joined:?}",
        );

        filament::library::revert("generic-pla");
    }

    #[test]
    fn nozzle_vector_assembly_replicates_for_u1() {
        // Snappy has 4 extruders all bound to 0.4 SS — the assembled
        // nozzle_diameter must be "0.4,0.4,0.4,0.4" — libslic3r's
        // ConfigOptionVector deserialize splits on ','.
        let _registry = RegistryGuard::acquire();
        let snappy = lookup_instance("snappy").expect("snappy present");
        let cascade = compose_cascade(&snappy, &[]).expect("compose");

        // Find the synthesized extruder-vector rule.
        let vector_rule = cascade
            .rules
            .iter()
            .find(|r| r.source.path.to_string_lossy() == "<extruder-vector-assembly>")
            .expect("extruder-vector rule present");
        let diameter = vector_rule
            .set
            .get("nozzle_diameter")
            .expect("nozzle_diameter assembled");
        assert_eq!(diameter, "0.4,0.4,0.4,0.4");
    }

    #[test]
    fn u1_filament_fragment_printer_rule_fires_at_compose_time() {
        // Regression for the U1 cold-bed bug. The snapmaker-pla fragment
        // carries its U1 overrides as a `[[rule]] when.printer.model =
        // "Snapmaker U1"` block (nozzle_temperature 220, hot_plate_temp
        // 55, textured_plate_temp 55 vs baseline 210 / 60 / 0). The
        // compose-time per-slot resolution must see `printer.model` so
        // that rule fires — without it the baseline leaks through and the
        // bed never heats (the active plate's curr_bed_type = "Textured
        // PEI Plate" reads textured_plate_temp).
        //
        // Bind all 4 U1 toolheads to snapmaker-pla explicitly: the
        // bundled `snappy` fixture defaults to generic-pla, which has no
        // U1 rule, so the binding must be set to exercise the
        // printer-keyed fragment rule.
        let _registry = RegistryGuard::acquire();
        let mut snappy = lookup_instance("snappy").expect("snappy present");
        assert_eq!(snappy.bed.identity, "Textured PEI Plate");
        for ext in snappy.extruders.iter_mut() {
            for slot in ext.slots.iter_mut() {
                slot.filament_identity = Some("snapmaker-pla".to_owned());
            }
        }

        let cascade = compose_cascade(&snappy, &[]).expect("compose");
        let fil = cascade
            .rules
            .iter()
            .find(|r| r.source.path.to_string_lossy() == "<filament-vector-assembly>")
            .expect("filament-vector rule present");

        // 4 slots all snapmaker-pla → length-4 vectors of the U1 value.
        assert_eq!(
            fil.set.get("nozzle_temperature").map(String::as_str),
            Some("220,220,220,220"),
            "U1 rule should override nozzle_temperature (220), not baseline 210",
        );
        assert_eq!(
            fil.set.get("hot_plate_temp").map(String::as_str),
            Some("55,55,55,55"),
            "U1 rule should override hot_plate_temp (55), not baseline 60",
        );
        assert_eq!(
            fil.set.get("textured_plate_temp").map(String::as_str),
            Some("55,55,55,55"),
            "U1 rule should set textured_plate_temp (55) — the bed-temp fix; \
             curr_bed_type = Textured PEI Plate reads this key",
        );
    }

    #[test]
    fn nozzle_vector_assembly_yields_single_value_for_a1_mini() {
        let _registry = RegistryGuard::acquire();
        let bambi = lookup_instance("bambi").expect("bambi present");
        let cascade = compose_cascade(&bambi, &[]).expect("compose");
        let vector_rule = cascade
            .rules
            .iter()
            .find(|r| r.source.path.to_string_lossy() == "<extruder-vector-assembly>")
            .expect("extruder-vector rule present");
        let diameter = vector_rule
            .set
            .get("nozzle_diameter")
            .expect("diameter present");
        // A1 mini has 1 extruder → no semicolons in the vector string.
        assert_eq!(diameter, "0.4");
    }

    /// Parity guard for C-1: applying overrides as the two-phase *project
    /// tier* (`resolve_with_overrides`) must yield byte-identical resolved
    /// values to baking them into the cascade as the Layer-6 `!important`
    /// rule (the pre-C-1 mechanism). This is the invariant that makes the
    /// live-path swap a behavioral no-op for existing project overrides —
    /// the new user/object tiers are then purely additive.
    #[test]
    fn project_tier_matches_baked_important_rule() {
        use crate::core::cascade::{resolve_with_overrides, FlatOverrides, OverrideTiers};
        let _registry = RegistryGuard::acquire();
        let bambi = lookup_instance("bambi").expect("bambi present");

        let mut overrides = BTreeMap::new();
        overrides.insert("layer_height".to_owned(), "0.12".to_owned());
        overrides.insert("bed_temp".to_owned(), "48".to_owned());
        overrides.insert("sparse_infill_density".to_owned(), "42%".to_owned());
        // An override-only key the fragments never set — exercises the
        // tier path's "add a key the cascade didn't have" branch.
        overrides.insert("brim_width".to_owned(), "3".to_owned());

        let ctx = slot_filament_context("generic-pla", "Bambu Lab A1 mini", "Textured PEI Plate");

        let authored = compose_cascade(&bambi, &[]).expect("compose authored");

        // OLD mechanism: overrides baked into the cascade as a trailing
        // `!important` rule (the pre-C-1 Layer 6 the composer no longer emits).
        let mut baked = authored.clone();
        baked.rules.push(Rule {
            when: Predicate::default(),
            set: overrides.clone(),
            source: SourceLocation {
                path: PathBuf::from("<plate-overrides>"),
                line: 1,
            },
            important: true,
        });
        let old = resolve(&baked, &ctx);

        // NEW path: authored cascade + overrides applied as the project tier.
        let tiers = OverrideTiers {
            user: vec![],
            project: vec![FlatOverrides {
                source: SourceLocation {
                    path: PathBuf::from("<project-overrides>"),
                    line: 1,
                },
                entries: overrides.clone(),
            }],
            object: None,
        };
        let new = resolve_with_overrides(&authored, &tiers, &ctx);

        let old_keys: std::collections::BTreeSet<&String> = old.keys().collect();
        let new_keys: std::collections::BTreeSet<&String> = new.keys().collect();
        assert_eq!(old_keys, new_keys, "resolved key sets diverge");

        for (k, ov) in &old {
            assert_eq!(
                &new[k].value, &ov.value,
                "resolved value for `{k}` diverges (baked={}, tier={})",
                ov.value, new[k].value,
            );
        }
        // Sanity: the overrides actually won in the tier path.
        assert_eq!(new["layer_height"].value, "0.12");
        assert_eq!(new["brim_width"].value, "3");
    }

    #[test]
    fn missing_nozzle_fragment_errors_with_useful_message() {
        let _registry = RegistryGuard::acquire();
        let mut bambi = lookup_instance("bambi").expect("bambi present");
        bambi.extruders[0].installed_nozzle.diameter = "0.9".to_string(); // not bundled
        let err = compose_cascade(&bambi, &[]).unwrap_err();
        assert!(
            matches!(&err, ComposeError::UnknownNozzleFragment { sku, .. } if sku == "0.9"),
            "got {err:?}",
        );
    }

    #[test]
    fn slot_filament_context_populates_predicates_from_lookup() {
        use crate::core::cascade::resolver::Context;
        // A bundled filament with known shape — generic-pla is shipped
        // by the converter under profiles/generic/filament/.
        let ctx = slot_filament_context("generic-pla", "Snapmaker U1", "Textured PEI Plate");
        assert_eq!(ctx.predicate_value("filament.type"), Some("PLA"));
        // filament.name carries the FilamentProfile.identity string,
        // not the slug — the bundled generic PLA is labeled "Generic PLA".
        assert!(ctx.predicate_value("filament.name").is_some());
        // The global dimensions must also be present so a fragment's
        // `when.printer.model` / `when.plate.type` rules fire at compose
        // time (regression guard for the U1 bed-temp bug).
        assert_eq!(ctx.predicate_value("printer.model"), Some("Snapmaker U1"));
        assert_eq!(
            ctx.predicate_value("plate.type"),
            Some("Textured PEI Plate")
        );
    }

    #[test]
    fn slot_filament_context_unknown_slug_keeps_global_dimensions() {
        use crate::core::cascade::resolver::Context;
        // Unknown slug — no `filament.*` predicates, but the global
        // dimensions still land so printer/plate-keyed rules match and
        // the unconditional default rule does too.
        let ctx = slot_filament_context(
            "not-a-real-filament-ever",
            "Snapmaker U1",
            "Textured PEI Plate",
        );
        assert!(ctx.predicate_value("filament.type").is_none());
        assert!(ctx.predicate_value("filament.name").is_none());
        assert_eq!(ctx.predicate_value("printer.model"), Some("Snapmaker U1"));
        assert_eq!(
            ctx.predicate_value("plate.type"),
            Some("Textured PEI Plate")
        );
    }

    #[test]
    fn slot_filament_context_omits_empty_global_dimensions() {
        use crate::core::cascade::resolver::Context;
        // Empty printer_model/plate_type (e.g. a non-slice caller that
        // doesn't have them) must not set blank predicates — a rule
        // keyed on `printer.model = ""` should never match.
        let ctx = slot_filament_context("generic-pla", "", "");
        assert!(ctx.predicate_value("printer.model").is_none());
        assert!(ctx.predicate_value("plate.type").is_none());
    }

    #[test]
    fn per_slot_fragment_resolution_fires_conditional_rules() {
        // Hand-build a cascade with one default rule + one conditional
        // rule keyed on filament.type. Resolve against two different
        // slot contexts (PLA vs PETG) and confirm the conditional
        // rule fires for each context's matching filament type — this
        // is the per-slot fan-out behavior that lets a future vendor
        // filament fragment carry `when.filament.type` rules safely.
        use crate::core::cascade::loader::parse_cascade_str;
        use std::path::Path;

        let rules = parse_cascade_str(
            "\
[[rule]]
set.fan_speed = 50

[[rule]]
when.filament.type = \"PETG\"
set.fan_speed = 30
",
            Path::new("synthetic.toml"),
        )
        .unwrap();
        let cascade = Cascade { rules };

        // Resolve against a PLA-typed context: only the default rule
        // matches → fan_speed = 50.
        let mut pla_ctx = MapContext::new();
        pla_ctx.set("filament.type", "PLA");
        let pla_scalars: BTreeMap<String, String> = resolve(&cascade, &pla_ctx)
            .into_iter()
            .map(|(k, v)| (k, v.value))
            .collect();
        assert_eq!(pla_scalars.get("fan_speed").map(String::as_str), Some("50"));

        // Resolve against a PETG-typed context: the conditional rule
        // matches and wins on higher specificity → fan_speed = 30.
        let mut petg_ctx = MapContext::new();
        petg_ctx.set("filament.type", "PETG");
        let petg_scalars: BTreeMap<String, String> = resolve(&cascade, &petg_ctx)
            .into_iter()
            .map(|(k, v)| (k, v.value))
            .collect();
        assert_eq!(
            petg_scalars.get("fan_speed").map(String::as_str),
            Some("30")
        );
    }

    #[test]
    fn missing_printer_fragment_errors() {
        let _registry = RegistryGuard::acquire();
        let mut bambi = lookup_instance("bambi").expect("bambi present");
        bambi.printer_fragment_slug = "ghost".into();
        let err = compose_cascade(&bambi, &[]).unwrap_err();
        assert_eq!(err, ComposeError::UnknownPrinterFragment("ghost".into()));
    }
}
