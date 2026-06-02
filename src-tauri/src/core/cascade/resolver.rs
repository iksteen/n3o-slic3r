//! Production cascade resolver.
//!
//! Replaces the throwaway resolver in `src-tauri/examples/spike1.rs`.
//! Implements the *authored-cascade* tier of the two-phase resolution
//! described in `docs/profiles.md` — predicate evaluation, specificity
//! ranking, source-order tie-breaks, within-cascade tie-break warnings.
//!
//! `resolve()` also honors an in-cascade override tier: a rule flagged
//! [`Rule::important`](super::types::Rule::important) wins over every
//! authored rule regardless of specificity. `compose_cascade` uses it for
//! plate/object overrides, which must beat a profile option even when that
//! option is set under a `when` predicate.
//!
//! Separately, the file-based `!important` user + project override tier
//! (loaded from override `.toml`s) lives in `overrides.rs` and consumes
//! this resolver's output. Trace tooling builds the structured "why is
//! X = 55?" report from the matching-rules list this resolver retains.

use super::types::{Cascade, Condition, ConditionValue, Rule, SourceLocation};
use std::collections::BTreeMap;
use std::fmt::Write;

/// What the resolver needs to know about the active slice context.
///
/// `predicate_value(key)` returns the live value for a dotted dimension
/// like `"filament.type"` or `"plate.type"`. The production implementor
/// is [`crate::core::project::SlicingContext`]; tests and lightweight
/// drivers can use [`MapContext`] below.
pub trait Context {
    fn predicate_value(&self, key: &str) -> Option<&str>;
}

/// A `Context` backed by a plain `BTreeMap`. Useful for tests, for
/// `cargo run --example` driving, and for hand-constructing contexts
/// outside the project layer.
#[derive(Debug, Clone, Default)]
pub struct MapContext {
    map: BTreeMap<String, String>,
}

impl MapContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with<I, K, V>(iter: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        Self {
            map: iter
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        }
    }

    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.map.insert(key.into(), value.into());
    }
}

impl Context for MapContext {
    fn predicate_value(&self, key: &str) -> Option<&str> {
        self.map.get(key).map(String::as_str)
    }
}

/// Resolved per-key value with full trace metadata.
///
/// `value` is the winning rule's set entry (the effective output);
/// `winning_*` describes that winner; `matching_rules` lists every
/// rule that matched the context for this key (winner + losers in
/// source order). trace tooling renders the losers list as
/// "specs that lost".
#[derive(Debug, Clone)]
pub struct ResolvedValue {
    pub value: String,
    pub winning_rule: SourceLocation,
    pub winning_specificity: usize,
    pub matching_rules: Vec<MatchingRule>,
}

#[derive(Debug, Clone)]
pub struct MatchingRule {
    pub source: SourceLocation,
    pub specificity: usize,
    pub value: String,
    /// Human-readable summary of the rule's `when` predicate for trace
    /// rendering. Empty string for the unconditional default rule;
    /// otherwise `<dim> = "<value>"` segments joined by ` + `.
    pub when_summary: String,
}

/// Render a rule's `when` predicate as a single human-readable string.
/// E.g. `filament.type = "PLA" + plate.type = "PEI"` for a 2-predicate
/// rule, or `""` (empty) for the unconditional default.
pub fn format_when(conditions: &[Condition]) -> String {
    let mut out = String::new();
    for (i, cond) in conditions.iter().enumerate() {
        if i > 0 {
            out.push_str(" + ");
        }
        match &cond.value {
            ConditionValue::Scalar(s) => {
                let _ = write!(out, "{} = \"{}\"", cond.dimension, s);
            }
            ConditionValue::Array(items) => {
                let quoted: Vec<String> = items.iter().map(|s| format!("\"{s}\"")).collect();
                let _ = write!(out, "{} in [{}]", cond.dimension, quoted.join(", "));
            }
        }
    }
    out
}

/// Flat map from libslic3r option key (dotted form) to its resolved
/// value plus trace.
pub type Resolved = BTreeMap<String, ResolvedValue>;

