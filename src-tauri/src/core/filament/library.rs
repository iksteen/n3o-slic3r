//! User overrides for bundled filament fragments.
//!
//! The bundled filament fragments under `profiles/<v>/filament/` are
//! read-only resources. A user can still *edit* one in place: their
//! changes are stored as an override layer keyed by the bundled fragment's
//! slug — the filament keeps its bundled identity and name ("Generic PLA"
//! stays "Generic PLA"), it just resolves with the user's tweaks folded on
//! top. There is at most one override profile per bundled fragment.
//!
//! The profile is created transparently on the first edit and removed when
//! its last override is cleared (or explicitly reverted) — so a filament is
//! "edited" exactly while it carries overrides, which the picker surfaces
//! as a Revert affordance.
//!
//! Cascade integration: a `SlotBinding.filament_identity` is always a
//! bundled slug; the composer folds any override profile for that slug on
//! top of the resolved fragment (see `composer::resolve_filament_ref`).
//!
//! Storage: one `<base-slug>.toml` per edited filament under a writable
//! root the runtime registers at startup. An unregistered root (the test
//! default) yields an empty set, so bundled-slug bindings keep working.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::core::profile_library::{self, FilamentFragmentSummary};

/// A user filament: an override layer over a bundled fragment. Two shapes,
/// distinguished by whether `id` equals `base`:
///
/// - **Edit-in-place** (`id == base`): tweaks to a bundled filament that
///   keeps its bundled identity/name ("Generic PLA" stays "Generic PLA").
/// - **Custom clone** (`id != base`): a new filament with its own identity,
///   composed from `base`'s fragment but relabeled (new brand/type) and
///   carrying the clone's edits. Created by `clone_custom`.
///
/// `overrides` are filament-bucket scalars folded on top of the resolved
/// base fragment at compose time. Persisted as `<id>.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserFilament {
    /// This filament's own slug — its wire identity (the form stored in
    /// `SlotBinding.filament_identity` and surfaced in the catalog).
    /// Defaults to `base` on load for legacy files that predate clones.
    #[serde(default)]
    pub id: String,
    /// Bundled fragment slug the cascade composes from. Equals `id` for an
    /// edit-in-place override.
    pub base: String,
    #[serde(default)]
    pub overrides: BTreeMap<String, String>,
}

impl UserFilament {
    /// True for a custom clone (its own identity, distinct from its base).
    fn is_custom(&self) -> bool {
        self.id != self.base
    }

    /// Legacy files carry only `base`; treat a missing `id` as in-place.
    fn normalized(mut self) -> Self {
        if self.id.is_empty() {
            self.id = self.base.clone();
        }
        self
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    #[error("no bundled filament fragment `{0}` to edit")]
    UnknownBase(String),
}

static REGISTRY: OnceLock<Mutex<Vec<UserFilament>>> = OnceLock::new();
static ROOT: OnceLock<PathBuf> = OnceLock::new();

/// Register the writable library directory (Tauri `setup()`).
pub fn init_root(root: PathBuf) {
    let _ = ROOT.set(root);
}

fn root() -> Option<&'static Path> {
    ROOT.get().map(|p| p.as_path())
}

fn registry() -> &'static Mutex<Vec<UserFilament>> {
    REGISTRY.get_or_init(|| {
        let initial = match root() {
            Some(root) => load_from_disk(root).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "user filament library load failed; starting empty");
                Vec::new()
            }),
            None => Vec::new(),
        };
        Mutex::new(initial)
    })
}

/// Every edited filament's override profile.
pub fn list() -> Vec<UserFilament> {
    registry().lock().expect("filament registry poisoned").clone()
}

/// The user filament with this identity, if one exists.
pub fn lookup(id: &str) -> Option<UserFilament> {
    registry()
        .lock()
        .expect("filament registry poisoned")
        .iter()
        .find(|f| f.id == id)
        .cloned()
}

fn is_bundled(base: &str) -> bool {
    profile_library::filament_fragment_summary(base).is_some()
}

