//! Override-tier resolution (PR-1-4).
//!
//! Implements the *absolute-override* half of the two-phase resolution
//! described in `docs/profiles.md` — user profile and project file
//! applied as `!important`-style tiers on top of the authored cascade
//! from `resolver.rs`. Project tier ranks higher than user; later
//! source within the same tier wins on ties.
//!
//! Override files have a *flatter* shape than cascade files: every
//! entry is an unconditional top-level `key = value`. No `[[rule]]`
//! blocks, no `when.*` predicates, no section shorthand. The stricter
//! loader [`load_override_file`] enforces this at parse time so a
//! `[[rule]]` block in an override file fails fast with file:line
//! rather than getting silently treated as the authored-cascade form.

use super::loader::CascadeLoadError;
use super::resolver::{resolve, Context, MatchingRule, Resolved, ResolvedValue};
use super::types::{Cascade, SourceLocation};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// Stack of override files for the resolver to apply over the cascade.
///
/// Both tiers are vectors of files in load order. `resolve_with_overrides`
/// applies them in the order *user* first, then *project* — project
/// values win when both tiers touch the same key.
#[derive(Debug, Clone, Default)]
pub struct OverrideTiers {
    pub user: Vec<FlatOverrides>,
    pub project: Vec<FlatOverrides>,
}

impl OverrideTiers {
    pub fn empty() -> Self {
        Self::default()
    }
}

/// A single loaded override file. `entries` keys are libslic3r option
/// names; values are serialized libslic3r-shaped strings (same shape as
/// `Rule::set`). `source` points at the file itself, used for trace +
/// the same-tier source-order warning.
#[derive(Debug, Clone)]
pub struct FlatOverrides {
    pub source: SourceLocation,
    pub entries: BTreeMap<String, String>,
}

/// Which override tier won a key. Renders into traces as
/// "tier=project file=foo.toml:1".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum OverrideTier {
    User,
    Project,
}

impl std::fmt::Display for OverrideTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::User => write!(f, "user"),
            Self::Project => write!(f, "project"),
        }
    }
}

/// Per-key resolved value carrying override + cascade-fallback metadata.
///
/// `value` is what slice time consumes. When `override_source` is `None`,
/// `value` equals `cascade.value`; the override fields are empty. When
/// an override tier wins, `cascade_fallback` records what the authored
/// cascade *would have* resolved to — drives the "Reset to cascade" UI
/// in Phase 4.
#[derive(Debug, Clone)]
pub struct ResolvedWithTrace {
    pub value: String,
    pub winning_rule: SourceLocation,
    pub winning_specificity: usize,
    pub matching_rules: Vec<MatchingRule>,
    pub override_source: Option<OverrideTraceEntry>,
    pub cascade_fallback: Option<String>,
}

#[derive(Debug, Clone)]
pub struct OverrideTraceEntry {
    pub tier: OverrideTier,
    pub source: SourceLocation,
    pub value: String,
}

/// Flat per-key map. Same shape as the basic `Resolved` but each entry
/// carries `ResolvedWithTrace`.
pub type ResolvedOverrides = BTreeMap<String, ResolvedWithTrace>;

/// Parse an override file with the stricter shape: only top-level
/// `key = value` entries; no `[[rule]]`, no `[section.shorthand]`, no
/// `when.*`. Rejection at parse time means UI typos surface as
/// file:line errors immediately rather than at slice time.
pub fn load_override_file(path: &Path) -> Result<FlatOverrides, CascadeLoadError> {
    let src = fs::read_to_string(path).map_err(|e| CascadeLoadError::Io {
        path: path.into(),
        source: e,
    })?;
    parse_override_str(&src, path)
}

pub fn parse_override_str(src: &str, path: &Path) -> Result<FlatOverrides, CascadeLoadError> {
    let parsed: toml::Value = src
        .parse::<toml::Value>()
        .map_err(|e| CascadeLoadError::TomlParse {
            path: path.into(),
            message: e.to_string(),
        })?;
    let root = parsed.as_table().ok_or_else(|| CascadeLoadError::TomlParse {
        path: path.into(),
        message: "expected a table at the file root".into(),
    })?;

    let mut entries = BTreeMap::new();
    for (key, value) in root {
        if key == "rule" {
            return Err(CascadeLoadError::InvalidShape {
                location: SourceLocation {
                    path: path.into(),
                    line: 1,
                },
                message: "override files must not contain `[[rule]]` blocks — \
                         the override tier is unconditional (no `when.*` predicates)"
                    .into(),
            });
        }
        if value.is_table() {
            return Err(CascadeLoadError::InvalidShape {
                location: SourceLocation {
                    path: path.into(),
                    line: 1,
                },
                message: format!(
                    "override files must not contain section headers like [{key}] — \
                     the override tier is unconditional. Use top-level `key = value` entries."
                ),
            });
        }
        let serialized = value_to_string(key, value, path)?;
        entries.insert(key.clone(), serialized);
    }

    Ok(FlatOverrides {
        source: SourceLocation {
            path: path.into(),
            line: 1,
        },
        entries,
    })
}

