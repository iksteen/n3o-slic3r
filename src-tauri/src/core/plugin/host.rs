//! The multi-plugin host: load discovered plugins into sandboxed
//! runtimes and dispatch hooks across them with strict error
//! isolation.
//!
//! Loading semantics: a plugin whose manifest parsed but whose Lua
//! fails to load (or whose entry can't be read) is **kept** in the
//! host in a disabled/errored state rather than dropped, so it shows
//! up in the Plugins panel with its error.
//!
//! Dispatch semantics: a hook is folded across every enabled plugin
//! that declares it, in lexical name order. A plugin that errors
//! (Lua error / timeout / bad return) is caught, recorded in its
//! `last_error`, and **auto-disabled for the session**; the fold
//! continues with the value as it stood before that plugin (its
//! transform is skipped, never applied half-way). A plugin failure is
//! never propagated to the pipeline — the slice/send proceeds as if
//! the plugin weren't there.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::bindings::build_settings_table;
use super::discovery::{discover, DiscoveredPlugin, MANIFEST_FILE};
use super::error::PluginError;
use super::manifest::{parse_manifest, HookKind, PluginManifest, PrinterCompat};
use super::resolve::{self, ResolvedPlugin};
use super::runtime::PluginRuntime;

/// One plugin as the host holds it.
pub struct LoadedPlugin {
    /// Directory the plugin lives in (used to reload from disk).
    dir: PathBuf,
    manifest: PluginManifest,
    /// `None` when the entry Lua failed to load — the plugin is kept
    /// (disabled) so its error is visible.
    runtime: Option<PluginRuntime>,
    enabled: bool,
    last_error: Option<String>,
}

/// A pipeline hook the host folds across plugins. Implementors own the
/// Lua marshalling for their payload; the host owns ordering, the
/// fold, and error isolation. (Real payloads — typed G-code, settings,
/// the send buffer — implement this in later tickets.)
pub trait Hook {
    /// The value threaded through the plugin chain.
    type Value;

    /// Manifest hook this corresponds to — selects which plugins run.
    fn kind(&self) -> HookKind;

    /// Run one plugin's hook with the current value, returning the
    /// (possibly transformed) value plus an optional error. On error,
    /// implementors **return the input value unchanged** so the host
    /// can continue the chain with it; a plugin that declares the hook
    /// but defines no function is a no-op (also returns the input).
    ///
    /// `&self` so the hook can carry per-dispatch context (e.g. the
    /// plate metadata a post-slice hook hands to each plugin).
    fn invoke(
        &self,
        runtime: &PluginRuntime,
        value: Self::Value,
    ) -> (Self::Value, Option<PluginError>);
}

/// One declared plugin setting, surfaced to the Plugins UI so it can
/// render the right control and show the default.
#[derive(Debug, Serialize)]
pub struct SettingSummary {
    pub key: String,
    /// `"string"` / `"number"` / `"bool"` / `"enum"`.
    pub kind: String,
    pub label: Option<String>,
    /// Manifest default, as the flat string the cascade uses.
    pub default: String,
    /// Allowed values for an `enum`; empty otherwise.
    pub values: Vec<String>,
}

/// Serializable view of a plugin for the `plugin_list` command.
#[derive(Debug, Serialize)]
pub struct PluginSummary {
    pub name: String,
    pub version: String,
    pub hooks: Vec<String>,
    /// `None` means "any printer"; `Some(list)` restricts to models.
    pub printers: Option<Vec<String>>,
    /// Cascade levels this plugin may be enabled/configured at
    /// (`"global"` / `"project"` / `"plate"`), canonical order. The
    /// panel offers a control only at these scopes.
    pub scopes: Vec<String>,
    /// **Health**: loaded and not auto-disabled by a runtime error. A
    /// `false` here with a `last_error` is the errored state; reload to
    /// recover. Distinct from `globally_enabled`.
    pub enabled: bool,
    /// **Global activation**: the user's persisted on/off for this
    /// plugin (the panel toggle), defaulting to `enabled_by_default`. A
    /// healthy plugin with `globally_enabled = false` simply doesn't run.
    pub globally_enabled: bool,
    /// The manifest's opt-in default (`enabled_by_default`) — the floor
    /// the UI uses for a plugin's root activation when nothing is set.
    pub enabled_by_default: bool,
    /// Declared settings (for the Plugins UI to render controls).
    pub settings: Vec<SettingSummary>,
    /// Current global-tier setting values (key → value), so the UI shows
    /// what's set at the global level. Absent keys use the declared
    /// default.
    pub global_settings: BTreeMap<String, String>,
    pub last_error: Option<String>,
}