/// Clone `source` (a bundled fragment or another user filament) into a new
/// custom filament with its own identity, relabeled by `vendor` and/or
/// `filament_type` (each `None`/blank keeps the source's value). Carries the
/// source's overrides forward, so cloning an edited filament keeps its edits.
/// Returns the new filament's catalog summary. Errors if `source` is unknown.
pub fn clone_custom(
    source: &str,
    vendor: Option<String>,
    filament_type: Option<String>,
) -> Result<FilamentFragmentSummary, LibraryError> {
    // Resolve the source to its base fragment + the overrides to inherit.
    let (base, mut overrides) = match lookup(source) {
        Some(uf) => (uf.base, uf.overrides),
        None if is_bundled(source) => (source.to_owned(), BTreeMap::new()),
        None => return Err(LibraryError::UnknownBase(source.to_owned())),
    };
    // Validate the base exists before registering anything.
    if profile_library::filament_fragment_summary(&base).is_none() {
        return Err(LibraryError::UnknownBase(base));
    }
    // Only the relabeled fields are stored; the display name is derived from
    // them in `custom_filament_summary`, so it tracks later edits. Drop any
    // inherited frozen name so it can't shadow the derivation.
    overrides.remove("filament_settings_id");
    let clean = |s: Option<String>| s.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
    if let Some(v) = clean(vendor) {
        overrides.insert("filament_vendor".to_owned(), v);
    }
    if let Some(t) = clean(filament_type) {
        overrides.insert("filament_type".to_owned(), t);
    }
    let uf = UserFilament {
        id: format!("custom-{}", Uuid::new_v4()),
        base,
        overrides,
    };
    registry()
        .lock()
        .expect("filament registry poisoned")
        .push(uf.clone());
    persist(&uf);
    profile_library::custom_filament_summary(&uf)
        .ok_or_else(|| LibraryError::UnknownBase(uf.base.clone()))
}

/// Set (or clear, with `None`) one override on the filament identified by
/// `id`. For an edit-in-place override it creates the profile on the first
/// edit and **removes** it once its last override is cleared (back to
/// pristine bundled); a custom filament is never auto-removed (its overrides
/// define it). Returns the resulting profile. Errors if `id` is neither an
/// existing user filament nor a bundled fragment.
pub fn set_override(
    id: &str,
    key: String,
    value: Option<String>,
) -> Result<UserFilament, LibraryError> {
    if lookup(id).is_none() && !is_bundled(id) {
        return Err(LibraryError::UnknownBase(id.to_owned()));
    }
    let (result, removed) = {
        let mut guard = registry().lock().expect("filament registry poisoned");
        let idx = match guard.iter().position(|f| f.id == id) {
            Some(i) => i,
            None => {
                // First in-place edit of a bundled fragment (id == base).
                guard.push(UserFilament {
                    id: id.to_owned(),
                    base: id.to_owned(),
                    overrides: BTreeMap::new(),
                });
                guard.len() - 1
            }
        };
        match value {
            Some(v) => {
                guard[idx].overrides.insert(key, v);
            }
            None => {
                guard[idx].overrides.remove(&key);
            }
        }
        // Only edit-in-place overrides evaporate when emptied; a custom
        // clone stays even with no scalar overrides (it's still a distinct
        // relabeled filament).
        if guard[idx].overrides.is_empty() && !guard[idx].is_custom() {
            (guard.remove(idx), true)
        } else {
            (guard[idx].clone(), false)
        }
    };
    if removed {
        delete_file(id);
    } else {
        persist(&result);
    }
    Ok(result)
}

/// Drop the user filament with this identity entirely — reverts an
/// edit-in-place to pristine bundled, or deletes a custom clone. No-op if
/// there was none.
pub fn remove(id: &str) {
    {
        let mut guard = registry().lock().expect("filament registry poisoned");
        guard.retain(|f| f.id != id);
    }
    delete_file(id);
}

fn persist(filament: &UserFilament) {
    let Some(root) = root() else { return };
    if let Err(e) = (|| -> std::io::Result<()> {
        std::fs::create_dir_all(root)?;
        let body =
            toml::to_string_pretty(filament).map_err(|e| std::io::Error::other(e.to_string()))?;
        std::fs::write(root.join(format!("{}.toml", filament.id)), body)
    })() {
        tracing::warn!(base = %filament.base, error = %e, "filament persist failed; memory unchanged");
    }
}

fn delete_file(id: &str) {
    let Some(root) = root() else { return };
    let path = root.join(format!("{id}.toml"));
    if path.exists() {
        if let Err(e) = std::fs::remove_file(&path) {
            tracing::warn!(error = %e, path = %path.display(), "filament file delete failed");
        }
    }
}