/// Resolve the cascade against `ctx`. Returns the flat per-key
/// `Resolved` map.
///
/// Application order: non-`important` rules first, then `important`
/// (override-tier) rules — so an override beats authored rules of **any**
/// specificity. Within each tier: lowest specificity first; within the
/// same specificity, source order (later wins). When two *non-override*
/// rules at the same specificity *and* from different cascade files both
/// set the same key, emits a `tracing::warn!` — the later rule still wins,
/// but the author probably didn't intend the dependency on load order. An
/// override overwriting an authored rule is intentional, so it never warns.
pub fn resolve(cascade: &Cascade, ctx: &dyn Context) -> Resolved {
    // Step 1: identify the rules that match, with their source index.
    let mut matching: Vec<(usize, &Rule)> = cascade
        .rules
        .iter()
        .enumerate()
        .filter(|(_, r)| rule_matches(r, ctx))
        .collect();

    // Sort by (important, specificity, source-index) — all ascending — so
    // override-tier rules apply last (winning over any specificity), and
    // within a tier later same-specificity rules overwrite earlier ones.
    matching.sort_by_key(|(idx, r)| (r.important, r.specificity(), *idx));

    let mut resolved: Resolved = BTreeMap::new();
    for (_, rule) in &matching {
        let when_summary = format_when(&rule.when.conditions);
        for (key, value) in &rule.set {
            let new_entry = MatchingRule {
                source: rule.source.clone(),
                specificity: rule.specificity(),
                value: value.clone(),
                when_summary: when_summary.clone(),
            };
            match resolved.get_mut(key) {
                Some(prior) => {
                    // Tie-break warning: same specificity, different file —
                    // but only among authored rules. An override (`important`)
                    // overwriting an authored value is the whole point of the
                    // override tier, so it's never a "tie" to warn about.
                    if !rule.important
                        && prior.winning_specificity == rule.specificity()
                        && prior.winning_rule.path != rule.source.path
                    {
                        tracing::warn!(
                            key = %key,
                            prior_source = %prior.winning_rule,
                            new_source = %rule.source,
                            specificity = rule.specificity(),
                            "cascade tie: two same-specificity rules from different files set this key — \
                             later (this) rule wins by source order"
                        );
                    }
                    prior.value = value.clone();
                    prior.winning_rule = rule.source.clone();
                    prior.winning_specificity = rule.specificity();
                    prior.matching_rules.push(new_entry);
                }
                None => {
                    resolved.insert(
                        key.clone(),
                        ResolvedValue {
                            value: value.clone(),
                            winning_rule: rule.source.clone(),
                            winning_specificity: rule.specificity(),
                            matching_rules: vec![new_entry],
                        },
                    );
                }
            }
        }
    }

    resolved
}

