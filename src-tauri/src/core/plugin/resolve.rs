//! Resolve a plugin's effective config — activation **and** settings —
//! across the cascade levels, enforcing the activation-gated settings
//! rule (PR-8-9 "Settings-cascade model").
//!
//! Levels: `global` (binary on/off), then `project`, then `plate` (each
//! tri-state inherit/on/off — `inherit` = the `plugin.<name>.enabled`
//! key absent at that tier). This is the single `plugin.*` resolver the
//! review asked for: activation is just one of its outputs, so the
//! tier-walk isn't duplicated for settings.
//!
//! The two rules:
//! - **Effective activation** (does it run): the first explicit
//!   (non-inherit) value walking **plate → project → global**, default
//!   on.
//! - **Settings promotion:** base = manifest defaults; a level's
//!   settings overlay **only where that level's activation is explicitly
//!   `on`**, in `global → project → plate` order. `inherit` / `off`
//!   levels never promote settings — even when the plugin is effectively
//!   running via inheritance. Enforced here, independent of any UI
//!   gating.

use std::collections::BTreeMap;

/// One cascade level's plugin overrides, extracted from that tier's flat
/// `plugin.*` entries. `activation = None` means "inherit".
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PluginLevel {
    /// `Some(true)` = on, `Some(false)` = off, `None` = inherit.
    pub activation: Option<bool>,
    /// This level's `plugin.<name>.<key>` setting values (key → value).
    pub settings: BTreeMap<String, String>,
}

/// A plugin's resolved config for one slice context.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedPlugin {
    /// Effective activation — whether the plugin runs for this context.
    pub enabled: bool,
    /// Resolved setting values (manifest defaults overlaid by the
    /// promoted levels). Always carries every default key.
    pub settings: BTreeMap<String, String>,
}

/// Pull a single plugin's per-level overrides out of a tier's flat
/// entries. A key `plugin.<name>.enabled` becomes the activation (parsed
/// as a bool); any other `plugin.<name>.<key>` becomes a setting. Plugin
/// names are kebab-case (no dots) and the prefix carries a trailing dot,
/// so a name that's a prefix of another (`foo` vs `foo-bar`) can't
/// cross-match.
pub fn level_for(name: &str, entries: &BTreeMap<String, String>) -> PluginLevel {
    let prefix = format!("plugin.{name}.");
    let mut level = PluginLevel::default();
    for (key, value) in entries {
        let Some(suffix) = key.strip_prefix(&prefix) else {
            continue;
        };
        if suffix == "enabled" {
            level.activation = parse_bool(value);
        } else {
            level.settings.insert(suffix.to_string(), value.clone());
        }
    }
    level
}