/// Per-dispatch gate deciding which plugins may run for *this* plate or
/// send. Distinct from a plugin's **health** (`LoadedPlugin.enabled` /
/// `last_error`, which the host manages): the gate is the user-intent
/// **activation** axis plus printer applicability, supplied per dispatch
/// by the caller (the orchestrator resolves it per plate from the
/// cascade).
#[derive(Debug, Default)]
pub struct DispatchGate {
    /// Active printer model, for `printer_compatibility` enforcement.
    /// `None` skips the printer check (a context with no bound model).
    pub printer_model: Option<String>,
    /// Flat `plugin.*` override entries for the **project** level
    /// (cascade *user* tier). The host resolves each plugin's per-level
    /// activation + settings from these and `plate` (see
    /// [`super::resolve`]). Empty = no project-level overrides.
    pub project: BTreeMap<String, String>,
    /// Flat `plugin.*` override entries for the **plate** level (cascade
    /// *project* tier).
    pub plate: BTreeMap<String, String>,
}

impl DispatchGate {
    /// A permissive gate: no printer filter, no per-level overrides
    /// (every plugin resolves to its global/default activation). The
    /// plain `dispatch` / `any_hook` use this.
    pub fn all() -> Self {
        Self::default()
    }
}

/// Whether `model` satisfies a plugin's declared printer compatibility.
fn printer_matches(compat: &PrinterCompat, model: &str) -> bool {
    match compat {
        PrinterCompat::Any => true,
        PrinterCompat::Models(list) => list.iter().any(|m| m == model),
    }
}

/// Owns every discovered plugin and dispatches hooks across them.
pub struct PluginHost {
    /// Name-sorted; later-discovered roots override earlier ones on a
    /// name clash (user plugins shadow bundled ones).
    plugins: Vec<LoadedPlugin>,
    /// Global per-plugin activation, keyed by name — the lowest
    /// activation tier (under per-project / per-plate overrides),
    /// persisted in `config.toml`. A plugin absent here uses its
    /// manifest default (enabled). This is **activation**, separate from
    /// a plugin's **health** (`LoadedPlugin.enabled`).
    global_enabled: BTreeMap<String, bool>,
    /// Global per-plugin **setting** values (the global tier of the
    /// settings cascade), keyed by plugin name then setting key, in the
    /// flat string vocabulary. From `config.toml`'s
    /// `[plugins.settings.<name>]`.
    global_settings: BTreeMap<String, BTreeMap<String, String>>,
}

impl PluginHost {
    /// Empty host (no plugins). Useful as a default before roots are
    /// resolved, and in tests.
    pub fn empty() -> Self {
        Self {
            plugins: Vec::new(),
            global_enabled: BTreeMap::new(),
            global_settings: BTreeMap::new(),
        }
    }

    /// Discover + load every plugin under each root, in order. A name
    /// declared in more than one root resolves to the last root's copy
    /// (so the user dir overrides bundled). Within a single root,
    /// duplicate names were already flagged as errors by `discover`.
    pub fn load(roots: &[PathBuf]) -> Self {
        let mut discovered: Vec<DiscoveredPlugin> = Vec::new();
        for root in roots {
            discovered.extend(discover(root));
        }
        Self::from_discovered(discovered)
    }

