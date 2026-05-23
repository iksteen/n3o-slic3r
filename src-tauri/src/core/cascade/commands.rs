//! Tauri command surface for the cascade pipeline (PR-1-9).
//!
//! Exposes load / resolve / trace / dimensions commands the frontend
//! drives. Stateful — a `CascadeRegistry` lives in Tauri's
//! `State<Mutex<...>>` and holds parsed `Cascade`s keyed by an opaque
//! `CascadeHandle` (u64) the frontend gets back from `cascade_load`.
//!
//! Contexts are passed as serialized JSON on every command rather
//! than stored in the registry — they're cheap to rebuild, and the
//! frontend already owns the source-of-truth project state. Reduces
//! the registry's surface to just the cascade IR.

use super::types::{Cascade, SourceLocation};
use super::{
    loader::{parse_cascade_str, CascadeLoadError},
    overrides::{parse_override_str, resolve_with_overrides, FlatOverrides, OverrideTiers},
    trace::{trace, Trace},
    validate::{default_known_dimensions, validate_cascade},
    ResolvedOverrides,
};
use crate::core::filament::FilamentProfile;
use crate::core::printer::PrinterProfile;
use crate::core::project::SlicingContext;
use crate::core::scene::BuildPlate;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tauri::State;

/// Opaque handle the frontend uses to refer to a loaded cascade.
/// Monotonic — invalidated when the registry is rebuilt; the
/// frontend reloads on app restart.
pub type CascadeHandle = u64;

/// Tauri-managed state holding parsed cascades.
///
/// Wrap in `Arc<Mutex<...>>` and `tauri::Builder::manage(...)` it
/// from `lib.rs::run()`.
#[derive(Default)]
pub struct CascadeRegistry {
    cascades: HashMap<CascadeHandle, Cascade>,
    next_handle: CascadeHandle,
}

impl CascadeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, cascade: Cascade) -> CascadeHandle {
        let id = self.next_handle;
        self.next_handle = self.next_handle.wrapping_add(1);
        self.cascades.insert(id, cascade);
        id
    }

    pub fn get(&self, handle: CascadeHandle) -> Option<&Cascade> {
        self.cascades.get(&handle)
    }
}

/// Serialized context the frontend constructs each command call.
///
/// The frontend owns the project state and rebuilds this from its
/// in-memory model on each invocation. Cheaper than keeping
/// `SlicingContext` in the registry — the registry would have to
/// invalidate on every printer / filament / plate switch.
#[derive(Debug, Clone, Deserialize)]
pub struct ContextJson {
    pub printer: PrinterProfile,
    pub plate: BuildPlate,
    pub filaments: Vec<FilamentProfile>,
    #[serde(default)]
    pub active_slot: usize,
    #[serde(default)]
    pub user_overrides: Vec<OverrideFileSpec>,
    #[serde(default)]
    pub project_overrides: Vec<OverrideFileSpec>,
    /// Per-object cascade overrides (PR-5-7). When the panel is in
    /// the Object tab, this carries the active object's authored
    /// overrides; otherwise empty / absent. Highest-priority tier:
    /// beats both user and project overrides.
    #[serde(default)]
    pub object_overrides: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OverrideFileSpec {
    pub label: String,
    pub content: String,
}

impl ContextJson {
    fn into_context(self) -> SlicingContext {
        SlicingContext {
            printer: Arc::new(self.printer),
            plate: Arc::new(self.plate),
            filaments: self.filaments.into_iter().map(Arc::new).collect(),
            active_slot: self.active_slot,
        }
    }

