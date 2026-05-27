//! Trace tooling: "why is X = 55?" answers.
//!
//! Consumes the `ResolvedOverrides` from `overrides::resolve_with_overrides`
//! and produces a structured `Trace` for a single key — winner, losers
//! at lower specificities, override source, cascade fallback.
//!
//! Drives FR-CAS-7 (the cascade source badge in the Settings UI's
//! Phase 4 panel) and the future debug command. Phase 1 only ships the
//! data API + pretty-printer; UI rendering is Phase 4.

use super::overrides::{OverrideTraceEntry, ResolvedOverrides};
use super::types::SourceLocation;
use serde::Serialize;
use std::fmt;

/// Structured "why is X = Y?" answer.
#[derive(Debug, Clone, Serialize)]
pub struct Trace {
    pub key: String,
    pub effective_value: String,
    pub source: TraceSource,
    pub cascade_winner: Option<TraceRule>,
    pub cascade_losers: Vec<TraceRule>,
    pub override_source: Option<OverrideTraceEntry>,
    pub cascade_fallback: Option<String>,
}

/// Where the effective value came from. `Cascade` = an authored rule
/// won; `Override` = a user/project override beat the cascade.
#[derive(Debug, Clone, Copy, Serialize)]
pub enum TraceSource {
    Cascade,
    Override,
}

/// A single matching rule in a trace, formatted for display.
#[derive(Debug, Clone, Serialize)]
pub struct TraceRule {
    pub source: SourceLocation,
    pub specificity: usize,
    pub when_summary: String,
    pub set_value: String,
}

/// Look up the trace for a single key. Returns `None` when the key
/// is absent from `resolved` (either because no rule set it and no
/// override touched it, or because the key is misspelled).
pub fn trace(resolved: &ResolvedOverrides, key: &str) -> Option<Trace> {
    let entry = resolved.get(key)?;
    let source = if entry.override_source.is_some() {
        TraceSource::Override
    } else {
        TraceSource::Cascade
    };

    let (cascade_winner, cascade_losers) = if entry.matching_rules.is_empty() {
        // Override-only key — no cascade winner to report.
        (None, Vec::new())
    } else {
        let last_idx = entry.matching_rules.len() - 1;
        let mut losers = Vec::with_capacity(last_idx);
        for (i, mr) in entry.matching_rules.iter().enumerate() {
            let rule = TraceRule {
                source: mr.source.clone(),
                specificity: mr.specificity,
                when_summary: if mr.when_summary.is_empty() {
                    "default".to_string()
                } else {
                    mr.when_summary.clone()
                },
                set_value: mr.value.clone(),
            };
            if i == last_idx {
                // Winner — handled below.
            } else {
                losers.push(rule);
            }
        }
        let winner = TraceRule {
            source: entry.matching_rules[last_idx].source.clone(),
            specificity: entry.matching_rules[last_idx].specificity,
            when_summary: if entry.matching_rules[last_idx].when_summary.is_empty() {
                "default".to_string()
            } else {
                entry.matching_rules[last_idx].when_summary.clone()
            },
            set_value: entry.matching_rules[last_idx].value.clone(),
        };
        (Some(winner), losers)
    };

    Some(Trace {
        key: key.to_string(),
        effective_value: entry.value.clone(),
        source,
        cascade_winner,
        cascade_losers,
        override_source: entry.override_source.clone(),
        cascade_fallback: entry.cascade_fallback.clone(),
    })
}

