//! Plugin manifest: the `plugin.toml` schema plus validation.
//!
//! A plugin directory holds a `plugin.toml` (this schema) and its Lua
//! entry file. Parsing happens in two steps: a permissive `RawManifest`
//! (serde over `toml`) captures whatever's on disk, then [`validate`]
//! turns it into a checked [`PluginManifest`] or a typed
//! [`ManifestError`] — so a malformed field yields a clear message
//! rather than a raw serde error.

use std::collections::BTreeMap;
use std::path::{Component, Path};

use serde::Deserialize;
use thiserror::Error;

/// A pipeline point a plugin can hook. The compose hook is deferred
/// post-MVP, so it is deliberately not represented here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookKind {
    PreSlice,
    PostSlice,
    PreSend,
}

impl HookKind {
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "pre_slice" => Some(Self::PreSlice),
            "post_slice" => Some(Self::PostSlice),
            "pre_send" => Some(Self::PreSend),
            _ => None,
        }
    }

    /// The manifest/string name for this hook (`"pre_slice"`, …).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PreSlice => "pre_slice",
            Self::PostSlice => "post_slice",
            Self::PreSend => "pre_send",
        }
    }
}

/// Which printers a plugin applies to. `Any` (the default, or an
/// explicit `["any"]`) means every printer; `Models` restricts to the
/// listed printer model strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrinterCompat {
    Any,
    Models(Vec<String>),
}

/// The declared type of a plugin setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingKind {
    String,
    Number,
    Bool,
    Enum,
}

impl SettingKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Number => "number",
            Self::Bool => "bool",
            Self::Enum => "enum",
        }
    }
}

/// A validated default value for a plugin setting. Numbers collapse to
/// `f64`; enums carry their chosen string.
#[derive(Debug, Clone, PartialEq)]
pub enum SettingValue {
    String(String),
    Number(f64),
    Bool(bool),
}

/// One plugin-declared setting. Consumed by the cascade UI later; here
/// it is only parsed + checked for a type-matching default.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingDecl {
    pub kind: SettingKind,
    pub default: SettingValue,
    pub label: Option<String>,
    /// Allowed values for an `Enum` setting; empty otherwise.
    pub values: Vec<String>,
}

/// A validated plugin manifest.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginManifest {
    pub name: String,
    /// Stored verbatim; validated to parse as semver.
    pub version: String,
    /// Lua entry file, relative to the plugin directory.
    pub entry: String,
    pub hooks: Vec<HookKind>,
    pub printer_compatibility: PrinterCompat,
    pub description: Option<String>,
    pub settings: BTreeMap<String, SettingDecl>,
}

/// Why a `plugin.toml` was rejected. `PartialEq` so tests can assert on
/// the variant.
#[derive(Debug, Error, PartialEq)]
pub enum ManifestError {
    #[error("could not read manifest: {0}")]
    Io(String),
    #[error("manifest is not valid TOML: {0}")]
    Toml(String),
    #[error("plugin name is empty")]
    EmptyName,
    #[error("plugin name `{0}` is not kebab-case (a-z, 0-9, hyphens; no leading/trailing hyphen)")]
    BadName(String),
    #[error("version `{0}` is not valid semver")]
    BadVersion(String),
    #[error("entry `{0}`: {1}")]
    BadEntry(String, String),
    #[error("a plugin must declare at least one hook (pre_slice, post_slice, pre_send)")]
    EmptyHooks,
    #[error("unknown hook `{0}` (expected one of: pre_slice, post_slice, pre_send)")]
    UnknownHook(String),
    #[error("setting `{key}`: {reason}")]
    BadSetting { key: String, reason: String },
    #[error("duplicate plugin name `{0}` (declared by more than one plugin directory)")]
    DuplicateName(String),
}

/// Permissive on-disk shape. Required fields are `Option` so a missing
/// one produces our own error in [`validate`], not a serde message.
#[derive(Debug, Deserialize)]
struct RawManifest {
    name: Option<String>,
    version: Option<String>,
    entry: Option<String>,
    hooks: Option<Vec<String>>,
    printer_compatibility: Option<Vec<String>>,
    description: Option<String>,
    #[serde(default)]
    settings: BTreeMap<String, RawSettingDecl>,
}

#[derive(Debug, Deserialize)]
struct RawSettingDecl {
    #[serde(rename = "type")]
    kind: String,
    default: toml::Value,
    label: Option<String>,
    values: Option<Vec<String>>,
}

/// Parse + validate a manifest from its TOML source. `plugin_dir` is
/// the directory the manifest lives in, used to confirm the `entry`
/// file exists and stays inside the directory.
pub fn parse_manifest(toml_src: &str, plugin_dir: &Path) -> Result<PluginManifest, ManifestError> {
    let raw: RawManifest =
        toml::from_str(toml_src).map_err(|e| ManifestError::Toml(e.to_string()))?;
    validate(raw, plugin_dir)
}