    fn into_overrides(
        user: &[OverrideFileSpec],
        project: &[OverrideFileSpec],
        object: &HashMap<String, String>,
    ) -> Result<OverrideTiers, CascadeLoadError> {
        let parse = |specs: &[OverrideFileSpec]| -> Result<Vec<FlatOverrides>, CascadeLoadError> {
            specs
                .iter()
                .map(|s| parse_override_str(&s.content, Path::new(&s.label)))
                .collect()
        };
        let object_tier = if object.is_empty() {
            None
        } else {
            // Object overrides are passed as a flat string map; wrap
            // them in a synthetic `<object>` source so traces have a
            // meaningful label distinct from real files on disk.
            Some(FlatOverrides {
                source: SourceLocation {
                    path: Path::new("<object>").into(),
                    line: 0,
                },
                entries: object.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            })
        };
        Ok(OverrideTiers {
            user: parse(user)?,
            project: parse(project)?,
            object: object_tier,
        })
    }
}

/// Load one or more cascade TOML strings into the registry. Each
/// `(label, content)` pair becomes one of the cascade's source
/// files. The label feeds source-location metadata so error messages
/// + traces render as `label:line`.
#[tauri::command]
#[tracing::instrument(skip(state, files))]
pub fn cascade_load(
    files: Vec<OverrideFileSpec>,
    state: State<Mutex<CascadeRegistry>>,
) -> Result<CascadeHandle, String> {
    let mut all_rules = Vec::new();
    for f in &files {
        let rules = parse_cascade_str(&f.content, Path::new(&f.label))
            .map_err(|e| e.to_string())?;
        all_rules.extend(rules);
    }
    let cascade = Cascade { rules: all_rules };

    // Best-effort load-time validation. Use the default known
    // dimensions for now; PR-1-7's typed Context will plug in the
    // project-active dimension list later.
    if let Err(errs) = validate_cascade(&cascade, &default_known_dimensions()) {
        let msg = errs
            .iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        return Err(msg);
    }

    let mut registry = state.lock().map_err(|e| format!("registry lock: {e}"))?;
    Ok(registry.insert(cascade))
}

/// Resolve a previously-loaded cascade against the supplied context.
/// Returns the full `ResolvedOverrides` map serialized into JSON.
#[tauri::command]
#[tracing::instrument(skip(state, context))]
pub fn cascade_resolve(
    handle: CascadeHandle,
    context: ContextJson,
    state: State<Mutex<CascadeRegistry>>,
) -> Result<ResolvedJson, String> {
    let overrides = ContextJson::into_overrides(
        &context.user_overrides,
        &context.project_overrides,
        &context.object_overrides,
    )
    .map_err(|e| e.to_string())?;
    let ctx = context.into_context();
    let registry = state.lock().map_err(|e| format!("registry lock: {e}"))?;
    let cascade = registry
        .get(handle)
        .ok_or_else(|| format!("unknown cascade handle: {handle}"))?;
    let resolved = resolve_with_overrides(cascade, &overrides, &ctx);
    Ok(ResolvedJson::from_resolved(&resolved))
}

/// Trace a single key of a resolved cascade. Returns `None` when the
/// key isn't in the resolved map.
#[tauri::command]
#[tracing::instrument(skip(state, context))]
pub fn cascade_trace(
    handle: CascadeHandle,
    context: ContextJson,
    key: String,
    state: State<Mutex<CascadeRegistry>>,
) -> Result<Option<Trace>, String> {
    let overrides = ContextJson::into_overrides(
        &context.user_overrides,
        &context.project_overrides,
        &context.object_overrides,
    )
    .map_err(|e| e.to_string())?;
    let ctx = context.into_context();
    let registry = state.lock().map_err(|e| format!("registry lock: {e}"))?;
    let cascade = registry
        .get(handle)
        .ok_or_else(|| format!("unknown cascade handle: {handle}"))?;
    let resolved = resolve_with_overrides(cascade, &overrides, &ctx);
    Ok(trace(&resolved, &key))
}

/// List the dotted predicate dimensions the cascade can target.
/// Today static (from `default_known_dimensions`); PR-1-7's
/// project model will derive this from the active context at the
/// per-project level.
#[tauri::command]
#[tracing::instrument]
pub fn cascade_context_dimensions() -> Vec<String> {
    default_known_dimensions().dimensions
}

/// Serialized cascade resolution. Mirrors `ResolvedOverrides` but
/// flattened for JSON friendliness.
#[derive(Debug, Clone, Serialize)]
pub struct ResolvedJson {
    pub entries: HashMap<String, ResolvedEntryJson>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolvedEntryJson {
    pub value: String,
    pub winning_specificity: usize,
    pub cascade_fallback: Option<String>,
}

impl ResolvedJson {
    pub fn from_resolved(r: &ResolvedOverrides) -> Self {
        let entries = r
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    ResolvedEntryJson {
                        value: v.value.clone(),
                        winning_specificity: v.winning_specificity,
                        cascade_fallback: v.cascade_fallback.clone(),
                    },
                )
            })
            .collect();
        Self { entries }
    }
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

    fn context_json() -> ContextJson {
        use crate::core::filament::FilamentProfile;
        use crate::core::printer::profile::{BoundingBox, Toolhead};
        use crate::core::scene::build_plate::SurfaceKind;
        ContextJson {
            printer: PrinterProfile {
                model: "Bambu A1 mini".into(),
                slot_count: 4,
                supported_build_plates: vec!["Textured PEI".into()],
                toolheads: vec![Toolhead {
                    nozzle_diameter: 0.4,
                    hotend_type: "stainless_steel".into(),
                    max_temp: 300.0,
                    slot_indices: vec![0],
                }],
                build_volume: BoundingBox::default(),
                exclusion_zones: vec![],
            },
            plate: BuildPlate {
                identity: "Textured PEI".into(),
                libslic3r_curr_bed_type: "Textured PEI Plate".into(),
                surface_kind: SurfaceKind::PEI,
            },
            filaments: vec![FilamentProfile {
                identity: "Generic PLA".into(),
                base_type: "PLA".into(),
                vendor: None,
                color: None,
            }],
            active_slot: 0,
            user_overrides: vec![],
            project_overrides: vec![],
            object_overrides: HashMap::new(),
        }
    }

    /// Direct fn-call test (no Tauri runtime). We can exercise the
    /// command bodies via their public signatures using a Mutex
    /// wrapper that implements the State pattern manually.
    #[test]
    fn load_then_resolve_then_trace_via_registry() {
        ensure_ffi();
        let registry = Arc::new(Mutex::new(CascadeRegistry::new()));

        let files = vec![OverrideFileSpec {
            label: "test.toml".into(),
            content: "bed_temp = 50\n\n[filament.type.PLA]\nbed_temp = 55\n".into(),
        }];

        let handle = {
            let mut reg = registry.lock().unwrap();
            // Mirror cascade_load body without the State indirection
            let mut all_rules = Vec::new();
            for f in &files {
                let rules = parse_cascade_str(&f.content, Path::new(&f.label)).unwrap();
                all_rules.extend(rules);
            }
            reg.insert(Cascade { rules: all_rules })
        };

        let ctx = context_json().into_context();
        let overrides = OverrideTiers::empty();
        let reg = registry.lock().unwrap();
        let cascade = reg.get(handle).unwrap();
        let resolved = resolve_with_overrides(cascade, &overrides, &ctx);

        let json = ResolvedJson::from_resolved(&resolved);
        let bed_temp = json.entries.get("bed_temp").unwrap();
        assert_eq!(bed_temp.value, "55", "filament rule wins");
        assert_eq!(bed_temp.winning_specificity, 1);

        let t = trace(&resolved, "bed_temp").unwrap();
        assert_eq!(t.effective_value, "55");
    }

    #[test]
    fn context_dimensions_includes_canonical_set() {
        let dims = cascade_context_dimensions();
        for expected in ["printer.model", "filament.type", "plate.type"] {
            assert!(dims.iter().any(|d| d == expected), "missing {expected}");
        }
    }

    #[test]
    fn unknown_handle_errors_cleanly() {
        ensure_ffi();
        let registry = Mutex::new(CascadeRegistry::new());
        let reg = registry.lock().unwrap();
        assert!(reg.get(9999).is_none());
    }
}