/// Resolve the cascade *with* override tiers. Returns the cascade
/// resolution + the override application, with cascade_fallback
/// retained per overridden key.
///
/// Order of application: (1) authored cascade resolve(), (2) user
/// overrides in load order, (3) project overrides in load order. Each
/// layer fully overrides the previous for any key it touches.
pub fn resolve_with_overrides(
    cascade: &Cascade,
    overrides: &OverrideTiers,
    ctx: &dyn Context,
) -> ResolvedOverrides {
    let base = resolve(cascade, ctx);
    let mut out: ResolvedOverrides = base
        .into_iter()
        .map(|(k, v)| (k, base_entry_to_with_trace(v)))
        .collect();

    apply_tier(&mut out, &overrides.user, OverrideTier::User);
    apply_tier(&mut out, &overrides.project, OverrideTier::Project);
    out
}

fn base_entry_to_with_trace(v: ResolvedValue) -> ResolvedWithTrace {
    ResolvedWithTrace {
        value: v.value,
        winning_rule: v.winning_rule,
        winning_specificity: v.winning_specificity,
        matching_rules: v.matching_rules,
        override_source: None,
        cascade_fallback: None,
    }
}

fn apply_tier(
    out: &mut ResolvedOverrides,
    files: &[FlatOverrides],
    tier: OverrideTier,
) {
    for file in files {
        for (key, value) in &file.entries {
            let entry = OverrideTraceEntry {
                tier,
                source: file.source.clone(),
                value: value.clone(),
            };
            match out.get_mut(key) {
                Some(existing) => {
                    // Same-tier source-order warning: when two override
                    // files at the same tier touch the same key, the
                    // later one wins but the author probably didn't
                    // intend the dependency.
                    if let Some(prior_override) = &existing.override_source {
                        if prior_override.tier == tier
                            && prior_override.source.path != file.source.path
                        {
                            tracing::warn!(
                                key = %key,
                                tier = %tier,
                                prior_source = %prior_override.source,
                                new_source = %file.source,
                                "override tier source-order tie: two {tier} files set this key — \
                                 later (this) file wins"
                            );
                        }
                    }
                    // Record cascade fallback the FIRST time we override
                    // this key (so cross-tier User→Project doesn't
                    // overwrite the original cascade value).
                    if existing.override_source.is_none() {
                        existing.cascade_fallback = Some(existing.value.clone());
                    }
                    existing.value = value.clone();
                    existing.override_source = Some(entry);
                }
                None => {
                    // Override sets a key the cascade didn't — fine,
                    // no fallback to record.
                    out.insert(
                        key.clone(),
                        ResolvedWithTrace {
                            value: value.clone(),
                            // No cascade winner — synthesize a "no
                            // rule" placeholder. Trace tooling
                            // (PR-1-5) distinguishes these from
                            // cascade-overridden entries via
                            // matching_rules.is_empty().
                            winning_rule: file.source.clone(),
                            winning_specificity: 0,
                            matching_rules: Vec::new(),
                            override_source: Some(entry),
                            cascade_fallback: None,
                        },
                    );
                }
            }
        }
    }
}

