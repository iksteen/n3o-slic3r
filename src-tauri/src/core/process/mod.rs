//! User process (quality) profile overrides.
//!
//! Stamping the current quality settings onto a bundled process profile as
//! a per-user diff. The override library + persistence live in [`library`];
//! the stamp itself (which reads a plate's project-tier overrides) is a
//! project mutation and lives in `core::project::commands`.

pub mod library;

pub use library::UserProcess;

// Revert (in-place) and Delete (named custom) both touch the bound plate —
// they optionally apply the profile's settings back to the plate's project
// tier before removing it, and Delete repoints the plate. They live with the
// other plate mutations in `core::project::commands`.
