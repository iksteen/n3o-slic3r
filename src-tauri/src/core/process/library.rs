//! User overrides for bundled process (quality) profiles.
//!
//! The bundled process fragments under `printer/<slug>/processes/` are
//! read-only resources. A user can stamp the current quality settings onto
//! one as an override layer: the diff-from-base is stored, keyed by the
//! printer + the bundled process slug, and folded back on top of the
//! fragment at compose time (`composer::resolve_process_ref`). The profile
//! keeps its bundled identity/name; it just resolves with the user's diff
//! applied.
//!
//! Process slugs are **printer-scoped** — `"0.20mm-standard"` means a
//! different fragment on the A1 mini than on the U1 — so every entry carries
//! its `printer` (the `printer_fragment_slug`) alongside the `base` slug.
//!
//! `base` is the profile-level pointer to the bundled fragment this
//! overrides; `id` is the entry's own identity. They're equal for the
//! stamp-in-place case shipping now; a future "save as a new named profile"
//! gives a clone its own `id` while `base` still names the fragment it
//! composes from (mirroring the filament library's id/base split).
//!
//! Storage: one `<printer>/<id>.toml` per entry under a writable root the
//! runtime registers at startup. An unregistered root (the test default)
//! yields an empty set, so bundled-slug bindings keep working.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A user's override layer over one bundled process fragment, scoped to a
/// printer. `overrides` are process-bucket scalars folded on top of the
/// resolved base fragment at compose time.
///
/// Two shapes, by whether `id == base`:
/// - **Stamp-in-place** (`id == base`, `name` `None`): edits saved onto a
///   bundled profile, which keeps its bundled name.
/// - **Named custom** (`id != base`, `name` `Some`): a "save as…" clone with
///   its own identity + display name, composed from `base`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserProcess {
    /// This profile's own slug — its identity. Equals `base` for the
    /// stamp-in-place case; a distinct slug for a named custom clone.
    #[serde(default)]
    pub id: String,
    /// The `printer_fragment_slug` this profile is scoped to.
    pub printer: String,
    /// Bundled process slug the cascade composes from (and, in-place, is
    /// identified by). The profile-level pointer to the overridden fragment.
    pub base: String,
    /// Display name for a named custom clone; `None` for a stamp-in-place
    /// override (which shows the bundled name).
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub overrides: BTreeMap<String, String>,
}

impl UserProcess {
    /// Legacy/short files may omit `id`; treat a missing one as in-place.
    fn normalized(mut self) -> Self {
        if self.id.is_empty() {
            self.id = self.base.clone();
        }
        self
    }
}

static REGISTRY: OnceLock<Mutex<Vec<UserProcess>>> = OnceLock::new();
static ROOT: OnceLock<PathBuf> = OnceLock::new();

/// Register the writable library directory (Tauri `setup()`).
pub fn init_root(root: PathBuf) {
    let _ = ROOT.set(root);
}

fn root() -> Option<&'static Path> {
    ROOT.get().map(|p| p.as_path())
}

fn registry() -> &'static Mutex<Vec<UserProcess>> {
    REGISTRY.get_or_init(|| {
        let initial = match root() {
            Some(root) => load_from_disk(root).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "user process library load failed; starting empty");
                Vec::new()
            }),
            None => Vec::new(),
        };
        Mutex::new(initial)
    })
}

/// Every stamped process profile.
pub fn list() -> Vec<UserProcess> {
    registry().lock().expect("process registry poisoned").clone()
}

/// The override profile for `(printer, id)`, if one exists.
pub fn lookup(printer: &str, id: &str) -> Option<UserProcess> {
    registry()
        .lock()
        .expect("process registry poisoned")
        .iter()
        .find(|p| p.printer == printer && p.id == id)
        .cloned()
}

