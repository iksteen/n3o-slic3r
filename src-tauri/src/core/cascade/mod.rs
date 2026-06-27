//! Rule cascade resolver.
//!
//! Loads TOML rule files, validates predicates and set keys against the
//! libslic3r schema, accepts a context object, and returns resolved
//! settings with trace metadata (winning rule's file:line and
//! specificity, list of also-matching losers, override-tier source).
//! Two-phase resolution per `docs/dev/profiles.md`: authored cascade with
//! specificity-and-source-order, then `!important`-style user / project /
//! object override tiers.
//!
//! Owns FR-CAS-1 through FR-CAS-13 (PRD §6.1). The submodules:
//!
//! - **`types`**: the typed cascade IR — `Cascade`, `Rule`,
//!   `Predicate`, `SourceLocation`. Sharable across resolver, adapter,
//!   trace tooling, and the Tauri command surface.
//! - **`loader`**: TOML parser that desugars the three authoring
//!   forms (top-level keys, `[section.shorthand]`, `[[rule]]`)
//!   into the IR and load-validates against the libslic3r schema.
//! - **`resolver`**, **`overrides`**, **`trace`**: per-key
//!   resolution, override tiers, and inspection tooling.

pub mod commands;
pub mod loader;
pub mod overrides;
pub mod resolver;
pub mod trace;
pub mod types;
pub mod validate;

pub use commands::{ContextJson, OverrideFileSpec};
pub use loader::{load_cascade, CascadeLoadError};
pub use overrides::{
    load_override_file, parse_override_str, resolve_with_overrides, to_resolved, FlatOverrides,
    OverrideTier, OverrideTiers, OverrideTraceEntry, ResolvedOverrides, ResolvedWithTrace,
};
pub use resolver::{
    format_when, resolve, Context, MapContext, MatchingRule, Resolved, ResolvedValue,
};
pub use trace::{trace, Trace, TraceRule, TraceSource};
pub use types::{Cascade, Condition, ConditionValue, Predicate, Rule, SourceLocation};
pub use validate::{default_known_dimensions, validate_cascade, KnownDimensions};