    fn from_discovered(discovered: Vec<DiscoveredPlugin>) -> Self {
        // Keyed by name: a BTreeMap gives last-wins-on-insert (root
        // order → user overrides bundled) and name-sorted iteration
        // (deterministic dispatch order) in one structure.
        let mut by_name: BTreeMap<String, LoadedPlugin> = BTreeMap::new();
        for entry in discovered {
            let manifest = match entry.manifest {
                Ok(m) => m,
                Err(err) => {
                    tracing::warn!(
                        dir = %entry.dir.display(),
                        error = %err,
                        "skipping plugin with invalid manifest",
                    );
                    continue;
                }
            };
            let loaded = build_loaded(&entry.dir, manifest);
            by_name.insert(loaded.manifest.name.clone(), loaded);
        }
        Self {
            plugins: by_name.into_values().collect(),
            global_enabled: BTreeMap::new(),
            global_settings: BTreeMap::new(),
        }
    }

    /// Fold `value` through every enabled plugin that declares the hook,
    /// in name order, with a permissive gate (no printer filter, all
    /// default-activated). Equivalent to the pre-gating behaviour.
    pub fn dispatch<H: Hook>(&mut self, hook: &H, value: H::Value) -> H::Value {
        self.dispatch_gated(hook, value, &DispatchGate::all())
    }

    /// Fold `value` through every plugin that declares the hook **and**
    /// passes `gate` (healthy + printer-compatible + activated), in name
    /// order. Plugin failures are isolated (recorded + the plugin
    /// auto-disabled) and never surface to the caller.
    pub fn dispatch_gated<H: Hook>(
        &mut self,
        hook: &H,
        value: H::Value,
        gate: &DispatchGate,
    ) -> H::Value {
        let kind = hook.kind();
        let mut current = value;
        for i in 0..self.plugins.len() {
            // Resolve this plugin's config (activation + settings) and
            // decide whether it runs this hook.
            let resolved = {
                let p = &self.plugins[i];
                if !p.manifest.hooks.contains(&kind) {
                    continue;
                }
                match self.plugin_runs(p, gate) {
                    Some(r) => r,
                    None => continue,
                }
            };
            // Borrow the runtime only for settings-install + invoke, then
            // release it so the error path can mutate the plugin's state.
            let (next, err) = {
                let p = &self.plugins[i];
                let runtime = p.runtime.as_ref().expect("plugin_runs checked runtime");
                // Install the plugin's typed, read-only `settings` global
                // before the hook runs. A build failure isn't the
                // plugin's fault — log and run with the prior settings.
                if let Err(e) = runtime.install_settings(|lua| {
                    build_settings_table(lua, &p.manifest.settings, &resolved.settings)
                }) {
                    tracing::warn!(
                        plugin = %p.manifest.name,
                        error = %e,
                        "failed to install plugin settings",
                    );
                }
                hook.invoke(runtime, current)
            };
            current = next;
            if let Some(e) = err {
                let plugin = &mut self.plugins[i];
                tracing::warn!(
                    plugin = %plugin.manifest.name,
                    error = %e,
                    "plugin hook failed; disabling for this session",
                );
                plugin.enabled = false;
                plugin.last_error = Some(e.to_string());
            }
        }
        current
    }

    /// Resolve `p`'s effective config (activation + settings) under
    /// `gate` via [`super::resolve`]: per-level activation (global tier
    /// from `global_enabled`, project/plate from the gate) and the
    /// activation-gated settings overlay over the manifest defaults.
    fn resolve_plugin(&self, p: &LoadedPlugin, gate: &DispatchGate) -> ResolvedPlugin {
        let name = &p.manifest.name;
        let project = resolve::level_for(name, &gate.project);
        let plate = resolve::level_for(name, &gate.plate);
        let global_on = self.global_enabled.get(name).copied();
        let empty = BTreeMap::new();
        let global_settings = self.global_settings.get(name).unwrap_or(&empty);
        resolve::resolve(
            p.manifest.enabled_by_default,
            global_on,
            global_settings,
            &project,
            &plate,
            &p.manifest.setting_defaults(),
        )
    }

