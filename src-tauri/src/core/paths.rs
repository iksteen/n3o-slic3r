//! Shared filesystem locations for n3o-slic3r's per-user data.

use std::path::{Path, PathBuf};

/// Write `bytes` to `path` atomically: stage to a sibling temp file, then
/// rename over `path` (atomic within a directory on POSIX). The original is
/// untouched unless the rename succeeds; the temp is cleaned up on failure.
/// Callers needing the parent directory should `create_dir_all` first.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut tmp_os = path.as_os_str().to_owned();
    tmp_os.push(".tmp");
    let tmp = PathBuf::from(tmp_os);
    std::fs::write(&tmp, bytes)?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// Per-user data directory for `subdir` — e.g. `data_dir("plugins")` →
/// `$XDG_DATA_HOME/n3o-slic3r/plugins`, or `~/.local/share/...`, or a
/// `temp_dir` fallback. Mirrors the XDG base-dir convention; the
/// fallback keeps non-Linux dev runs working until Tauri's path API
/// replaces this. Used by both the autosave store and the plugin host
/// so the data-root convention lives in one place.
pub fn data_dir(subdir: &str) -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("n3o-slic3r").join(subdir);
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("n3o-slic3r")
            .join(subdir);
    }
    std::env::temp_dir().join("n3o-slic3r").join(subdir)
}

/// Per-user *config* directory: `$XDG_CONFIG_HOME/n3o-slic3r`, or
/// `~/.config/n3o-slic3r`, or a temp fallback. Distinct from
/// [`data_dir`] (state under `$XDG_DATA_HOME`): config holds the
/// user-edited preferences that outlive a session — currently the
/// global plugin enable/disable map (`config.toml`).
pub fn config_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("n3o-slic3r");
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".config").join("n3o-slic3r");
    }
    std::env::temp_dir().join("n3o-slic3r-config")
}