fn value_to_string(
    key: &str,
    value: &toml::Value,
    path: &Path,
) -> Result<String, CascadeLoadError> {
    match value {
        toml::Value::String(s) => Ok(s.clone()),
        toml::Value::Integer(i) => Ok(i.to_string()),
        toml::Value::Float(f) => Ok(if f.fract() == 0.0 && f.is_finite() {
            format!("{}", *f as i64)
        } else {
            format!("{f}")
        }),
        toml::Value::Boolean(b) => Ok(b.to_string()),
        toml::Value::Array(items) => {
            let mut parts = Vec::with_capacity(items.len());
            for item in items {
                let s = match item {
                    toml::Value::String(s) => s.clone(),
                    toml::Value::Integer(i) => i.to_string(),
                    toml::Value::Float(f) => format!("{f}"),
                    toml::Value::Boolean(b) => b.to_string(),
                    _ => {
                        return Err(CascadeLoadError::InvalidShape {
                            location: SourceLocation {
                                path: path.into(),
                                line: 1,
                            },
                            message: format!("override `{key}` array element is not a scalar"),
                        });
                    }
                };
                parts.push(s);
            }
            Ok(parts.join(","))
        }
        toml::Value::Table(_) => Err(CascadeLoadError::InvalidShape {
            location: SourceLocation {
                path: path.into(),
                line: 1,
            },
            message: format!("override `{key}` must be a leaf value (scalar or array)"),
        }),
        toml::Value::Datetime(_) => Err(CascadeLoadError::InvalidShape {
            location: SourceLocation {
                path: path.into(),
                line: 1,
            },
            message: format!("override `{key}` datetimes are not supported"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::cascade::loader::parse_cascade_str;
    use crate::core::cascade::resolver::MapContext;
    use std::path::Path;

    fn parse_cascade(src: &str) -> Cascade {
        Cascade {
            rules: parse_cascade_str(src, Path::new("cascade.toml")).expect("cascade parse"),
        }
    }

    fn parse_override(src: &str, name: &str) -> FlatOverrides {
        parse_override_str(src, Path::new(name)).expect("override parse")
    }

    fn pla_pei() -> MapContext {
        MapContext::with([("filament.type", "PLA"), ("plate.type", "PEI")])
    }

    #[test]
    fn no_overrides_behaves_like_resolve() {
        let cascade = parse_cascade("bed_temp = 50\n");
        let result = resolve_with_overrides(&cascade, &OverrideTiers::empty(), &pla_pei());
        let v = result.get("bed_temp").unwrap();
        assert_eq!(v.value, "50");
        assert!(v.override_source.is_none());
        assert!(v.cascade_fallback.is_none());
    }

    #[test]
    fn project_override_beats_specificity_2_rule() {
        let cascade = parse_cascade(
            "\
[[rule]]
when.filament.type = \"PLA\"
when.plate.type = \"PEI\"
set.bed_temp = 55
",
        );
        let project = parse_override("bed_temp = 50\n", "project.toml");
        let overrides = OverrideTiers {
            user: vec![],
            project: vec![project],
        };
        let result = resolve_with_overrides(&cascade, &overrides, &pla_pei());
        let v = result.get("bed_temp").unwrap();
        assert_eq!(v.value, "50", "project override wins");
        assert_eq!(v.cascade_fallback.as_deref(), Some("55"));
        let os = v.override_source.as_ref().unwrap();
        assert_eq!(os.tier, OverrideTier::Project);
    }

    #[test]
    fn user_override_behaves_like_project_except_tier_marker() {
        let cascade = parse_cascade(
            "[[rule]]\nwhen.filament.type = \"PLA\"\nset.bed_temp = 55\n",
        );
        let user = parse_override("bed_temp = 50\n", "user.toml");
        let overrides = OverrideTiers {
            user: vec![user],
            project: vec![],
        };
        let result = resolve_with_overrides(&cascade, &overrides, &pla_pei());
        let v = result.get("bed_temp").unwrap();
        assert_eq!(v.value, "50");
        assert_eq!(v.cascade_fallback.as_deref(), Some("55"));
        assert_eq!(v.override_source.as_ref().unwrap().tier, OverrideTier::User);
    }

    #[test]
    fn project_beats_user_when_both_override_same_key() {
        let cascade = parse_cascade("bed_temp = 55\n");
        let user = parse_override("bed_temp = 50\n", "user.toml");
        let project = parse_override("bed_temp = 45\n", "project.toml");
        let overrides = OverrideTiers {
            user: vec![user],
            project: vec![project],
        };
        let result = resolve_with_overrides(&cascade, &overrides, &pla_pei());
        let v = result.get("bed_temp").unwrap();
        assert_eq!(v.value, "45", "project wins");
        assert_eq!(v.override_source.as_ref().unwrap().tier, OverrideTier::Project);
        assert_eq!(
            v.cascade_fallback.as_deref(),
            Some("55"),
            "cascade_fallback retained even with intermediate user override"
        );
    }

    #[test]
    fn override_only_key_synthesizes_resolved_entry() {
        let cascade = parse_cascade("layer_height = 0.2\n");
        let project = parse_override("bed_temp = 60\n", "project.toml");
        let overrides = OverrideTiers {
            user: vec![],
            project: vec![project],
        };
        let result = resolve_with_overrides(&cascade, &overrides, &pla_pei());
        let v = result.get("bed_temp").unwrap();
        assert_eq!(v.value, "60");
        assert!(v.cascade_fallback.is_none(), "no fallback (cascade didn't set it)");
        assert!(v.matching_rules.is_empty(), "no matching cascade rules");
    }

    #[test]
    fn rule_array_in_override_is_rejected() {
        let src = "[[rule]]\nwhen.filament.type = \"PLA\"\nset.bed_temp = 50\n";
        let err = parse_override_str(src, Path::new("o.toml")).expect_err("should reject");
        match err {
            CascadeLoadError::InvalidShape { message, .. } => {
                assert!(message.contains("[[rule]]"), "names the offending form");
            }
            other => panic!("unexpected error {other:?}"),
        }
    }

    #[test]
    fn section_header_in_override_is_rejected() {
        let src = "[filament.type.PLA]\nbed_temp = 50\n";
        let err = parse_override_str(src, Path::new("o.toml")).expect_err("should reject");
        match err {
            CascadeLoadError::InvalidShape { message, .. } => {
                assert!(message.contains("section"), "explains the rejection");
            }
            other => panic!("unexpected error {other:?}"),
        }
    }

    #[test]
    fn flat_override_with_scalar_and_array() {
        let src = "\
bed_temp = 50
nozzle_diameter = [\"0.4\", \"0.6\"]
";
        let parsed = parse_override(src, "o.toml");
        assert_eq!(parsed.entries.get("bed_temp").map(String::as_str), Some("50"));
        // Vectors get comma-joined like in the cascade loader.
        assert_eq!(
            parsed.entries.get("nozzle_diameter").map(String::as_str),
            Some("0.4,0.6")
        );
    }
}
