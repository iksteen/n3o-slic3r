//! Core backend modules.
//!
//! The module boundaries here mirror PRD §8.2. Each submodule owns a
//! cohesive slice of the backend's responsibilities and exposes either
//! Tauri commands (renderer-callable surface) or Rust APIs consumed by
//! other modules. The renderer (`src/`) talks to this tree only through
//! Tauri commands and events — never via direct calls.

pub mod cascade;
pub mod cascade_adapter;
pub mod filament;
pub mod gcode;
pub mod logging;
pub mod plugin;
pub mod printer;
pub mod project;
pub mod scene;
pub mod schema;
pub mod slice;
pub mod threemf;