fn validate(raw: RawManifest, plugin_dir: &Path) -> Result<PluginManifest, ManifestError> {
    let name = raw.name.unwrap_or_default();
    if name.is_empty() {
        return Err(ManifestError::EmptyName);
    }
    if !is_kebab_case(&name) {
        return Err(ManifestError::BadName(name));
    }

    let version = raw.version.unwrap_or_default();
    semver::Version::parse(&version).map_err(|_| ManifestError::BadVersion(version.clone()))?;

    let entry = raw.entry.unwrap_or_default();
    validate_entry(&entry, plugin_dir)?;

    let raw_hooks = raw.hooks.unwrap_or_default();
    if raw_hooks.is_empty() {
        return Err(ManifestError::EmptyHooks);
    }
    let mut hooks = Vec::with_capacity(raw_hooks.len());
    for h in &raw_hooks {
        match HookKind::from_str(h) {
            Some(k) => hooks.push(k),
            None => return Err(ManifestError::UnknownHook(h.clone())),
        }
    }

    let printer_compatibility = match raw.printer_compatibility {
        None => PrinterCompat::Any,
        Some(list) if list.is_empty() || list.iter().any(|m| m == "any") => PrinterCompat::Any,
        Some(list) => PrinterCompat::Models(list),
    };

    let mut settings = BTreeMap::new();
    for (key, decl) in raw.settings {
        let validated = validate_setting(&key, decl)?;
        settings.insert(key, validated);
    }

    Ok(PluginManifest {
        name,
        version,
        entry,
        hooks,
        printer_compatibility,
        description: raw.description,
        settings,
    })
}

fn validate_entry(entry: &str, plugin_dir: &Path) -> Result<(), ManifestError> {
    let bad = |reason: &str| ManifestError::BadEntry(entry.to_string(), reason.to_string());
    if entry.is_empty() {
        return Err(bad("missing entry file"));
    }
    let rel = Path::new(entry);
    for comp in rel.components() {
        match comp {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir => return Err(bad("must not contain `..`")),
            Component::RootDir | Component::Prefix(_) => {
                return Err(bad("must be a relative path"))
            }
        }
    }
    if rel.extension().and_then(|e| e.to_str()) != Some("lua") {
        return Err(bad("must end in .lua"));
    }
    if !plugin_dir.join(rel).is_file() {
        return Err(bad("file not found"));
    }
    Ok(())
}

fn validate_setting(key: &str, raw: RawSettingDecl) -> Result<SettingDecl, ManifestError> {
    let bad = |reason: String| ManifestError::BadSetting {
        key: key.to_string(),
        reason,
    };
    let kind = match raw.kind.as_str() {
        "string" => SettingKind::String,
        "number" => SettingKind::Number,
        "bool" => SettingKind::Bool,
        "enum" => SettingKind::Enum,
        other => return Err(bad(format!("unknown type `{other}` (string|number|bool|enum)"))),
    };
    let values = raw.values.unwrap_or_default();
    let default = match (kind, &raw.default) {
        (SettingKind::String, toml::Value::String(s)) => SettingValue::String(s.clone()),
        (SettingKind::Number, toml::Value::Integer(i)) => SettingValue::Number(*i as f64),
        (SettingKind::Number, toml::Value::Float(f)) => SettingValue::Number(*f),
        (SettingKind::Bool, toml::Value::Boolean(b)) => SettingValue::Bool(*b),
        (SettingKind::Enum, toml::Value::String(s)) => {
            if values.is_empty() {
                return Err(bad("enum setting needs a non-empty `values` list".to_string()));
            }
            if !values.iter().any(|v| v == s) {
                return Err(bad(format!("default `{s}` is not one of `values`")));
            }
            SettingValue::String(s.clone())
        }
        (k, v) => {
            return Err(bad(format!(
                "default is a {}, but type is {}",
                v.type_str(),
                k.as_str()
            )))
        }
    };
    Ok(SettingDecl {
        kind,
        default,
        label: raw.label,
        values,
    })
}

