//! Plugin host errors.

use thiserror::Error;

/// Errors from loading or running a plugin's Lua code. The `Display`
/// impl yields a user-facing message (surfaced in the Plugins panel).
#[derive(Debug, Error)]
pub enum PluginError {
    /// The Lua chunk failed to compile, or building the sandbox /
    /// installing its limits failed. Surfaces at load time, before any
    /// hook runs.
    #[error("plugin load error: {0}")]
    Load(String),

    /// A runtime error raised while executing plugin Lua (a Lua
    /// `error(...)`, a nil index, a type error, etc.).
    #[error("plugin runtime error: {0}")]
    Runtime(String),

    /// The plugin exceeded its instruction budget and was aborted
    /// mid-call. Distinguished from [`PluginError::Runtime`] so the
    /// host can report "runaway plugin" specifically.
    #[error("plugin exceeded its instruction budget")]
    Timeout,

    /// A named global the host tried to call was present but not a
    /// function.
    #[error("plugin returned an unexpected value: {0}")]
    BadReturn(String),
}