/// True iff every condition in the rule's predicate is satisfied by
/// the context. A rule with zero predicates (the default) always
/// matches.
pub fn rule_matches(rule: &Rule, ctx: &dyn Context) -> bool {
    rule.when.conditions.iter().all(|cond| {
        let Some(ctx_value) = ctx.predicate_value(&cond.dimension) else {
            return false;
        };
        match &cond.value {
            ConditionValue::Scalar(s) => ctx_value == s.as_str(),
            ConditionValue::Array(items) => items.iter().any(|i| i == ctx_value),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::cascade::loader::parse_cascade_str;
    use std::path::Path;

    fn parse(src: &str) -> Cascade {
        Cascade {
            rules: parse_cascade_str(src, Path::new("test.toml")).expect("parse"),
        }
    }

    fn parse_named(src: &str, path: &str) -> Cascade {
        Cascade {
            rules: parse_cascade_str(src, Path::new(path)).expect("parse"),
        }
    }

    fn pla_pei_ctx() -> MapContext {
        MapContext::with([("filament.type", "PLA"), ("plate.type", "PEI")])
    }

    #[test]
    fn default_rule_only_resolves() {
        let cascade = parse("bed_temp = 50\n");
        let resolved = resolve(&cascade, &pla_pei_ctx());
        let v = resolved.get("bed_temp").expect("bed_temp resolved");
        assert_eq!(v.value, "50");
        assert_eq!(v.winning_specificity, 0);
        assert_eq!(v.matching_rules.len(), 1);
    }

    #[test]
    fn higher_specificity_wins() {
        let src = "\
bed_temp = 50

[[rule]]
when.filament.type = \"PLA\"
set.bed_temp = 45

[[rule]]
when.filament.type = \"PLA\"
when.plate.type = \"PEI\"
set.bed_temp = 55
";
        let cascade = parse(src);
        let resolved = resolve(&cascade, &pla_pei_ctx());
        let v = resolved.get("bed_temp").expect("bed_temp resolved");
        assert_eq!(v.value, "55", "highest specificity wins");
        assert_eq!(v.winning_specificity, 2);
        assert_eq!(v.matching_rules.len(), 3, "three matched: 0, 1, 2");
    }

    #[test]
    fn unmatched_rules_dont_set_keys() {
        let src = "[[rule]]\nwhen.filament.type = \"PETG\"\nset.bed_temp = 70\n";
        let cascade = parse(src);
        let resolved = resolve(&cascade, &pla_pei_ctx());
        assert!(
            !resolved.contains_key("bed_temp"),
            "PETG rule doesn't fire for PLA context"
        );
    }

    #[test]
    fn array_predicate_matches_any_member() {
        let src = "[[rule]]\nwhen.filament.type = [\"PLA\", \"PETG\"]\nset.bed_temp = 55\n";
        let cascade = parse(src);
        let resolved = resolve(&cascade, &pla_pei_ctx());
        assert_eq!(
            resolved.get("bed_temp").map(|v| v.value.as_str()),
            Some("55")
        );
    }

    #[test]
    fn source_order_breaks_specificity_ties() {
        let src = "\
[[rule]]
when.filament.type = \"PLA\"
set.bed_temp = 45

[[rule]]
when.filament.type = \"PLA\"
set.bed_temp = 47
";
        let cascade = parse(src);
        let resolved = resolve(&cascade, &pla_pei_ctx());
        let v = resolved.get("bed_temp").expect("bed_temp resolved");
        assert_eq!(v.value, "47", "later same-specificity rule wins");
        assert_eq!(v.matching_rules.len(), 2);
    }

    #[test]
    fn matching_rules_preserve_source_order() {
        // Three rules all matching the PLA/PEI context, applied in
        // increasing specificity order.
        let src = "\
bed_temp = 50

[[rule]]
when.filament.type = \"PLA\"
set.bed_temp = 45

[[rule]]
when.filament.type = \"PLA\"
when.plate.type = \"PEI\"
set.bed_temp = 55
";
        let cascade = parse(src);
        let resolved = resolve(&cascade, &pla_pei_ctx());
        let v = resolved.get("bed_temp").unwrap();
        // matching_rules[0] = default (spec 0), [1] = filament rule
        // (spec 1), [2] = filament+plate (spec 2). Spec ascending.
        let specs: Vec<usize> = v.matching_rules.iter().map(|m| m.specificity).collect();
        assert_eq!(specs, vec![0, 1, 2]);
    }

    #[test]
    fn cross_file_tie_emits_warning() {
        // Two cascades, same specificity-1 rule, different files. The
        // resolver should still pick the second one but emit a
        // tracing::warn. We use tracing-subscriber's test capture to
        // observe the warning.
        use tracing::subscriber::with_default;
        use tracing_subscriber::fmt::format::FmtSpan;

        let a = parse_named(
            "[[rule]]\nwhen.filament.type = \"PLA\"\nset.bed_temp = 45\n",
            "fileA.toml",
        );
        let b = parse_named(
            "[[rule]]\nwhen.filament.type = \"PLA\"\nset.bed_temp = 47\n",
            "fileB.toml",
        );
        // Merge by appending b's rules onto a's rules
        let cascade = Cascade {
            rules: a.rules.into_iter().chain(b.rules).collect(),
        };

        // Capture tracing output
        let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
        let writer = TestWriter {
            buf: captured.clone(),
        };
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_span_events(FmtSpan::NONE)
            .with_max_level(tracing::Level::WARN)
            .finish();

        with_default(subscriber, || {
            let _ = resolve(&cascade, &pla_pei_ctx());
        });

        let log = String::from_utf8(captured.lock().unwrap().clone()).unwrap();
        assert!(
            log.contains("cascade tie"),
            "expected cascade-tie warning in log: {log}"
        );
        assert!(log.contains("fileA.toml"));
        assert!(log.contains("fileB.toml"));
    }

    #[test]
    fn important_override_beats_higher_specificity_and_does_not_warn() {
        use tracing::subscriber::with_default;

        // Authored cascade: a specificity-2 rule sets bed_temp = 55. Without
        // the override tier this would win over an unconditional override.
        let authored = parse(
            "\
[[rule]]
when.filament.type = \"PLA\"
when.plate.type = \"PEI\"
set.bed_temp = 55
",
        );
        // Override tier: an unconditional (specificity 0) `important` rule —
        // the shape `compose_cascade` appends for plate/object overrides.
        let override_rule = Rule {
            when: crate::core::cascade::types::Predicate::default(),
            set: BTreeMap::from([("bed_temp".to_string(), "99".to_string())]),
            source: SourceLocation {
                path: "<plate-overrides>".into(),
                line: 1,
            },
            important: true,
        };
        let cascade = Cascade {
            rules: authored.rules.into_iter().chain([override_rule]).collect(),
        };

        let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
        let writer = TestWriter {
            buf: captured.clone(),
        };
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .with_max_level(tracing::Level::WARN)
            .finish();
        let resolved = with_default(subscriber, || resolve(&cascade, &pla_pei_ctx()));

        let v = resolved.get("bed_temp").expect("bed_temp resolved");
        assert_eq!(
            v.value, "99",
            "the override wins over the specificity-2 rule"
        );
        let log = String::from_utf8(captured.lock().unwrap().clone()).unwrap();
        assert!(
            !log.contains("cascade tie"),
            "an override overwriting an authored rule must not warn: {log}"
        );
    }

    /// Test-only `Write` impl that fans every line into a shared
    /// buffer. Used to capture tracing output without pulling in a
    /// heavier `tracing-test` dep.
    #[derive(Clone)]
    struct TestWriter {
        buf: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    }
    impl std::io::Write for TestWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.buf.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
}
