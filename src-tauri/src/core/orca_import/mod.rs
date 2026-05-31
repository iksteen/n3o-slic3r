//! OrcaSlicer / Bambu Studio `.3mf` **project** import (PR-9-6).
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

use serde_json::Value;
use slic3r_ffi::{bucket_of, OptBucket};

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
    /// string, or a comma-joined list. A vector whose libslic3r type
    /// actually joins with `;` will compare unequal to our `,`-joined
    /// baseline and be kept as an override — safe (the foreign value
    /// still wins), occasionally non-minimal; refine via the FFI option
    /// type later.
    pub fn canonical(&self, key: &str) -> Option<String> {
        Some(match self.settings.get(key)? {
            Value::String(s) => s.clone(),
            Value::Array(a) => a
                .iter()
                .map(|x| match x {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .collect::<Vec<_>>()
                .join(","),
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
}

/// Where each settings key lands when importing.
#[derive(Debug, Default)]
pub struct KeyPartition {
    /// Process-bucket keys → candidate project overrides (delta path).
    pub process: Vec<String>,
    /// Filament-bucket keys → candidate filament overrides (delta path).
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

/// The bundled printer an imported project binds to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrinterMatch {
    /// Catalog identity bound to (always set when the catalog is
    /// non-empty — a fallback still picks one).
    pub identity: String,
    /// True when `printer_model` matched no bundled printer and we fell
    /// back. Flagged in the report; machine settings are dropped
    /// regardless, so a fallback bind is safe.
    pub fallback: bool,
}

/// Map a BBS/Orca `printer_model` to a bundled printer: exact match on
/// the model name, else the first catalog entry flagged as a fallback.
/// `None` only when no printers are bundled.
pub fn infer_printer(printer_model: Option<&str>) -> Option<PrinterMatch> {
    let catalog = crate::core::profile_library::printer_catalog();
    if let Some(model) = printer_model {
        if let Some(e) = catalog.iter().find(|e| e.profile.model == model) {
            return Some(PrinterMatch {
                identity: e.identity.clone(),
                fallback: false,
            });
        }
    }
    catalog.first().map(|e| PrinterMatch {
        identity: e.identity.clone(),
        fallback: true,
    })
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
    /// Process/Filament keys we couldn't read a value for (skipped).
    pub unreadable: Vec<String>,
}

/// Compute the overrides to import: for each **Process** + **Filament**
/// key (machine keys are already excluded by [`partition`]), keep it
/// only when the foreign value differs from `baseline` — our cascade
/// resolved (libslic3r key → value) for the bound printer / filament /
/// process. Keys that match the baseline are redundant and dropped, so
/// the imported project's override set stays minimal and the cascade
/// readable.
///
/// When in doubt the key is kept (an absent baseline entry counts as a
/// difference): a redundant override is harmless, a *missing* one would
/// silently lose the project's setting.
pub fn compute_overrides(
    foreign: &OrcaProjectSettings,
    partition: &KeyPartition,
    baseline: &BTreeMap<String, String>,
) -> OverrideOutcome {
    let mut out = OverrideOutcome::default();
    for key in partition.process.iter().chain(partition.filament.iter()) {
        let Some(value) = foreign.canonical(key) else {
            out.unreadable.push(key.clone());
            continue;
        };
        match baseline.get(key) {
            Some(base) if *base == value => out.redundant.push(key.clone()),
            _ => {
                out.overrides.insert(key.clone(), value);
            }
        }
    }
    out
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
    fn infers_a_bundled_printer_and_falls_back_for_unknown() {
        // The fixture targets the A1 mini, which we ship → exact bind.
        let m = infer_printer(Some("Bambu Lab A1 mini")).expect("catalog non-empty");
        assert!(!m.fallback, "A1 mini should match exactly");
        let entry = crate::core::profile_library::printer_catalog_lookup(&m.identity)
            .expect("bound identity is in the catalog");
        assert_eq!(entry.profile.model, "Bambu Lab A1 mini");

        // An unknown model falls back (still binds something, flagged).
        let f = infer_printer(Some("Frobozz MagiPrint 9000")).expect("catalog non-empty");
        assert!(f.fallback, "unknown model must fall back");
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
        let s = fourcolor_project_settings();
        // layer_height is a scalar string — no comma.
        let lh = s.canonical("layer_height").expect("layer_height present");
        assert!(!lh.contains(','), "scalar should not be comma-joined: {lh}");
        // filament_settings_id is a 4-element list → comma-joined (3 commas).
        let f = s.canonical("filament_settings_id").expect("present");
        assert_eq!(
            f.matches(',').count(),
            3,
            "four elements → three commas: {f}"
        );
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

        let out = compute_overrides(&s, &p, &baseline);

        // The pinned key matched the baseline → redundant, not overridden.
        assert!(out.redundant.contains(&pinned));
        assert!(!out.overrides.contains_key(&pinned));
        // The rest differ → overrides.
        assert!(!out.overrides.is_empty());
        // A machine key is never an override (partition dropped it).
        assert!(!out.overrides.contains_key("machine_start_gcode"));
        // Every Process+Filament key is accounted for.
        let seen = out.overrides.len() + out.redundant.len() + out.unreadable.len();
        assert_eq!(seen, p.process.len() + p.filament.len());
    }
}
