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
use std::path::Path;

use serde_json::Value;
use slic3r_ffi::{bucket_of, OptBucket};

use crate::core::printer::{list_instances, lookup, PrinterInstance};
use crate::core::project::model::Project;
use crate::core::scene::state::{MeshId, NewSceneObject};
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
    /// Printer/machine settings dropped (owned by the bound printer).
    pub settings_machine_dropped: usize,
    /// Keys with no home in our model.
    pub settings_unmapped: usize,
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

fn register_obj(project: &mut Project, mesh_ids: &[MeshId], obj: &ProjectObject) {
    project.register_object(NewSceneObject {
        mesh: mesh_ids[obj.mesh_idx],
        transform: obj.transform,
        name: obj.name.clone(),
        visible: true,
        extruder_id: obj.extruder_id,
        parent: None,
        group_id: obj.group_id,
    });
}

/// Import an OrcaSlicer / Bambu Studio `.3mf` **project** into a fresh
/// n3o `Project`. Geometry + per-object/plate structure come from the
/// existing 3MF reader (which applies `model_settings.config`); the
/// project's `project_settings.config` settings are carried as overrides
/// (Process/Filament only — machine keys dropped). The plate(s) bind to
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

    // Fresh project (one default plate). Bind plate 1 before registering
    // objects so register_object's material→slot auto-bind sees the
    // instance.
    let mut project = Project::new();
    project.file_metadata = file_metadata;
    if let Some(id) = &instance_id {
        project.plates[0].printer_instance_id = Some(id.clone());
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
    let mut object_count = 0usize;
    if foreign_plate_ids.is_empty() {
        for obj in &objects {
            register_obj(&mut project, &mesh_ids, obj);
            object_count += 1;
        }
    } else {
        for fid in &foreign_plate_ids {
            project
                .set_active_plate(plate_map[fid])
                .map_err(|e| format!("set active plate: {e:?}"))?;
            for &idx in &plate_assignments[fid] {
                register_obj(&mut project, &mesh_ids, &objects[idx]);
                object_count += 1;
            }
        }
        let first = project.plates[0].id;
        let _ = project.set_active_plate(first);
    }

    // Settings → project-tier overrides. Empty baseline for now (keep all
    // Process/Filament; machine already excluded by `partition`).
    let outcome = compute_overrides(&settings, &part, &BTreeMap::new());
    for (k, v) in &outcome.overrides {
        project.user_overrides.insert(k.clone(), v.clone());
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
        settings_machine_dropped: part.machine.len(),
        settings_unmapped: part.unmapped.len(),
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

    #[test]
    fn imports_a_real_bambu_studio_project_end_to_end() {
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
        assert_eq!(report.settings_applied, project.user_overrides.len());
        assert!(!project.user_overrides.contains_key("machine_start_gcode"));
        assert!(!project.user_overrides.contains_key("nozzle_diameter"));
    }
}