/// Merge `additions` into `(printer, base)`'s in-place override profile
/// (creating it on the first stamp), and return the resulting profile. An
/// addition with `None` clears that key. If the profile ends up empty it is
/// removed (back to pristine bundled). `additions` are assumed pre-filtered
/// to stampable process-bucket keys by the caller.
pub fn stamp(
    printer: &str,
    base: &str,
    additions: BTreeMap<String, Option<String>>,
) -> UserProcess {
    let (result, removed) = {
        let mut guard = registry().lock().expect("process registry poisoned");
        let idx = match guard
            .iter()
            .position(|p| p.printer == printer && p.id == base)
        {
            Some(i) => i,
            None => {
                guard.push(UserProcess {
                    id: base.to_owned(),
                    printer: printer.to_owned(),
                    base: base.to_owned(),
                    name: None,
                    overrides: BTreeMap::new(),
                });
                guard.len() - 1
            }
        };
        for (k, v) in additions {
            match v {
                Some(v) => {
                    guard[idx].overrides.insert(k, v);
                }
                None => {
                    guard[idx].overrides.remove(&k);
                }
            }
        }
        if guard[idx].overrides.is_empty() {
            (guard.remove(idx), true)
        } else {
            (guard[idx].clone(), false)
        }
    };
    if removed {
        delete_file(printer, base);
    } else {
        persist(&result);
    }
    result
}

/// Create a named custom profile (`id != base`) on `printer`, composed from
/// the bundled `base` fragment with `overrides` folded on top, under the
/// given display `name`. Returns the new profile (its generated `id` is what
/// a plate binds to). Unlike a stamp, an empty `overrides` is kept — a named
/// clone is a distinct profile even before it diverges.
pub fn create_custom(
    printer: &str,
    base: &str,
    name: String,
    overrides: BTreeMap<String, String>,
) -> UserProcess {
    let up = UserProcess {
        id: format!("custom-process-{}", Uuid::new_v4()),
        printer: printer.to_owned(),
        base: base.to_owned(),
        name: Some(name),
        overrides,
    };
    registry()
        .lock()
        .expect("process registry poisoned")
        .push(up.clone());
    persist(&up);
    up
}

/// Discard `(printer, id)`'s override profile entirely — back to pristine
/// bundled (in-place) or deletes a named custom. No-op if there was none.
pub fn remove(printer: &str, id: &str) {
    {
        let mut guard = registry().lock().expect("process registry poisoned");
        guard.retain(|p| !(p.printer == printer && p.id == id));
    }
    delete_file(printer, id);
}

fn persist(profile: &UserProcess) {
    let Some(root) = root() else { return };
    let dir = root.join(&profile.printer);
    if let Err(e) = (|| -> std::io::Result<()> {
        std::fs::create_dir_all(&dir)?;
        let body =
            toml::to_string_pretty(profile).map_err(|e| std::io::Error::other(e.to_string()))?;
        std::fs::write(dir.join(format!("{}.toml", profile.id)), body)
    })() {
        tracing::warn!(printer = %profile.printer, id = %profile.id, error = %e, "process persist failed; memory unchanged");
    }
}

fn delete_file(printer: &str, id: &str) {
    let Some(root) = root() else { return };
    let path = root.join(printer).join(format!("{id}.toml"));
    if path.exists() {
        if let Err(e) = std::fs::remove_file(&path) {
            tracing::warn!(error = %e, path = %path.display(), "process file delete failed");
        }
    }
}