    /// `Some(resolved)` when `p` should run under `gate` — **healthy**
    /// (loaded, not auto-disabled), **printer-compatible**, and
    /// **effectively activated** — else `None`. Hook membership is
    /// checked by the caller.
    fn plugin_runs(&self, p: &LoadedPlugin, gate: &DispatchGate) -> Option<ResolvedPlugin> {
        if !p.enabled || p.runtime.is_none() {
            return None;
        }
        if let Some(model) = &gate.printer_model {
            if !printer_matches(&p.manifest.printer_compatibility, model) {
                return None;
            }
        }
        let resolved = self.resolve_plugin(p, gate);
        if resolved.enabled {
            Some(resolved)
        } else {
            None
        }
    }

    /// Whether any enabled, loaded plugin declares `kind` (permissive
    /// gate). Lets a caller skip building a hook payload entirely when
    /// nothing would run (e.g. the orchestrator avoids re-parsing G-code).
    pub fn any_hook(&self, kind: HookKind) -> bool {
        self.any_active_hook(kind, &DispatchGate::all())
    }

    /// As [`Self::any_hook`], but only counts plugins that pass `gate`
    /// (healthy + printer-compatible + activated) — so the orchestrator
    /// skips the G-code round-trip when nothing would run for this plate.
    pub fn any_active_hook(&self, kind: HookKind, gate: &DispatchGate) -> bool {
        self.plugins
            .iter()
            .any(|p| p.manifest.hooks.contains(&kind) && self.plugin_runs(p, gate).is_some())
    }

    /// Set one plugin's **global** activation (the persisted lowest
    /// tier). In-memory only — the command layer persists to
    /// `config.toml`. Affects every subsequent dispatch (slice + send).
    pub fn set_global_enabled(&mut self, name: &str, enabled: bool) {
        self.global_enabled.insert(name.to_string(), enabled);
    }

    /// Replace the whole global tier (startup, or after a config edit):
    /// per-plugin enable/disable and global setting values, both from
    /// `config.toml`.
    pub fn apply_global(
        &mut self,
        enabled: BTreeMap<String, bool>,
        settings: BTreeMap<String, BTreeMap<String, String>>,
    ) {
        self.global_enabled = enabled;
        self.global_settings = settings;
    }

    /// Summaries for the Plugins panel.
    pub fn list(&self) -> Vec<PluginSummary> {
        self.plugins
            .iter()
            .map(|p| PluginSummary {
                name: p.manifest.name.clone(),
                version: p.manifest.version.clone(),
                hooks: p.manifest.hooks.iter().map(|h| h.as_str().to_string()).collect(),
                printers: match &p.manifest.printer_compatibility {
                    PrinterCompat::Any => None,
                    PrinterCompat::Models(m) => Some(m.clone()),
                },
                scopes: p.manifest.scopes.iter().map(|s| s.as_str().to_string()).collect(),
                enabled: p.enabled,
                globally_enabled: self
                    .global_enabled
                    .get(&p.manifest.name)
                    .copied()
                    .unwrap_or(p.manifest.enabled_by_default),
                enabled_by_default: p.manifest.enabled_by_default,
                settings: p
                    .manifest
                    .settings
                    .iter()
                    .map(|(key, decl)| SettingSummary {
                        key: key.clone(),
                        kind: decl.kind.as_str().to_string(),
                        label: decl.label.clone(),
                        default: decl.default.as_override_string(),
                        values: decl.values.clone(),
                    })
                    .collect(),
                global_settings: self
                    .global_settings
                    .get(&p.manifest.name)
                    .cloned()
                    .unwrap_or_default(),
                last_error: p.last_error.clone(),
            })
            .collect()
    }

    /// Enable/disable a plugin by name. Enabling a plugin that failed
    /// to load is rejected (reload it first).
    pub fn set_enabled(&mut self, name: &str, enabled: bool) -> Result<(), PluginError> {
        let plugin = self.find_mut(name)?;
        if enabled && plugin.runtime.is_none() {
            return Err(PluginError::Runtime(format!(
                "plugin `{name}` failed to load and cannot be enabled (reload it): {}",
                plugin.last_error.as_deref().unwrap_or("unknown error"),
            )));
        }
        // Re-enabling a plugin that auto-disabled on a runtime error is
        // a fresh chance — clear the stale error so the panel doesn't
        // show an enabled plugin permanently flagged with a past failure.
        if enabled {
            plugin.last_error = None;
        }
        plugin.enabled = enabled;
        Ok(())
    }

