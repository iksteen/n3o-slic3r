//! Cascade context wire types.
//!
//! The frontend resolves per-plate via `project::commands::
//! plate_cascade_resolve`, and the slice path composes the cascade
//! directly in `core::slice` — so the old stateful cascade Tauri
//! command surface (load / resolve / trace, keyed by a registry handle)
//! is gone. What remains is the serialized context the slice input
//! builder fills in: `ContextJson` (printer + plate + filaments +
//! override tiers) and its `OverrideFileSpec` rows.

use crate::core::filament::FilamentProfile;
use crate::core::printer::PrinterProfile;
use crate::core::scene::BuildPlate;
use serde::Deserialize;
use std::collections::HashMap;

/// Serialized slicing context. The slice input builder rebuilds this
/// from the project's in-memory model — cheaper than persisting a
/// resolved `SlicingContext`, which would have to invalidate on every
/// printer / filament / plate switch.
#[derive(Debug, Clone, Deserialize)]
pub struct ContextJson {
    pub printer: PrinterProfile,
    pub plate: BuildPlate,
    pub filaments: Vec<FilamentProfile>,
    #[serde(default)]
    pub active_slot: usize,
    #[serde(default)]
    pub user_overrides: Vec<OverrideFileSpec>,
    #[serde(default)]
    pub project_overrides: Vec<OverrideFileSpec>,
    /// Per-object cascade overrides. When the panel is in
    /// the Object tab, this carries the active object's authored
    /// overrides; otherwise empty / absent. Highest-priority tier:
    /// beats both user and project overrides.
    #[serde(default)]
    pub object_overrides: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OverrideFileSpec {
    pub label: String,
    pub content: String,
}
