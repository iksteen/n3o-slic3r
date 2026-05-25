//! User-owned printer instance library: persists `PrinterInstance`s
//! as TOML files in a writable directory the runtime points us at.
//!
//! Production starts empty — first launch has no printers, the
//! frontend renders the onboarding empty-state, and the add-printer
//! wizard writes the first instance via
//! [`instance_registry::create_instance`](super::instance_registry::create_instance).
//! Subsequent launches load whatever's on disk; the user owns
//! those files now, may have renamed/edited/deleted any of them.
//! Mutations through
//! [`instance_registry::mutate_instance`](super::instance_registry::mutate_instance)
//! call [`persist`] to write the change back.
//!
//! Test fallback: when no storage root is registered (the path tests
//! take when they don't call `init_root`), the registry falls back to
//! [`bundled_instances`](super::instance_library::bundled_instances)
//! as an in-memory fixture — bambi + snappy — so the wide existing
//! test surface doesn't need plumbing for a temp library per test.
//!
//! Layout: `<root>/<instance_id>.toml`. One instance per file so the
//! user can drop in a single TOML to add a printer manually.
//!
//! Threading: the `ROOT` `OnceLock` is set once by Tauri's `setup()`.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use super::instance::PrinterInstance;
#[cfg(test)]
use super::instance_library::bundled_instances;

static ROOT: OnceLock<PathBuf> = OnceLock::new();

/// Set the user-library directory. The Tauri runtime calls this once
/// at startup; subsequent calls are no-ops (the directory is fixed
/// for the process lifetime). Tests use [`init_root_for_test`].
pub fn init_root(root: PathBuf) {
    let _ = ROOT.set(root);
}

/// Current user-library directory, if one was registered. The
/// instance registry calls this on first-access to decide whether to
/// load from disk or fall back to the bundled set.
pub fn root() -> Option<&'static Path> {
    ROOT.get().map(|p| p.as_path())
}

/// Errors propagating up from a filesystem operation. Wrapped in
/// `tracing::warn!` at the call sites — none of these are fatal:
/// a persist failure leaves the in-memory state intact, a load
/// failure falls back to the bundled set.
#[derive(Debug)]
pub enum StorageError {
    Io(std::io::Error),
    Serialize(toml::ser::Error),
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io: {e}"),
            Self::Serialize(e) => write!(f, "serialize: {e}"),
        }
    }
}

impl std::error::Error for StorageError {}

impl From<std::io::Error> for StorageError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<toml::ser::Error> for StorageError {
    fn from(e: toml::ser::Error) -> Self {
        Self::Serialize(e)
    }
}

/// Load every `<id>.toml` from `root` in sorted filename order.
/// Missing directory is created and treated the same as empty —
/// production first-launch returns `Ok(vec![])` so the empty-state
/// UI fires. Malformed files are logged and skipped.
pub fn load_from_disk(root: &Path) -> Result<Vec<PrinterInstance>, StorageError> {
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
        match toml::from_str::<PrinterInstance>(&raw) {
            Ok(inst) => out.push(inst),
            Err(e) => tracing::warn!(
                path = %path.display(),
                error = %e,
                "skipping malformed printer instance file",
            ),
        }
    }
    Ok(out)
}

/// Write one instance to `<root>/<id>.toml`. Creates the directory
/// if it doesn't exist.
pub fn persist(root: &Path, instance: &PrinterInstance) -> Result<(), StorageError> {
    std::fs::create_dir_all(root)?;
    let body = toml::to_string_pretty(instance)?;
    let path = root.join(format!("{}.toml", instance.id));
    std::fs::write(path, body)?;
    Ok(())
}

/// Remove `<root>/<id>.toml`. Silent no-op if the file doesn't
/// exist (already gone, or never persisted because root wasn't set).
#[allow(dead_code)]
pub fn delete(root: &Path, id: &str) -> Result<(), StorageError> {
    let path = root.join(format!("{}.toml", id));
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn first_launch_returns_empty() {
        let dir = tempdir().expect("tempdir");
        let loaded = load_from_disk(dir.path()).expect("load empty");
        assert!(loaded.is_empty(), "fresh launch must start with no instances");
    }

    #[test]
    fn missing_directory_is_created_returning_empty() {
        let dir = tempdir().expect("tempdir");
        let inner = dir.path().join("printers-not-yet-created");
        assert!(!inner.exists());
        let loaded = load_from_disk(&inner).expect("load creates dir");
        assert!(loaded.is_empty());
        assert!(inner.is_dir(), "load_from_disk creates the directory");
    }

    #[test]
    fn persist_round_trips_single_instance() {
        let dir = tempdir().expect("tempdir");
        // Hand-write a bundled fixture so we have something to round-trip.
        for inst in bundled_instances() {
            persist(dir.path(), &inst).expect("seed");
        }
        let seeded = load_from_disk(dir.path()).expect("load");
        let mut bambi = seeded.into_iter().find(|i| i.id == "bambi").unwrap();
        bambi.extruders[0].slots[0].color = Some("#ff00ff".into());
        persist(dir.path(), &bambi).expect("persist");
        let reloaded = load_from_disk(dir.path()).expect("reload");
        let bambi2 = reloaded.iter().find(|i| i.id == "bambi").unwrap();
        assert_eq!(
            bambi2.extruders[0].slots[0].color.as_deref(),
            Some("#ff00ff"),
        );
    }

    #[test]
    fn malformed_file_is_skipped_not_fatal() {
        let dir = tempdir().expect("tempdir");
        for inst in bundled_instances() {
            persist(dir.path(), &inst).expect("seed");
        }
        std::fs::write(dir.path().join("broken.toml"), "not = toml = invalid").unwrap();
        let loaded = load_from_disk(dir.path()).expect("reload tolerates the bad file");
        assert!(loaded.iter().any(|i| i.id == "bambi"));
        assert!(loaded.iter().any(|i| i.id == "snappy"));
    }
}
