//! Renderer-agnostic 3D scene state.
//!
//! Authoritative model for the 3D viewport: mesh registry, per-object
//! transforms and metadata, hierarchy, selection state, gizmo state,
//! camera state, exclusion-zone data per the active plate's printer.
//! The frontend renderer (Three.js for MVP, possibly wgpu later) is a
//! read-only consumer that reflects state changes via Tauri events;
//! all mutations enter through commands defined here.
//!
//! This module exists so the renderer is swappable without rewriting
//! state management — see PRD FR-3D-7 and AD-8. The renderer-vs-state
//! ownership boundary is load-bearing; resist the urge to let the
//! frontend hold authoritative state even for "just this one case."
//!
//! Implementation lands in Phase 2. Performance contract: state ops
//! ≤5ms p99 on 1000-object scenes (PRD AD-8).
