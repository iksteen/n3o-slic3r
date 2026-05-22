//! Lua plugin host.
//!
//! Embeds Lua 5.4 via mlua, sandboxed (no `io`, `os.execute`,
//! `package` access by default). Loads plugin manifests, dispatches
//! pre-slice / post-slice / pre-send / compose hooks, exposes
//! read-only views of project / typed-gcode / filament state.
//!
//! Owns FR-PL-1 through FR-PL-9 (PRD §6.9). Implementation lands in
//! Phase 8. The platecycler plugin (compose hook) is the MVP's proof
//! point for the plugin architecture.
