//! Lua plugin host.
//!
//! Embeds Lua 5.4 via mlua, sandboxed (no `io`, `os.execute`,
//! `package`, or `debug` access). Will load plugin manifests,
//! dispatch pre-slice / post-slice / pre-send hooks, and expose
//! read-only views of project / typed-gcode / filament state.
//!
//! Owns FR-PL-1 through FR-PL-9 (PRD §6.9), minus the compose hook
//! (FR-PL-5), which is deferred post-MVP.
//!
//! This module currently holds the foundation: [`sandbox`] builds the
//! restricted runtime, and [`PluginRuntime`] loads a single plugin
//! chunk and calls its functions under instruction + memory bounds.
//! The manifest loader, the multi-plugin host + hook dispatch, and the
//! typed-G-code bindings build on top of this primitive.

pub mod commands;
mod discovery;
mod error;
mod host;
mod manifest;
mod runtime;
mod sandbox;

pub use discovery::{discover, DiscoveredPlugin, MANIFEST_FILE};
pub use error::PluginError;
pub use host::{user_plugins_dir, Hook, LoadedPlugin, PluginHost, PluginSummary};
pub use manifest::{
    HookKind, ManifestError, PluginManifest, PrinterCompat, SettingDecl, SettingKind, SettingValue,
};
pub use runtime::PluginRuntime;
