//! User-owned printer instance library: persists `PrinterInstance`s
//! as TOML files in a writable directory the runtime points us at.
//!
//! On first launch the directory is empty; we copy
//! [`bundled_instances`](super::instance_library::bundled_instances)
//! into it and return that as the seed. Subsequent launches load
//! whatever's on disk — the user owns those files now, may have
//! renamed/edited/deleted any of them. Mutations through
//! [`instance_registry::mutate_instance`](super::instance_registry::mutate_instance)
//! call [`persist`] to write the change back.
//!
//! Layout: `<root>/<instance_id>.toml`. One instance per file so the
//! user can drop in a single TOML to add a printer manually.
//!
//! Threading: the `ROOT` `OnceLock` is set once by Tauri's `setup()`;
//! tests use [`with_root`] / `init_root_for_test` to scope a temp
//! directory for the duration of one test.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use super::instance::PrinterInstance;
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
/// First-launch behavior: when the directory is missing or empty,
/// seeds with [`bundled_instances`] (writing each to disk) and
/// returns the seeded set. Malformed files are logged and skipped.
pub fn load_or_seed(root: &Path) -> Result<Vec<PrinterInstance>, StorageError> {
    if root.is_dir() {
        let mut files: Vec<PathBuf> = std::fs::read_dir(root)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("toml"))
            .collect();
        files.sort();
        if !files.is_empty() {
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
            return Ok(out);
        }
    }
    let seeded = bundled_instances();
    std::fs::create_dir_all(root)?;
    for inst in &seeded {
        persist(root, inst)?;
    }
    Ok(seeded)
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
    fn first_launch_seeds_bundled_into_empty_dir() {
        let dir = tempdir().expect("tempdir");
        let loaded = load_or_seed(dir.path()).expect("seed");
        // Bundled set is bambi + snappy.
        assert_eq!(loaded.len(), 2);
        // Each one landed on disk as <id>.toml.
        assert!(dir.path().join("bambi.toml").exists());
        assert!(dir.path().join("snappy.toml").exists());
    }

    #[test]
    fn reload_returns_what_was_written() {
        let dir = tempdir().expect("tempdir");
        let first = load_or_seed(dir.path()).expect("seed");
        // Mutate the on-disk copy by hand to prove the second load
        // reads what we wrote, not the bundled fixtures.
        let path = dir.path().join("bambi.toml");
        let raw = std::fs::read_to_string(&path).expect("read");
        let edited = raw.replace("display_name = \"Bambi\"", "display_name = \"Edited Bambi\"");
        assert_ne!(raw, edited, "search-replace must hit the field");
        std::fs::write(&path, edited).expect("write");

        let second = load_or_seed(dir.path()).expect("reload");
        assert_eq!(second.len(), first.len());
        let bambi = second.iter().find(|i| i.id == "bambi").expect("bambi present");
        assert_eq!(bambi.display_name, "Edited Bambi");
    }

    #[test]
    fn persist_round_trips_single_instance() {
        let dir = tempdir().expect("tempdir");
        let seeded = load_or_seed(dir.path()).expect("seed");
        let mut bambi = seeded.into_iter().find(|i| i.id == "bambi").unwrap();
        // Change something a mutation path might touch.
        bambi.extruders[0].slots[0].color = Some("#ff00ff".into());
        persist(dir.path(), &bambi).expect("persist");
        let reloaded = load_or_seed(dir.path()).expect("reload");
        let bambi2 = reloaded.iter().find(|i| i.id == "bambi").unwrap();
        assert_eq!(
            bambi2.extruders[0].slots[0].color.as_deref(),
            Some("#ff00ff"),
        );
    }

    #[test]
    fn malformed_file_is_skipped_not_fatal() {
        let dir = tempdir().expect("tempdir");
        load_or_seed(dir.path()).expect("seed");
        // Drop in garbage.
        std::fs::write(dir.path().join("broken.toml"), "not = toml = invalid").unwrap();
        let loaded = load_or_seed(dir.path()).expect("reload tolerates the bad file");
        // The two seeded instances still load; broken.toml is skipped.
        assert!(loaded.iter().any(|i| i.id == "bambi"));
        assert!(loaded.iter().any(|i| i.id == "snappy"));
    }
}
