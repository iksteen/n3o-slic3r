//! Shared filesystem locations for n3o-slic3r's per-user data.

use std::path::PathBuf;

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