/// Load every `<printer>/<id>.toml`; malformed files are logged + skipped.
fn load_from_disk(root: &Path) -> std::io::Result<Vec<UserProcess>> {
    if !root.is_dir() {
        std::fs::create_dir_all(root)?;
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    // One subdirectory per printer slug.
    let mut printers: Vec<PathBuf> = std::fs::read_dir(root)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    printers.sort();
    for dir in printers {
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("toml"))
            .collect();
        files.sort();
        for path in files {
            let raw = std::fs::read_to_string(&path)?;
            match toml::from_str::<UserProcess>(&raw) {
                Ok(p) if !p.overrides.is_empty() => out.push(p.normalized()),
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "skipping malformed process")
                }
            }
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
            let dir = std::env::temp_dir().join(format!("n3o-proc-{}", Uuid::new_v4()));
            init_root(dir);
        }
    }

    // The registry is process-global and tests run in parallel, so each uses
    // a DISTINCT (printer, base) pair to avoid racing on the same key.

    #[test]
    fn stamp_then_remove_round_trips() {
        with_temp_root();
        let (printer, base) = ("test-printer-a", "0.20mm-standard");
        remove(printer, base);
        assert!(lookup(printer, base).is_none());

        let mut add = BTreeMap::new();
        add.insert("layer_height".to_owned(), Some("0.28".to_owned()));
        add.insert("outer_wall_speed".to_owned(), Some("60".to_owned()));
        let up = stamp(printer, base, add);
        assert_eq!(up.id, base, "in-place id == base");
        assert_eq!(up.printer, printer);
        assert_eq!(up.overrides.get("layer_height").map(String::as_str), Some("0.28"));
        assert!(lookup(printer, base).is_some());

        // A second stamp merges (here, clears one key, keeps the other).
        let mut add2 = BTreeMap::new();
        add2.insert("outer_wall_speed".to_owned(), None);
        let up2 = stamp(printer, base, add2);
        assert!(!up2.overrides.contains_key("outer_wall_speed"));
        assert_eq!(up2.overrides.get("layer_height").map(String::as_str), Some("0.28"));

        remove(printer, base);
        assert!(lookup(printer, base).is_none());
    }

    #[test]
    fn emptying_all_overrides_removes_the_profile() {
        with_temp_root();
        let (printer, base) = ("test-printer-b", "0.16mm-optimal");
        let mut add = BTreeMap::new();
        add.insert("layer_height".to_owned(), Some("0.16".to_owned()));
        stamp(printer, base, add);
        assert!(lookup(printer, base).is_some());
        // Clearing the only override drops the whole profile.
        let mut clear = BTreeMap::new();
        clear.insert("layer_height".to_owned(), None);
        let up = stamp(printer, base, clear);
        assert!(up.overrides.is_empty());
        assert!(lookup(printer, base).is_none(), "no overrides → pristine");
    }

    #[test]
    fn create_custom_makes_a_named_distinct_profile() {
        with_temp_root();
        let printer = "test-printer-e";
        let mut ov = BTreeMap::new();
        ov.insert("layer_height".to_owned(), "0.12".to_owned());
        let up = create_custom(printer, "0.20mm-standard", "My Fine".to_owned(), ov);
        assert!(up.id.starts_with("custom-process-"), "own identity");
        assert_ne!(up.id, up.base, "custom: id != base");
        assert_eq!(up.base, "0.20mm-standard", "composes from the base fragment");
        assert_eq!(up.name.as_deref(), Some("My Fine"));
        // Lookup is by id; the base slug is untouched (no in-place entry made).
        assert!(lookup(printer, &up.id).is_some());
        assert!(lookup(printer, "0.20mm-standard").is_none());
        remove(printer, &up.id);
        assert!(lookup(printer, &up.id).is_none());
    }

    #[test]
    fn same_slug_distinct_printers_are_separate_entries() {
        with_temp_root();
        let base = "0.20mm-standard";
        let mut a = BTreeMap::new();
        a.insert("layer_height".to_owned(), Some("0.18".to_owned()));
        stamp("test-printer-c", base, a);
        let mut b = BTreeMap::new();
        b.insert("layer_height".to_owned(), Some("0.24".to_owned()));
        stamp("test-printer-d", base, b);
        assert_eq!(
            lookup("test-printer-c", base).unwrap().overrides["layer_height"],
            "0.18"
        );
        assert_eq!(
            lookup("test-printer-d", base).unwrap().overrides["layer_height"],
            "0.24"
        );
        remove("test-printer-c", base);
        remove("test-printer-d", base);
    }
}
