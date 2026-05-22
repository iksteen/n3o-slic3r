//! 3MF reader and writer utility.
//!
//! Used by project save/load (our own .3mf format extends standard
//! 3MF), project import from other slicers (Bambu Studio, OrcaSlicer,
//! Snapmaker Orca all save .3mf), preview drag-drop of `.gcode.3mf`,
//! and the Bambu driver's send-format wrapping. The U1 driver does not
//! depend on this module — it sends raw G-code.
//!
//! Implementation lands in Phase 3.
