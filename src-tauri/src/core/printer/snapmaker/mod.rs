//! Snapmaker U1 driver (HTTP API over LAN).
//!
//! Send format: plain `.gcode` (Klipper-based firmware accepts raw
//! G-code over HTTP). Status: state + currently-mounted toolhead +
//! per-toolhead loaded filament + nozzle/bed temperatures. Tool
//! offsets are managed by the printer (eddy current sensor); we
//! surface them read-only. Commands: pause, resume, stop.
//!
//! This driver targets Snapmaker's HTTP wrapper specifically, not
//! Klipper's Moonraker (PRD AD-7). A future generic Klipper driver
//! is a separate driver, not a generalization of this one.
//!
//! Owns FR-SU-1 through FR-SU-9 (PRD §6.7). Implementation lands in
//! Phase 7.