    /// Re-read a plugin's manifest + entry from disk and swap in a
    /// fresh runtime. A reload that fails leaves the slot in an
    /// errored state (so the panel reflects it); only an unknown name
    /// is an error.
    pub fn reload(&mut self, name: &str) -> Result<(), PluginError> {
        let idx = self
            .plugins
            .iter()
            .position(|p| p.manifest.name == name)
            .ok_or_else(|| PluginError::Runtime(format!("no plugin named `{name}`")))?;
        let dir = self.plugins[idx].dir.clone();
        let manifest_path = dir.join(MANIFEST_FILE);
        let replacement = match std::fs::read_to_string(&manifest_path) {
            Ok(src) => match parse_manifest(&src, &dir) {
                Ok(manifest) => build_loaded(&dir, manifest),
                Err(err) => errored_in_place(&self.plugins[idx], err.to_string()),
            },
            Err(err) => errored_in_place(&self.plugins[idx], format!("read manifest: {err}")),
        };
        self.plugins[idx] = replacement;
        Ok(())
    }

    fn find_mut(&mut self, name: &str) -> Result<&mut LoadedPlugin, PluginError> {
        self.plugins
            .iter_mut()
            .find(|p| p.manifest.name == name)
            .ok_or_else(|| PluginError::Runtime(format!("no plugin named `{name}`")))
    }
}

/// Read a plugin's entry Lua and build its runtime. A read/load
/// failure yields a kept-but-disabled plugin carrying the error.
fn build_loaded(dir: &Path, manifest: PluginManifest) -> LoadedPlugin {
    let entry_path = dir.join(&manifest.entry);
    match std::fs::read_to_string(&entry_path) {
        Ok(source) => match PluginRuntime::load(&source, &manifest.name) {
            Ok(runtime) => LoadedPlugin {
                dir: dir.to_path_buf(),
                manifest,
                runtime: Some(runtime),
                enabled: true,
                last_error: None,
            },
            Err(e) => {
                let msg = e.to_string();
                LoadedPlugin {
                    dir: dir.to_path_buf(),
                    manifest,
                    runtime: None,
                    enabled: false,
                    last_error: Some(msg),
                }
            }
        },
        Err(e) => {
            let msg = format!("read entry `{}`: {e}", manifest.entry);
            LoadedPlugin {
                dir: dir.to_path_buf(),
                manifest,
                runtime: None,
                enabled: false,
                last_error: Some(msg),
            }
        }
    }
}

/// Build an errored placeholder that keeps the prior plugin's identity
/// (dir + manifest) when a reload can't even parse the new manifest.
fn errored_in_place(prior: &LoadedPlugin, error: String) -> LoadedPlugin {
    LoadedPlugin {
        dir: prior.dir.clone(),
        manifest: prior.manifest.clone(),
        runtime: None,
        enabled: false,
        last_error: Some(error),
    }
}

