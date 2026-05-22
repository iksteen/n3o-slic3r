//! Printer drivers.
//!
//! Each driver owns its send path end-to-end, including any wrapping,
//! transformation, or metadata injection the target printer requires.
//! The slicer produces canonical G-code via libslic3r; the driver
//! decides what to do with it before transmission. Adding a future
//! printer is a self-contained exercise — write the driver, declare
//! its capabilities, no shared-code changes (PRD §8.2 note).

pub mod bambu;
pub mod snapmaker;
