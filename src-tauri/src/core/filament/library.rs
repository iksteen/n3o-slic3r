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

use crate::core::profile_library;

/// A user's override layer over one bundled filament fragment. Keyed by
/// `base` (the bundled slug); `overrides` are filament-bucket scalars
/// folded on top of the resolved base at compose time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserFilament {
    /// Bundled fragment slug this overrides (and is identified by).
    pub base: String,
    #[serde(default)]
    pub overrides: BTreeMap<String, String>,
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

/// The override profile for a bundled slug, if one exists.
pub fn lookup(base: &str) -> Option<UserFilament> {
    registry()
        .lock()
        .expect("filament registry poisoned")
        .iter()
        .find(|f| f.base == base)
        .cloned()
}

fn is_bundled(base: &str) -> bool {
    profile_library::filament_fragment_summary(base).is_some()
}

/// Set (or clear, with `None`) one override on `base`'s profile. Creates
/// the profile on first override; **removes** it once its last override is
/// cleared — so the filament is back to pristine bundled defaults. Returns
/// the resulting profile (with possibly-empty overrides). Errors if `base`
/// isn't a bundled fragment.
pub fn set_override(
    base: &str,
    key: String,
    value: Option<String>,
) -> Result<UserFilament, LibraryError> {
    if lookup(base).is_none() && !is_bundled(base) {
        return Err(LibraryError::UnknownBase(base.to_owned()));
    }
    let (result, now_empty) = {
        let mut guard = registry().lock().expect("filament registry poisoned");
        let idx = match guard.iter().position(|f| f.base == base) {
            Some(i) => i,
            None => {
                guard.push(UserFilament {
                    base: base.to_owned(),
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
        let now_empty = guard[idx].overrides.is_empty();
        if now_empty {
            let removed = guard.remove(idx);
            (removed, true)
        } else {
            (guard[idx].clone(), false)
        }
    };
    if now_empty {
        delete_file(base);
    } else {
        persist(&result);
    }
    Ok(result)
}

/// Discard `base`'s override profile entirely — back to pristine bundled.
/// No-op if there was none.
pub fn revert(base: &str) {
    {
        let mut guard = registry().lock().expect("filament registry poisoned");
        guard.retain(|f| f.base != base);
    }
    delete_file(base);
}

fn persist(filament: &UserFilament) {
    let Some(root) = root() else { return };
    if let Err(e) = (|| -> std::io::Result<()> {
        std::fs::create_dir_all(root)?;
        let body =
            toml::to_string_pretty(filament).map_err(|e| std::io::Error::other(e.to_string()))?;
        std::fs::write(root.join(format!("{}.toml", filament.base)), body)
    })() {
        tracing::warn!(base = %filament.base, error = %e, "filament persist failed; memory unchanged");
    }
}

fn delete_file(base: &str) {
    let Some(root) = root() else { return };
    let path = root.join(format!("{base}.toml"));
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
            Ok(f) if !f.overrides.is_empty() => out.push(f),
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
        revert(base);
        assert!(lookup(base).is_none());
        // First override creates the profile.
        let f = set_override(base, "nozzle_temperature".into(), Some("215".into()))
            .expect("snapmaker-pla is bundled");
        assert_eq!(f.base, base);
        assert_eq!(f.overrides.get("nozzle_temperature").map(String::as_str), Some("215"));
        assert!(lookup(base).is_some(), "now edited");
        // Clearing the last override removes the profile (back to pristine).
        let cleared = set_override(base, "nozzle_temperature".into(), None).unwrap();
        assert!(cleared.overrides.is_empty());
        assert!(lookup(base).is_none(), "no overrides → pristine again");
    }

    #[test]
    fn revert_discards_all_overrides() {
        with_temp_root();
        let base = "generic-pla-silk";
        set_override(base, "nozzle_temperature".into(), Some("215".into())).unwrap();
        set_override(base, "filament_flow_ratio".into(), Some("0.97".into())).unwrap();
        assert!(lookup(base).is_some());
        revert(base);
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
}
