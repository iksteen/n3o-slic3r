//! Snapmaker U1 printer-profile adapter (cascade-layer side).
//!
//! This module owns the *cascade-side* knowledge of the U1 printer
//! profile: bed identities, toolhead topology, the printer fragment
//! slug `snapmaker-u1`. Per-toolhead nozzle config (FR-SU-6) is
//! authored under `profiles/snapmaker/printer/snapmaker-u1/`.
//!
//! **Driver-side comms live elsewhere**: `core/driver/snapmaker/`
//! holds the actual Moonraker HTTP+WS client (PR-7b). FR-SU-1
//! through FR-SU-9 are split — the cascade fields (FR-SU-6/7/8/9)
//! live here; the network surface (FR-SU-1/2/3/4/5) lives in the
//! driver module.
//!
//! Architecture clarification (corrected in PR-7b-6 — see PRD AD-7
//! living-document note): the U1 exposes vanilla Moonraker on plain
//! HTTP+WS port 80, not a Snapmaker-proprietary wrapper. The
//! Snapmaker-specific pair / mTLS / MQTT control plane (used for
//! the webcam) is out of MVP scope. The prior version of this
//! comment claimed otherwise; corrected per PRD §11.3.
