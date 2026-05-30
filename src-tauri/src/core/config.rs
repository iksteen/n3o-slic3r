//! Application config persisted to
//! `$XDG_CONFIG_HOME/n3o-slic3r/config.toml` (or
//! `~/.config/n3o-slic3r/config.toml`).
//!
//! User-edited preferences that outlive a session. Today it holds the
//! **global** plugin enable/disable map — the lowest activation tier,
//! under any per-project / per-plate override. Add new `[section]`s by
//! extending [`AppConfig`].
//!
//! Persistence is typed: only fields declared on [`AppConfig`] survive a
//! load→save round-trip (unknown sections are dropped on write), so keep
//! every persisted setting modelled here rather than hand-editing extra
//! keys into the file.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::core::paths::config_dir;

/// The whole `config.toml`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub plugins: PluginsConfig,
}

/// The `[plugins]` section.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PluginsConfig {
    /// Global per-plugin enable/disable, keyed by plugin name. A plugin
    /// absent here uses its manifest default (enabled). This is the
    /// global tier beneath any per-project / per-plate override.
    #[serde(default)]
    pub enabled: BTreeMap<String, bool>,
}

/// Path to `config.toml`.
pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

/// Load the config, falling back to defaults when the file is absent or
/// unparseable — a corrupt config must never brick startup.
pub fn load() -> AppConfig {
    load_from(&config_path())
}

/// Persist the config to the default path.
pub fn save(cfg: &AppConfig) -> io::Result<()> {
    save_to(cfg, &config_path())
}

/// Set one plugin's global enablement and persist. Load-modify-save so
/// any other config the user set survives.
pub fn set_plugin_enabled(name: &str, enabled: bool) -> io::Result<()> {
    let mut cfg = load();
    cfg.plugins.enabled.insert(name.to_string(), enabled);
    save(&cfg)
}

/// Load from an explicit path (the testable core of [`load`]).
pub fn load_from(path: &Path) -> AppConfig {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return AppConfig::default(),
    };
    match toml::from_str(&text) {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "config.toml is unparseable; using defaults",
            );
            AppConfig::default()
        }
    }
}

/// Write to an explicit path, creating the parent dir, atomically
/// (temp sibling + rename) so a partial write can't corrupt the file.
pub fn save_to(cfg: &AppConfig, path: &Path) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let body =
        toml::to_string_pretty(cfg).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, body)?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_plugin_enablement() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");

        let mut cfg = AppConfig::default();
        cfg.plugins.enabled.insert("platecycler".into(), false);
        cfg.plugins.enabled.insert("beep-at-layer".into(), true);
        save_to(&cfg, &path).unwrap();

        let loaded = load_from(&path);
        assert_eq!(loaded, cfg);
        assert_eq!(loaded.plugins.enabled.get("platecycler"), Some(&false));
        // Hyphenated plugin names round-trip as bare TOML keys.
        assert_eq!(loaded.plugins.enabled.get("beep-at-layer"), Some(&true));
    }

    #[test]
    fn missing_file_yields_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let loaded = load_from(&tmp.path().join("does-not-exist.toml"));
        assert_eq!(loaded, AppConfig::default());
        assert!(loaded.plugins.enabled.is_empty());
    }

    #[test]
    fn corrupt_file_yields_defaults_not_panic() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        std::fs::write(&path, "this is = not [valid toml").unwrap();
        assert_eq!(load_from(&path), AppConfig::default());
    }

    #[test]
    fn save_creates_missing_parent_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nested").join("dir").join("config.toml");
        save_to(&AppConfig::default(), &path).unwrap();
        assert!(path.is_file());
    }
}
