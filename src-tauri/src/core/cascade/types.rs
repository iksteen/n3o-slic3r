//! Cascade intermediate representation.
//!
//! Parser output (PR-1-2) and resolver input (PR-1-3). Designed to be
//! cheap to clone and serializable for the Tauri command surface
//! (PR-1-9). All source-location data carries through to the trace
//! tooling (PR-1-5) so the UI can show "winner: filament-rule.toml:14".

use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// A loaded cascade — one or more TOML files merged in load order. The
/// rules list is flat (section-shorthand and `[[rule]]` blocks have been
/// desugared) and ordered by source position; the resolver consumes
/// this directly.
#[derive(Debug, Clone, Serialize)]
pub struct Cascade {
    pub rules: Vec<Rule>,
}

/// A single resolved cascade rule.
///
/// `when.conditions` empty = the unconditional default (specificity 0).
/// The set map's values are serialized strings — libslic3r consumes them
/// via `Config::set(key, &str)` and parses per-key.
#[derive(Debug, Clone, Serialize)]
pub struct Rule {
    pub when: Predicate,
    pub set: BTreeMap<String, String>,
    pub source: SourceLocation,
}

impl Rule {
    /// Number of context-dimension predicates in `when`. Specificity is
    /// the count of distinct dimensions, not predicate complexity — a
    /// set-membership condition like
    /// `when.filament.type = ["PLA", "PETG"]` has specificity 1.
    pub fn specificity(&self) -> usize {
        self.when.conditions.len()
    }

    /// True if this rule has no predicates and applies unconditionally.
    pub fn is_default(&self) -> bool {
        self.when.conditions.is_empty()
    }
}

/// AST of a rule's `when` block. Empty conditions list = unconditional.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Predicate {
    pub conditions: Vec<Condition>,
}

/// A single `when.<dimension> = <value>` clause.
///
/// `dimension` is the dotted-path key as authored (`"filament.type"`,
/// `"plate.type"`, `"printer.model"`); the resolver looks it up against
/// the live `Context`. `value` is preserved as-authored — the resolver
/// (PR-1-3) interprets richer operator forms (`">= 0.6"`, `"!= Cool"`)
/// from the value string rather than from a parser-level operator
/// variant. Set membership uses the array form
/// (`when.filament.type = ["PLA", "PETG"]`).
#[derive(Debug, Clone, Serialize)]
pub struct Condition {
    pub dimension: String,
    pub value: ConditionValue,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum ConditionValue {
    Scalar(String),
    Array(Vec<String>),
}

/// Where a rule (or a single offending key) came from. Renders as
/// `file:line` in error messages and the trace UI.
#[derive(Debug, Clone, Serialize)]
pub struct SourceLocation {
    pub path: PathBuf,
    pub line: u32,
}

impl std::fmt::Display for SourceLocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.path.display(), self.line)
    }
}