/// Load every `<base>.toml`; malformed files are logged + skipped.
fn load_from_disk(root: &Path) -> std::io::Result<Vec<UserFilament>> {
    if !root.is_dir() {
        std::fs::create_dir_all(root)?;
        return Ok(Vec::new());
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(root)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("toml"))
        .collect();
    files.sort();
    let mut out = Vec::new();
    for path in files {
        let raw = std::fs::read_to_string(&path)?;
        match toml::from_str::<UserFilament>(&raw) {
            // Ignore empty profiles (e.g. a stale file) — they carry no edits.
            Ok(f) if !f.overrides.is_empty() => out.push(f.normalized()),
            Ok(_) => {}
            Err(e) => tracing::warn!(path = %path.display(), error = %e, "skipping malformed filament"),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn with_temp_root() {
        if root().is_none() {
            let dir = std::env::temp_dir().join(format!("n3o-fil-{}", Uuid::new_v4()));
            init_root(dir);
        }
    }

    // The registry is process-global and these tests run in parallel, so
    // each uses a DISTINCT bundled base to avoid racing on the same key
    // (the composer's override test owns `generic-pla`).

    #[test]
    fn edit_in_place_then_revert_to_pristine() {
        with_temp_root();
        let base = "snapmaker-pla";
        // No profile until the first override.
        remove(base);
        assert!(lookup(base).is_none());
        // First override creates the profile (id == base for in-place).
        let f = set_override(base, "nozzle_temperature".into(), Some("215".into()))
            .expect("snapmaker-pla is bundled");
        assert_eq!(f.id, base);
        assert_eq!(f.base, base);
        assert_eq!(f.overrides.get("nozzle_temperature").map(String::as_str), Some("215"));
        assert!(lookup(base).is_some(), "now edited");
        // Clearing the last override removes the profile (back to pristine).
        let cleared = set_override(base, "nozzle_temperature".into(), None).unwrap();
        assert!(cleared.overrides.is_empty());
        assert!(lookup(base).is_none(), "no overrides → pristine again");
    }

    #[test]
    fn remove_discards_all_overrides() {
        with_temp_root();
        let base = "generic-pla-silk";
        set_override(base, "nozzle_temperature".into(), Some("215".into())).unwrap();
        set_override(base, "filament_flow_ratio".into(), Some("0.97".into())).unwrap();
        assert!(lookup(base).is_some());
        remove(base);
        assert!(lookup(base).is_none());
    }

    #[test]
    fn editing_unknown_base_errors() {
        with_temp_root();
        assert!(matches!(
            set_override("not-a-real-fragment", "nozzle_temperature".into(), Some("215".into())),
            Err(LibraryError::UnknownBase(_)),
        ));
    }

    #[test]
    fn clone_makes_a_distinct_relabeled_filament() {
        with_temp_root();
        let s = clone_custom("snapmaker-petg", Some("Acme".into()), None)
            .expect("snapmaker-petg is bundled");
        assert!(s.custom, "summary marked custom");
        assert!(s.identity.starts_with("custom-"), "own identity");
        assert_eq!(s.vendor, "Acme");
        // The name is derived from brand + type (type kept from the source).
        assert_eq!(s.display_name, format!("Acme {}", s.base_type));
        // The clone is a real, lookup-able entry whose base is the source.
        let uf = lookup(&s.identity).expect("clone registered");
        assert_eq!(uf.base, "snapmaker-petg");
        assert_eq!(uf.overrides.get("filament_vendor").map(String::as_str), Some("Acme"));
        // No separately-stored name to drift.
        assert!(!uf.overrides.contains_key("filament_settings_id"));

        // Editing the brand re-derives the name — it's not frozen at clone.
        set_override(&s.identity, "filament_vendor".into(), Some("Zylo".into())).unwrap();
        let renamed = crate::core::profile_library::custom_filament_summary(
            &lookup(&s.identity).unwrap(),
        )
        .unwrap();
        assert_eq!(renamed.display_name, format!("Zylo {}", renamed.base_type));

        // Clearing its overrides does NOT evaporate it (unlike in-place).
        set_override(&s.identity, "filament_vendor".into(), None).unwrap();
        assert!(lookup(&s.identity).is_some(), "custom survives empty overrides");
        remove(&s.identity);
        assert!(lookup(&s.identity).is_none(), "delete removes it");
    }
}
