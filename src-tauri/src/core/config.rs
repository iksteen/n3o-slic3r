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
    #[serde(default)]
    pub defaults: DefaultsConfig,
}

/// The `[defaults]` section — workspace defaults remembered across sessions.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DefaultsConfig {
    /// The printer instance the user most recently bound to a plate. Used as
    /// the default printer for new plates and new projects. `None` until the
    /// user has bound a printer at least once.
    #[serde(default)]
    pub printer_instance: Option<String>,
}

/// The `[plugins]` section.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PluginsConfig {
    /// Global per-plugin enable/disable, keyed by plugin name. A plugin
    /// absent here uses its manifest default (enabled). This is the
    /// global tier beneath any per-project / per-plate override.
    #[serde(default)]
    pub enabled: BTreeMap<String, bool>,
    /// Global per-plugin **setting** values — the global tier of the
    /// settings cascade — keyed by plugin name then setting key
    /// (`[plugins.settings.<name>]`). Values are typed in the file
    /// (`layer = 10`); they're flattened to the cascade's string
    /// vocabulary for resolution.
    #[serde(default)]
    pub settings: BTreeMap<String, BTreeMap<String, toml::Value>>,
}

impl PluginsConfig {
    /// Global plugin settings flattened to the cascade string
    /// vocabulary, keyed by plugin name then setting key — the shape the
    /// host's global tier + the resolver consume.
    pub fn settings_as_strings(&self) -> BTreeMap<String, BTreeMap<String, String>> {
        self.settings
            .iter()
            .map(|(name, kv)| {
                let flat = kv
                    .iter()
                    .map(|(k, v)| (k.clone(), toml_scalar_to_string(v)))
                    .collect();
                (name.clone(), flat)
            })
            .collect()
    }
}

/// Render a scalar TOML value to the cascade's flat-string vocabulary (a
/// whole float prints without a fractional part, matching how overrides
/// are flattened). Non-scalars fall back to TOML's own `to_string`.
fn toml_scalar_to_string(v: &toml::Value) -> String {
    match v {
        toml::Value::String(s) => s.clone(),
        toml::Value::Integer(i) => i.to_string(),
        toml::Value::Float(f) => {
            if f.fract() == 0.0 && f.is_finite() {
                format!("{}", *f as i64)
            } else {
                format!("{f}")
            }
        }
        toml::Value::Boolean(b) => b.to_string(),
        other => other.to_string(),
    }
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

/// Remember `id` as the user's last-selected printer instance — the default
/// for new plates and new projects. Load-modify-save so other config survives.
pub fn set_default_printer_instance(id: &str) -> io::Result<()> {
    let mut cfg = load();
    cfg.defaults.printer_instance = Some(id.to_string());
    save(&cfg)
}

/// Set one plugin's global setting value and persist (load-modify-save).
pub fn set_plugin_setting(name: &str, key: &str, value: toml::Value) -> io::Result<()> {
    let mut cfg = load();
    cfg.plugins
        .settings
        .entry(name.to_string())
        .or_default()
        .insert(key.to_string(), value);
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
    crate::core::paths::atomic_write(path, body.as_bytes())
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
    fn round_trips_and_flattens_plugin_settings() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");

        let mut cfg = AppConfig::default();
        let kv = cfg
            .plugins
            .settings
            .entry("beep-at-layer".into())
            .or_default();
        kv.insert("layer".into(), toml::Value::Integer(10));
        kv.insert("tag".into(), toml::Value::String("hi".into()));
        save_to(&cfg, &path).unwrap();

        let loaded = load_from(&path);
        assert_eq!(loaded, cfg, "typed values round-trip");
        // Flattened to the cascade string vocabulary the resolver wants.
        let flat = loaded.plugins.settings_as_strings();
        assert_eq!(flat["beep-at-layer"]["layer"], "10");
        assert_eq!(flat["beep-at-layer"]["tag"], "hi");
    }

    #[test]
    fn round_trips_default_printer_instance() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");

        let mut cfg = AppConfig::default();
        assert_eq!(cfg.defaults.printer_instance, None, "none by default");
        cfg.defaults.printer_instance = Some("snappy".into());
        save_to(&cfg, &path).unwrap();

        let loaded = load_from(&path);
        assert_eq!(loaded.defaults.printer_instance.as_deref(), Some("snappy"));
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