/// User plugins directory: `$XDG_DATA_HOME` (or `~/.local/share`) +
/// `/n3o-slic3r/plugins`. Same data-root convention as autosaves.
pub fn user_plugins_dir() -> PathBuf {
    crate::core::paths::data_dir("plugins")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stub hook over a `String` so we can exercise the fold +
    /// isolation without the real (later) payloads.
    struct StubHook;
    impl Hook for StubHook {
        type Value = String;
        fn kind(&self) -> HookKind {
            HookKind::PostSlice
        }
        fn invoke(
            &self,
            runtime: &PluginRuntime,
            value: String,
        ) -> (String, Option<PluginError>) {
            match runtime.call::<_, String>("on_post_slice", value.clone()) {
                Ok(Some(next)) => (next, None),
                Ok(None) => (value, None),
                Err(e) => (value, Some(e)),
            }
        }
    }

    fn write_plugin(root: &Path, name: &str, hooks: &str, lua: &str) {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(MANIFEST_FILE),
            format!(
                "name=\"{name}\"\nversion=\"1.0.0\"\nentry=\"main.lua\"\n\
                 hooks={hooks}\nenabled_by_default=true\n"
            ),
        )
        .unwrap();
        std::fs::write(dir.join("main.lua"), lua).unwrap();
    }

    #[test]
    fn dispatch_folds_in_name_order() {
        let tmp = tempfile::tempdir().unwrap();
        write_plugin(
            tmp.path(),
            "alpha",
            r#"["post_slice"]"#,
            r#"function on_post_slice(s) return s .. "-alpha" end"#,
        );
        write_plugin(
            tmp.path(),
            "bravo",
            r#"["post_slice"]"#,
            r#"function on_post_slice(s) return s .. "-bravo" end"#,
        );
        // Declares a different hook → must be skipped by post-slice.
        write_plugin(
            tmp.path(),
            "zulu",
            r#"["pre_slice"]"#,
            r#"function on_pre_slice(s) return s .. "-zulu" end"#,
        );

        let mut host = PluginHost::load(&[tmp.path().to_path_buf()]);
        let out = host.dispatch(&StubHook, "start".to_string());
        assert_eq!(out, "start-alpha-bravo");
    }

    #[test]
    fn erroring_plugin_is_isolated_and_disabled() {
        let tmp = tempfile::tempdir().unwrap();
        write_plugin(
            tmp.path(),
            "good",
            r#"["post_slice"]"#,
            r#"function on_post_slice(s) return s .. "-good" end"#,
        );
        write_plugin(
            tmp.path(),
            "kaboom",
            r#"["post_slice"]"#,
            r#"function on_post_slice(s) error("boom") end"#,
        );

        let mut host = PluginHost::load(&[tmp.path().to_path_buf()]);
        let out = host.dispatch(&StubHook, "start".to_string());
        // The bad plugin's transform is skipped, the good one's applies.
        assert_eq!(out, "start-good");

        // It was recorded + disabled, and a second dispatch skips it.
        let summary: Vec<_> = host.list();
        let bad = summary.iter().find(|p| p.name == "kaboom").unwrap();
        assert!(!bad.enabled);
        assert!(bad.last_error.is_some());

        let out2 = host.dispatch(&StubHook, "again".to_string());
        assert_eq!(out2, "again-good");
    }

    #[test]
    fn reenable_after_error_clears_last_error() {
        let tmp = tempfile::tempdir().unwrap();
        write_plugin(
            tmp.path(),
            "kaboom",
            r#"["post_slice"]"#,
            r#"function on_post_slice(s) error("boom") end"#,
        );
        let mut host = PluginHost::load(&[tmp.path().to_path_buf()]);
        let _ = host.dispatch(&StubHook, "x".to_string());
        assert!(host.list()[0].last_error.is_some());

        host.set_enabled("kaboom", true).unwrap();
        let summary = &host.list()[0];
        assert!(summary.enabled);
        assert!(
            summary.last_error.is_none(),
            "re-enabling should clear the stale error"
        );
    }

    #[test]
    fn runaway_plugin_is_isolated() {
        let tmp = tempfile::tempdir().unwrap();
        write_plugin(
            tmp.path(),
            "spinner",
            r#"["post_slice"]"#,
            r#"function on_post_slice(s) while true do end end"#,
        );
        let mut host = PluginHost::load(&[tmp.path().to_path_buf()]);
        let out = host.dispatch(&StubHook, "x".to_string());
        assert_eq!(out, "x"); // unchanged — runaway aborted + skipped
        assert!(!host.list()[0].enabled);
    }

    #[test]
    fn set_enabled_toggles_dispatch() {
        let tmp = tempfile::tempdir().unwrap();
        write_plugin(
            tmp.path(),
            "p",
            r#"["post_slice"]"#,
            r#"function on_post_slice(s) return s .. "-p" end"#,
        );
        let mut host = PluginHost::load(&[tmp.path().to_path_buf()]);
        host.set_enabled("p", false).unwrap();
        assert_eq!(host.dispatch(&StubHook, "a".into()), "a");
        host.set_enabled("p", true).unwrap();
        assert_eq!(host.dispatch(&StubHook, "a".into()), "a-p");
    }

    #[test]
    fn load_failure_keeps_plugin_disabled_with_error() {
        let tmp = tempfile::tempdir().unwrap();
        // Valid manifest, but the Lua has a syntax error → load fails.
        write_plugin(
            tmp.path(),
            "broken",
            r#"["post_slice"]"#,
            "function on_post_slice(s) return s ..",
        );
        let host = PluginHost::load(&[tmp.path().to_path_buf()]);
        let summary = &host.list()[0];
        assert_eq!(summary.name, "broken");
        assert!(!summary.enabled);
        assert!(summary.last_error.is_some());
    }

    #[test]
    fn gate_skips_printer_incompatible_plugin() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("a1-only");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(MANIFEST_FILE),
            "name=\"a1-only\"\nversion=\"1.0.0\"\nentry=\"main.lua\"\nenabled_by_default=true\n\
             hooks=[\"post_slice\"]\nprinter_compatibility=[\"Bambu Lab A1 mini\"]\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("main.lua"),
            r#"function on_post_slice(s) return s .. "-a1" end"#,
        )
        .unwrap();
        let mut host = PluginHost::load(&[tmp.path().to_path_buf()]);

        // Wrong printer → skipped (and any_active_hook reports nothing).
        let wrong = DispatchGate {
            printer_model: Some("Snapmaker U1".into()),
            ..Default::default()
        };
        assert_eq!(host.dispatch_gated(&StubHook, "x".into(), &wrong), "x");
        assert!(!host.any_active_hook(HookKind::PostSlice, &wrong));

        // Right printer → runs.
        let right = DispatchGate {
            printer_model: Some("Bambu Lab A1 mini".into()),
            ..Default::default()
        };
        assert_eq!(host.dispatch_gated(&StubHook, "x".into(), &right), "x-a1");

        // No model supplied → printer check skipped, runs.
        assert_eq!(
            host.dispatch_gated(&StubHook, "x".into(), &DispatchGate::all()),
            "x-a1"
        );
    }

    #[test]
    fn gate_skips_deactivated_plugin() {
        let tmp = tempfile::tempdir().unwrap();
        write_plugin(
            tmp.path(),
            "p",
            r#"["post_slice"]"#,
            r#"function on_post_slice(s) return s .. "-p" end"#,
        );
        let mut host = PluginHost::load(&[tmp.path().to_path_buf()]);

        // Activation resolves false at the plate level → skipped.
        let off = DispatchGate {
            printer_model: None,
            plate: [("plugin.p.enabled".to_string(), "false".to_string())]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        assert_eq!(host.dispatch_gated(&StubHook, "x".into(), &off), "x");
        assert!(!host.any_active_hook(HookKind::PostSlice, &off));

        // Absent from the map → default-activated, runs.
        assert_eq!(
            host.dispatch_gated(&StubHook, "x".into(), &DispatchGate::all()),
            "x-p"
        );
    }

    #[test]
    fn global_disable_skips_plugin_and_override_re_enables() {
        let tmp = tempfile::tempdir().unwrap();
        write_plugin(
            tmp.path(),
            "p",
            r#"["post_slice"]"#,
            r#"function on_post_slice(s) return s .. "-p" end"#,
        );
        let mut host = PluginHost::load(&[tmp.path().to_path_buf()]);

        // Globally disabled → skipped even under a permissive gate.
        host.set_global_enabled("p", false);
        assert_eq!(host.dispatch(&StubHook, "x".into()), "x");
        assert!(!host.any_active_hook(HookKind::PostSlice, &DispatchGate::all()));
        assert!(!host.list()[0].globally_enabled);

        // A per-slice override (plate tier) wins over the global tier and
        // re-enables it.
        let on = DispatchGate {
            printer_model: None,
            plate: [("plugin.p.enabled".to_string(), "true".to_string())]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        assert_eq!(host.dispatch_gated(&StubHook, "x".into(), &on), "x-p");

        // Re-enabling globally runs it again under the permissive gate.
        host.set_global_enabled("p", true);
        assert_eq!(host.dispatch(&StubHook, "x".into()), "x-p");
    }

    #[test]
    fn settings_global_promotes_only_at_explicit_on_levels() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("tagger");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(MANIFEST_FILE),
            "name=\"tagger\"\nversion=\"1.0.0\"\nentry=\"main.lua\"\nenabled_by_default=true\n\
             hooks=[\"post_slice\"]\n\n[settings.tag]\ntype=\"string\"\ndefault=\"DEF\"\n",
        )
        .unwrap();
        // The plugin echoes its resolved `tag` setting.
        std::fs::write(
            dir.join("main.lua"),
            r#"function on_post_slice(s) return s .. ":" .. settings.tag end"#,
        )
        .unwrap();
        let mut host = PluginHost::load(&[tmp.path().to_path_buf()]);

        // No override → manifest default.
        assert_eq!(host.dispatch(&StubHook, "x".into()), "x:DEF");

        // Plate explicitly on + tag set → the value is promoted.
        let on = DispatchGate {
            plate: [
                ("plugin.tagger.enabled".to_string(), "true".to_string()),
                ("plugin.tagger.tag".to_string(), "OVR".to_string()),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        };
        assert_eq!(host.dispatch_gated(&StubHook, "x".into(), &on), "x:OVR");

        // Plate inherit (no enabled key) but tag set → NOT promoted: the
        // plugin still runs (effective-on via the global default) but
        // resolves to the manifest default, because the plate isn't
        // explicitly `on`. This is the backend enforcing the rule.
        let inherit = DispatchGate {
            plate: [("plugin.tagger.tag".to_string(), "INH".to_string())]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        assert_eq!(host.dispatch_gated(&StubHook, "x".into(), &inherit), "x:DEF");
    }

    #[test]
    fn global_settings_feed_the_resolver() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("tagger");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(MANIFEST_FILE),
            "name=\"tagger\"\nversion=\"1.0.0\"\nentry=\"main.lua\"\nenabled_by_default=true\n\
             hooks=[\"post_slice\"]\n\n[settings.tag]\ntype=\"string\"\ndefault=\"DEF\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("main.lua"),
            r#"function on_post_slice(s) return s .. ":" .. settings.tag end"#,
        )
        .unwrap();
        let mut host = PluginHost::load(&[tmp.path().to_path_buf()]);

        // Default before any global setting is applied.
        assert_eq!(host.dispatch(&StubHook, "x".into()), "x:DEF");

        // A global-tier setting value resolves through (no overrides).
        let settings: BTreeMap<String, BTreeMap<String, String>> = [(
            "tagger".to_string(),
            [("tag".to_string(), "GLOBAL".to_string())]
                .into_iter()
                .collect(),
        )]
        .into_iter()
        .collect();
        host.apply_global(BTreeMap::new(), settings);
        assert_eq!(host.dispatch(&StubHook, "x".into()), "x:GLOBAL");
    }

    #[test]
    fn reload_picks_up_an_edit() {
        let tmp = tempfile::tempdir().unwrap();
        write_plugin(
            tmp.path(),
            "edit-me",
            r#"["post_slice"]"#,
            r#"function on_post_slice(s) return s .. "-v1" end"#,
        );
        let mut host = PluginHost::load(&[tmp.path().to_path_buf()]);
        assert_eq!(host.dispatch(&StubHook, "x".into()), "x-v1");

        std::fs::write(
            tmp.path().join("edit-me").join("main.lua"),
            r#"function on_post_slice(s) return s .. "-v2" end"#,
        )
        .unwrap();
        host.reload("edit-me").unwrap();
        assert_eq!(host.dispatch(&StubHook, "x".into()), "x-v2");
    }
}