fn is_kebab_case(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('-')
        && !s.ends_with('-')
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Write `plugin.toml` + a `main.lua` into a fresh temp dir and
    /// return the dir (kept alive by the returned guard).
    fn plugin_dir(manifest: &str, entry_name: &str) -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("plugin.toml"), manifest).unwrap();
        if !entry_name.is_empty() {
            std::fs::write(tmp.path().join(entry_name), "-- lua").unwrap();
        }
        let path = tmp.path().to_path_buf();
        (tmp, path)
    }

    const FULL: &str = r#"
        name = "platecycler"
        version = "0.2.1"
        entry = "main.lua"
        hooks = ["post_slice"]
        printer_compatibility = ["Bambu Lab A1 mini"]
        description = "auto-eject on completion"

        [settings.swap_gcode]
        type = "string"
        default = "M400"
        label = "Swap G-code"
    "#;

    #[test]
    fn parses_a_full_manifest() {
        let (_tmp, dir) = plugin_dir(FULL, "main.lua");
        let m = parse_manifest(FULL, &dir).unwrap();
        assert_eq!(m.name, "platecycler");
        assert_eq!(m.version, "0.2.1");
        assert_eq!(m.entry, "main.lua");
        assert_eq!(m.hooks, vec![HookKind::PostSlice]);
        assert_eq!(
            m.printer_compatibility,
            PrinterCompat::Models(vec!["Bambu Lab A1 mini".into()])
        );
        let s = &m.settings["swap_gcode"];
        assert_eq!(s.kind, SettingKind::String);
        assert_eq!(s.default, SettingValue::String("M400".into()));
    }

    #[test]
    fn defaults_printer_compat_to_any() {
        let src = r#"
            name = "p"
            version = "1.0.0"
            entry = "main.lua"
            hooks = ["pre_slice"]
        "#;
        let (_tmp, dir) = plugin_dir(src, "main.lua");
        assert_eq!(
            parse_manifest(src, &dir).unwrap().printer_compatibility,
            PrinterCompat::Any
        );
    }

    #[test]
    fn rejects_bad_semver() {
        let src = r#"name="p"
version="not-a-version"
entry="main.lua"
hooks=["pre_slice"]"#;
        let (_tmp, dir) = plugin_dir(src, "main.lua");
        assert!(matches!(
            parse_manifest(src, &dir),
            Err(ManifestError::BadVersion(_))
        ));
    }

    #[test]
    fn rejects_unknown_hook() {
        let src = r#"name="p"
version="1.0.0"
entry="main.lua"
hooks=["compose"]"#;
        let (_tmp, dir) = plugin_dir(src, "main.lua");
        assert_eq!(
            parse_manifest(src, &dir),
            Err(ManifestError::UnknownHook("compose".into()))
        );
    }

    #[test]
    fn rejects_empty_hooks() {
        let src = r#"name="p"
version="1.0.0"
entry="main.lua"
hooks=[]"#;
        let (_tmp, dir) = plugin_dir(src, "main.lua");
        assert_eq!(parse_manifest(src, &dir), Err(ManifestError::EmptyHooks));
    }

    #[test]
    fn rejects_non_kebab_name() {
        let src = r#"name="Plate_Cycler"
version="1.0.0"
entry="main.lua"
hooks=["pre_slice"]"#;
        let (_tmp, dir) = plugin_dir(src, "main.lua");
        assert!(matches!(
            parse_manifest(src, &dir),
            Err(ManifestError::BadName(_))
        ));
    }

    #[test]
    fn rejects_entry_escaping_the_dir() {
        let src = r#"name="p"
version="1.0.0"
entry="../evil.lua"
hooks=["pre_slice"]"#;
        let (_tmp, dir) = plugin_dir(src, "main.lua");
        assert!(matches!(
            parse_manifest(src, &dir),
            Err(ManifestError::BadEntry(_, _))
        ));
    }

    #[test]
    fn rejects_missing_entry_file() {
        let src = r#"name="p"
version="1.0.0"
entry="main.lua"
hooks=["pre_slice"]"#;
        // Don't create main.lua.
        let (_tmp, dir) = plugin_dir(src, "");
        assert!(matches!(
            parse_manifest(src, &dir),
            Err(ManifestError::BadEntry(_, _))
        ));
    }

    #[test]
    fn rejects_non_lua_entry() {
        let src = r#"name="p"
version="1.0.0"
entry="main.txt"
hooks=["pre_slice"]"#;
        let (_tmp, dir) = plugin_dir(src, "main.txt");
        assert!(matches!(
            parse_manifest(src, &dir),
            Err(ManifestError::BadEntry(_, _))
        ));
    }

    #[test]
    fn rejects_type_mismatched_setting_default() {
        let src = r#"name="p"
version="1.0.0"
entry="main.lua"
hooks=["pre_slice"]

[settings.count]
type="number"
default="seven"
"#;
        let (_tmp, dir) = plugin_dir(src, "main.lua");
        assert!(matches!(
            parse_manifest(src, &dir),
            Err(ManifestError::BadSetting { .. })
        ));
    }

    #[test]
    fn rejects_enum_default_not_in_values() {
        let src = r#"name="p"
version="1.0.0"
entry="main.lua"
hooks=["pre_slice"]

[settings.mode]
type="enum"
values=["a","b"]
default="c"
"#;
        let (_tmp, dir) = plugin_dir(src, "main.lua");
        assert!(matches!(
            parse_manifest(src, &dir),
            Err(ManifestError::BadSetting { .. })
        ));
    }
}