impl fmt::Display for Trace {
    /// CLI pretty-print for the Phase 1 exit-smoke driver and the
    /// eventual cascade debug command.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let src_label = match self.source {
            TraceSource::Cascade => "cascade",
            TraceSource::Override => "override",
        };
        writeln!(f, "{} = {} ({src_label})", self.key, self.effective_value)?;

        if let Some(override_entry) = &self.override_source {
            writeln!(
                f,
                "  override: tier={} at {} → set.{} = {}",
                override_entry.tier,
                override_entry.source,
                self.key,
                override_entry.value,
            )?;
        }

        if let Some(winner) = &self.cascade_winner {
            let label = if matches!(self.source, TraceSource::Override) {
                "cascade_fallback"
            } else {
                "winner"
            };
            writeln!(
                f,
                "  {label:<16}  spec={} {} at {} → set.{} = {}",
                winner.specificity,
                winner.when_summary,
                winner.source,
                self.key,
                winner.set_value,
            )?;
        }

        for loser in &self.cascade_losers {
            writeln!(
                f,
                "  loser:            spec={} {} at {} → set.{} = {}",
                loser.specificity,
                loser.when_summary,
                loser.source,
                self.key,
                loser.set_value,
            )?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::cascade::loader::parse_cascade_str;
    use crate::core::cascade::overrides::{
        parse_override_str, resolve_with_overrides, OverrideTier, OverrideTiers,
    };
    use crate::core::cascade::resolver::MapContext;
    use crate::core::cascade::types::Cascade;
    use std::path::Path;

    fn ctx() -> MapContext {
        MapContext::with([("filament.type", "PLA"), ("plate.type", "PEI")])
    }

    fn cascade(src: &str) -> Cascade {
        Cascade {
            rules: parse_cascade_str(src, Path::new("cascade.toml")).expect("cascade parse"),
        }
    }

    #[test]
    fn three_rule_ladder_winner_and_two_losers() {
        let c = cascade(
            "\
bed_temp = 50

[[rule]]
when.filament.type = \"PLA\"
set.bed_temp = 60

[[rule]]
when.filament.type = \"PLA\"
when.plate.type = \"PEI\"
set.bed_temp = 55
",
        );
        let resolved = resolve_with_overrides(&c, &OverrideTiers::empty(), &ctx());
        let t = trace(&resolved, "bed_temp").expect("traced");
        assert_eq!(t.effective_value, "55");
        assert!(matches!(t.source, TraceSource::Cascade));
        let winner = t.cascade_winner.unwrap();
        assert_eq!(winner.specificity, 2);
        assert_eq!(winner.set_value, "55");
        assert_eq!(t.cascade_losers.len(), 2);
        // Losers are in source-order; spec-0 then spec-1.
        assert_eq!(t.cascade_losers[0].specificity, 0);
        assert_eq!(t.cascade_losers[1].specificity, 1);
    }

    #[test]
    fn override_trace_records_fallback() {
        let c = cascade(
            "\
[[rule]]
when.filament.type = \"PLA\"
when.plate.type = \"PEI\"
set.bed_temp = 55
",
        );
        let project = parse_override_str("bed_temp = 50\n", Path::new("project.toml"))
            .expect("override parse");
        let overrides = OverrideTiers {
            user: vec![],
            project: vec![project],
            object: None,
        };
        let resolved = resolve_with_overrides(&c, &overrides, &ctx());
        let t = trace(&resolved, "bed_temp").expect("traced");
        assert!(matches!(t.source, TraceSource::Override));
        assert_eq!(t.effective_value, "50");
        assert_eq!(t.cascade_fallback.as_deref(), Some("55"));
        let os = t.override_source.unwrap();
        assert_eq!(os.tier, OverrideTier::Project);
    }

    #[test]
    fn override_only_key_has_no_cascade_winner() {
        let c = cascade("layer_height = 0.2\n");
        let project = parse_override_str("bed_temp = 60\n", Path::new("project.toml"))
            .expect("override parse");
        let overrides = OverrideTiers {
            user: vec![],
            project: vec![project],
            object: None,
        };
        let resolved = resolve_with_overrides(&c, &overrides, &ctx());
        let t = trace(&resolved, "bed_temp").expect("traced");
        assert!(matches!(t.source, TraceSource::Override));
        assert!(t.cascade_winner.is_none());
        assert!(t.cascade_losers.is_empty());
        assert!(t.cascade_fallback.is_none());
    }

    #[test]
    fn missing_key_returns_none() {
        let c = cascade("layer_height = 0.2\n");
        let resolved = resolve_with_overrides(&c, &OverrideTiers::empty(), &ctx());
        assert!(trace(&resolved, "bed_temp").is_none());
        assert!(trace(&resolved, "typo_key").is_none());
    }

    #[test]
    fn default_rule_when_summary_renders_as_default() {
        let c = cascade("bed_temp = 50\n");
        let resolved = resolve_with_overrides(&c, &OverrideTiers::empty(), &ctx());
        let t = trace(&resolved, "bed_temp").expect("traced");
        let w = t.cascade_winner.unwrap();
        assert_eq!(w.when_summary, "default");
    }

    #[test]
    fn pretty_print_shape_smoke() {
        let c = cascade(
            "\
bed_temp = 50

[[rule]]
when.filament.type = \"PLA\"
when.plate.type = \"PEI\"
set.bed_temp = 55
",
        );
        let resolved = resolve_with_overrides(&c, &OverrideTiers::empty(), &ctx());
        let t = trace(&resolved, "bed_temp").expect("traced");
        let s = format!("{t}");
        assert!(s.contains("bed_temp = 55"));
        assert!(s.contains("cascade"));
        assert!(s.contains("spec=2"));
        assert!(s.contains("filament.type = \"PLA\""));
        assert!(s.contains("loser:"));
    }
}
