//! Bambu Lab printer driver (MQTT over LAN).
//!
//! Send format: `.gcode.3mf` with Bambu metadata extensions. Status:
//! state machine + AMS lite slot contents + nozzle/bed temperatures +
//! current layer. Commands: pause, resume, stop. No cloud dependency
//! — access code + serial only.
//!
//! Owns FR-BL-1 through FR-BL-6 (PRD §6.7). Implementation lands in
//! Phase 7.
