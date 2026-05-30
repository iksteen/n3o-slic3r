//! The read-only, typed `settings` global handed to a plugin's hooks —
//! the plugin's OWN declared settings, resolved for this slice context.
//!
//! Resolved values arrive as flat strings (the cascade vocabulary); each
//! declared setting is converted to its manifest-declared Lua type so a
//! plugin reads `settings.layer` as a number, `settings.enabled` as a
//! bool, `settings.swap_gcode` as a string. Read-only via the shared
//! proxy.

use std::collections::BTreeMap;

use mlua::{Lua, Result as LuaResult, Table};

use crate::core::plugin::{SettingDecl, SettingKind};

/// Build the read-only `settings` table: each *declared* setting,
/// converted from its resolved flat-string value to its declared type.
/// Undeclared resolved keys are not exposed (the manifest is the
/// contract); a numeric value that won't parse falls back to its string.
pub fn build_settings_table(
    lua: &Lua,
    declared: &BTreeMap<String, SettingDecl>,
    resolved: &BTreeMap<String, String>,
) -> LuaResult<Table> {
    let data = lua.create_table()?;
    for (key, decl) in declared {
        let Some(raw) = resolved.get(key) else {
            continue;
        };
        match decl.kind {
            SettingKind::String | SettingKind::Enum => data.set(key.as_str(), raw.clone())?,
            SettingKind::Number => match raw.trim().parse::<f64>() {
                Ok(n) => data.set(key.as_str(), n)?,
                Err(_) => data.set(key.as_str(), raw.clone())?,
            },
            SettingKind::Bool => data.set(key.as_str(), matches!(raw.trim(), "true" | "1"))?,
        }
    }
    super::read_only(lua, data, "settings")
}
