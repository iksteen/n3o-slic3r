//! Lua bindings for the host data types handed to plugin hooks.

pub mod filament;
pub mod gcode;
pub mod plugin_settings;
pub mod settings;

pub use filament::{FilamentHandle, FilamentLoadout, SlotInfo};
pub use gcode::{GcodeCell, GcodeHandle};
pub use plugin_settings::build_settings_table;
pub use settings::SettingsHandle;

use mlua::{Lua, Result as LuaResult, Table, Value};

/// Wrap `data` in a read-only proxy table: an empty table whose
/// metatable reads through to `data` and rejects `=` assignment, with
/// the metatable itself hidden (`__metatable = false`). The sandbox
/// strips `rawset`/`getmetatable`/`setmetatable`, so there's no Lua path
/// to mutate it or reach `data` — the view is effectively immutable.
/// Shared by the filament loadout and the plugin-settings views. `what`
/// names the thing in the error (`"<what> is read-only"`).
pub(crate) fn read_only(lua: &Lua, data: Table, what: &'static str) -> LuaResult<Table> {
    let proxy = lua.create_table()?;
    let mt = lua.create_table()?;
    mt.set("__index", data)?;
    let msg = format!("{what} is read-only");
    mt.set(
        "__newindex",
        lua.create_function(move |_, (_, _, _): (Table, Value, Value)| -> LuaResult<()> {
            Err(mlua::Error::RuntimeError(msg.clone()))
        })?,
    )?;
    mt.set("__metatable", false)?;
    proxy.set_metatable(Some(mt))?;
    Ok(proxy)
}
