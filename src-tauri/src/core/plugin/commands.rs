//! Tauri command surface for the plugin host.
//!
//! The host lives in shared state as `Arc<Mutex<PluginHost>>`. Mutating
//! commands emit `plugin:changed` so the Plugins panel re-fetches.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tauri::{AppHandle, Emitter, State};

use super::host::{PluginHost, PluginSummary};

/// Managed-state alias for the shared host.
pub type PluginHostState = Arc<Mutex<PluginHost>>;

/// Emitted whenever the plugin set or any plugin's state changes.
pub const CHANGED_EVENT: &str = "plugin:changed";

/// Lock the host, recovering the guard if a plugin panic poisoned the
/// mutex — a poisoned host should keep serving the panel, not error
/// every command for the rest of the process.
fn lock_host(host: &PluginHostState) -> MutexGuard<'_, PluginHost> {
    host.lock().unwrap_or_else(PoisonError::into_inner)
}

#[tauri::command]
pub fn plugin_list(host: State<'_, PluginHostState>) -> Result<Vec<PluginSummary>, String> {
    Ok(lock_host(&host).list())
}

#[tauri::command]
pub fn plugin_set_enabled(
    name: String,
    enabled: bool,
    host: State<'_, PluginHostState>,
    app: AppHandle,
) -> Result<(), String> {
    lock_host(&host)
        .set_enabled(&name, enabled)
        .map_err(|e| e.to_string())?;
    let _ = app.emit(CHANGED_EVENT, ());
    Ok(())
}

#[tauri::command]
pub fn plugin_reload(
    name: String,
    host: State<'_, PluginHostState>,
    app: AppHandle,
) -> Result<(), String> {
    lock_host(&host).reload(&name).map_err(|e| e.to_string())?;
    let _ = app.emit(CHANGED_EVENT, ());
    Ok(())
}
