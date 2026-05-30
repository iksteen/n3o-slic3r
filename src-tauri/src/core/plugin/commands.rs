//! Tauri command surface for the plugin host.
//!
//! The host lives in shared state as `Arc<Mutex<PluginHost>>`. Mutating
//! commands emit `plugin:changed` so the Plugins panel re-fetches.

use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, State};

use super::host::{PluginHost, PluginSummary};

/// Managed-state alias for the shared host.
pub type PluginHostState = Arc<Mutex<PluginHost>>;

/// Emitted whenever the plugin set or any plugin's state changes.
pub const CHANGED_EVENT: &str = "plugin:changed";

#[tauri::command]
pub fn plugin_list(host: State<'_, PluginHostState>) -> Result<Vec<PluginSummary>, String> {
    let host = host.lock().map_err(|e| e.to_string())?;
    Ok(host.list())
}

#[tauri::command]
pub fn plugin_set_enabled(
    name: String,
    enabled: bool,
    host: State<'_, PluginHostState>,
    app: AppHandle,
) -> Result<(), String> {
    {
        let mut host = host.lock().map_err(|e| e.to_string())?;
        host.set_enabled(&name, enabled).map_err(|e| e.to_string())?;
    }
    let _ = app.emit(CHANGED_EVENT, ());
    Ok(())
}

#[tauri::command]
pub fn plugin_reload(
    name: String,
    host: State<'_, PluginHostState>,
    app: AppHandle,
) -> Result<(), String> {
    {
        let mut host = host.lock().map_err(|e| e.to_string())?;
        host.reload(&name).map_err(|e| e.to_string())?;
    }
    let _ = app.emit(CHANGED_EVENT, ());
    Ok(())
}