/// Parse a flat override value as a bool (`true`/`1`, `false`/`0`),
/// `None` for anything else (an unparseable activation reads as inherit).
pub fn parse_bool(value: &str) -> Option<bool> {
    match value.trim() {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

/// Resolve a plugin's effective config. `global_on`: `None` = the global
/// default (on); `Some(false)` = globally off. `defaults`: the plugin's
/// manifest setting defaults (the always-present base).
pub fn resolve(
    global_on: Option<bool>,
    global_settings: &BTreeMap<String, String>,
    project: &PluginLevel,
    plate: &PluginLevel,
    defaults: &BTreeMap<String, String>,
) -> ResolvedPlugin {
    // Effective activation: first explicit value, finest level first.
    let enabled = plate
        .activation
        .or(project.activation)
        .or(global_on)
        .unwrap_or(true);

    // Settings: defaults, then each *explicitly-on* level overlaid
    // coarse → fine. Global is "on" unless explicitly off.
    let mut settings = defaults.clone();
    if global_on != Some(false) {
        overlay(&mut settings, global_settings);
    }
    if project.activation == Some(true) {
        overlay(&mut settings, &project.settings);
    }
    if plate.activation == Some(true) {
        overlay(&mut settings, &plate.settings);
    }

    ResolvedPlugin { enabled, settings }
}

fn overlay(into: &mut BTreeMap<String, String>, from: &BTreeMap<String, String>) {
    for (k, v) in from {
        into.insert(k.clone(), v.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lvl(activation: Option<bool>, settings: &[(&str, &str)]) -> PluginLevel {
        PluginLevel {
            activation,
            settings: settings
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    fn defaults() -> BTreeMap<String, String> {
        [("swap".to_string(), "DEFAULT".to_string())]
            .into_iter()
            .collect()
    }

    #[test]
    fn level_for_splits_activation_and_settings() {
        let entries: BTreeMap<String, String> = [
            ("plugin.platecycler.enabled", "false"),
            ("plugin.platecycler.swap", "M400"),
            ("plugin.other.enabled", "true"), // different plugin, ignored
            ("bed_temperature", "60"),        // non-plugin key, ignored
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

        let level = level_for("platecycler", &entries);
        assert_eq!(level.activation, Some(false));
        assert_eq!(level.settings.get("swap").map(String::as_str), Some("M400"));
        assert_eq!(level.settings.len(), 1);
    }

    #[test]
    fn level_for_prefix_does_not_cross_match() {
        let entries: BTreeMap<String, String> =
            [("plugin.foo-bar.enabled", "true"), ("plugin.foo-bar.k", "v")]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
        // Plugin "foo" must NOT pick up "foo-bar" keys.
        let foo = level_for("foo", &entries);
        assert_eq!(foo.activation, None);
        assert!(foo.settings.is_empty());
        let foobar = level_for("foo-bar", &entries);
        assert_eq!(foobar.activation, Some(true));
        assert_eq!(foobar.settings.get("k").map(String::as_str), Some("v"));
    }

    #[test]
    fn inherit_runs_but_does_not_promote_its_settings() {
        // The worked example from the ticket: global on (swap=A),
        // project inherit, plate inherit (swap=C). Runs, but resolves to
        // the GLOBAL value — the plate's value is ignored because the
        // plate is inherit, not on.
        let global_settings = [("swap".to_string(), "A".to_string())]
            .into_iter()
            .collect();
        let project = lvl(None, &[]);
        let plate = lvl(None, &[("swap", "C")]);
        let r = resolve(None, &global_settings, &project, &plate, &defaults());
        assert!(r.enabled, "effective on via inheritance");
        assert_eq!(r.settings.get("swap").map(String::as_str), Some("A"));
    }

    #[test]
    fn explicit_on_promotes_and_finest_wins() {
        let global_settings = [("swap".to_string(), "A".to_string())]
            .into_iter()
            .collect();
        let project = lvl(Some(true), &[("swap", "B")]);
        let plate = lvl(Some(true), &[("swap", "C")]);
        let r = resolve(Some(true), &global_settings, &project, &plate, &defaults());
        assert!(r.enabled);
        assert_eq!(r.settings.get("swap").map(String::as_str), Some("C"));
    }

    #[test]
    fn off_at_a_level_drops_that_levels_settings() {
        // Global off (swap=A not promoted), project on (swap=B), plate
        // inherit. Runs via project; resolves to B; global A is dropped.
        let global_settings = [("swap".to_string(), "A".to_string())]
            .into_iter()
            .collect();
        let project = lvl(Some(true), &[("swap", "B")]);
        let plate = lvl(None, &[]);
        let r = resolve(Some(false), &global_settings, &project, &plate, &defaults());
        assert!(r.enabled, "project on overrides global off");
        assert_eq!(r.settings.get("swap").map(String::as_str), Some("B"));
    }

    #[test]
    fn plate_off_deactivates_even_if_lower_levels_on() {
        let project = lvl(Some(true), &[("swap", "B")]);
        let plate = lvl(Some(false), &[]);
        let r = resolve(Some(true), &BTreeMap::new(), &project, &plate, &defaults());
        assert!(!r.enabled, "plate off wins");
    }

    #[test]
    fn settings_floor_is_always_the_manifest_default() {
        // Nothing overridden anywhere → the default survives.
        let r = resolve(
            None,
            &BTreeMap::new(),
            &lvl(None, &[]),
            &lvl(None, &[]),
            &defaults(),
        );
        assert_eq!(r.settings.get("swap").map(String::as_str), Some("DEFAULT"));
    }
}
