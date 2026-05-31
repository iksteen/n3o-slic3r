//! Resolved-settings Lua binding for the pre-slice hook.
//!
//! Exposes the resolved cascade as a `settings` userdata that reads and
//! writes like a plain table (`settings.bed_temp`, `settings.bed_temp =
//! "55"`) via `__index` / `__newindex`. Values are strings (libslic3r's
//! config vocabulary); an integer is stringified exactly, a float is
//! rejected (use `tostring()` so the plugin controls the formatting and
//! avoids `0.1+0.2 → "0.30000000000000004"`). Settings are modify/add
//! only — assigning `nil` is a no-op, not a removal, so a plugin can't
//! silently drop a setting out of the cascade.
//!
//! Like the G-code binding, the map lives behind a shared `Arc<Mutex>`
//! so the host reads a plugin's edits back from the cell after the hook
//! — no return value needed.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use mlua::{MetaMethod, UserData, UserDataMethods, Value};

/// Shared key→value settings map behind the userdata.
pub type SettingsCell = Arc<Mutex<BTreeMap<String, String>>>;

pub struct SettingsHandle {
    map: SettingsCell,
}

impl SettingsHandle {
    pub fn new(map: BTreeMap<String, String>) -> Self {
        Self {
            map: Arc::new(Mutex::new(map)),
        }
    }

    /// The shared cell, for the host to read edits back after the hook.
    pub fn cell(&self) -> SettingsCell {
        self.map.clone()
    }
}

fn lock(cell: &SettingsCell) -> MutexGuard<'_, BTreeMap<String, String>> {
    cell.lock().unwrap_or_else(PoisonError::into_inner)
}

impl UserData for SettingsHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // settings.key -> value string, or nil if unset.
        methods.add_meta_method(MetaMethod::Index, |_, this, key: String| {
            Ok(lock(&this.map).get(&key).cloned())
        });
        // settings.key = value : a string or integer sets it; nil is a
        // no-op (no removal); a float or other type is rejected.
        methods.add_meta_method(
            MetaMethod::NewIndex,
            |_, this, (key, value): (String, Value)| {
                match value {
                    Value::Nil => {}
                    Value::String(s) => {
                        lock(&this.map).insert(key, s.to_str()?.to_owned());
                    }
                    Value::Integer(i) => {
                        lock(&this.map).insert(key, i.to_string());
                    }
                    Value::Number(_) => {
                        return Err(mlua::Error::RuntimeError(format!(
                            "setting `{key}`: use tostring() for fractional values so the \
                         plugin controls the formatting"
                        )))
                    }
                    other => {
                        return Err(mlua::Error::RuntimeError(format!(
                            "setting `{key}` must be a string, got {}",
                            other.type_name()
                        )))
                    }
                }
                Ok(())
            },
        );
    }
}
