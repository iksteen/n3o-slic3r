//! The plugin sandbox: a restricted Lua 5.4 runtime.
//!
//! Allow-list (stdlib loaded): `string`, `table`, `math`, `coroutine`,
//! plus the always-present base library (`print`, `pairs`, `pcall`,
//! `type`, `tostring`, …), and a curated `os` shim exposing only
//! `os.time` / `os.clock`.
//!
//! Deny-list (never loaded, or stripped): `io`, the real `os`
//! (`execute` / `getenv` / `remove` / `rename` / `exit` / …),
//! `package` / `require`, `debug`, and the base-library dynamic
//! loaders `load` / `loadstring` / `loadfile` / `dofile`. The first
//! three groups are simply not loaded via the `StdLib` selection; the
//! loaders ride in the always-present base library, so we strip them
//! explicitly.
//!
//! Resource bounds (instruction budget, memory limit) live on the
//! `PluginRuntime` in `runtime.rs`, not here — this module only shapes
//! the *capability* surface.

use std::sync::OnceLock;
use std::time::Instant;

use mlua::{Lua, LuaOptions, StdLib, Value};

use super::error::PluginError;

/// Base-library globals that ship with every Lua state regardless of
/// the `StdLib` selection and that we strip: arbitrary-string code
/// loading and filesystem loaders.
const DENIED_BASE_GLOBALS: &[&str] = &["load", "loadstring", "loadfile", "dofile"];

/// Build a fresh sandboxed Lua runtime. The caller (`PluginRuntime`)
/// layers instruction + memory limits on top before running anything.
pub fn build_sandbox() -> Result<Lua, PluginError> {
    // Only the pure, capability-free standard libraries. `io`, `os`,
    // `package`, and `debug` are deliberately absent.
    let libs = StdLib::STRING | StdLib::TABLE | StdLib::MATH | StdLib::COROUTINE;
    let lua = Lua::new_with(libs, LuaOptions::default())
        .map_err(|e| PluginError::Load(format!("sandbox init: {e}")))?;

    strip_denied_globals(&lua)?;
    install_os_shim(&lua)?;
    Ok(lua)
}

fn strip_denied_globals(lua: &Lua) -> Result<(), PluginError> {
    let globals = lua.globals();
    for name in DENIED_BASE_GLOBALS {
        globals
            .set(*name, Value::Nil)
            .map_err(|e| PluginError::Load(format!("strip `{name}`: {e}")))?;
    }
    Ok(())
}

/// Install a hand-curated `os` table holding only the two
/// non-capability functions plugins legitimately want for
/// timing/jitter. Never exposes `os.execute`, `os.getenv`,
/// `os.remove`, `os.rename`, `os.exit`, etc.
fn install_os_shim(lua: &Lua) -> Result<(), PluginError> {
    let os = lua.create_table().map_err(shim_err)?;

    // os.time() -> integer Unix seconds. Wall-clock only; a plugin
    // using it is non-deterministic by its own choice, but it is not a
    // security surface.
    let time = lua
        .create_function(|_, ()| {
            let secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            Ok(secs)
        })
        .map_err(shim_err)?;
    os.set("time", time).map_err(shim_err)?;

    // os.clock() -> seconds (f64) since first sandbox construction.
    // Lua's real os.clock is process CPU time; a monotonic
    // process-relative clock is close enough for plugin timing and
    // avoids pulling in the real os library.
    let clock = lua
        .create_function(|_, ()| Ok(process_start().elapsed().as_secs_f64()))
        .map_err(shim_err)?;
    os.set("clock", clock).map_err(shim_err)?;

    lua.globals().set("os", os).map_err(shim_err)?;
    Ok(())
}

fn process_start() -> Instant {
    static START: OnceLock<Instant> = OnceLock::new();
    *START.get_or_init(Instant::now)
}

fn shim_err(e: mlua::Error) -> PluginError {
    PluginError::Load(format!("os shim: {e}"))
}
