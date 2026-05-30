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

use super::discovery::{discover, DiscoveredPlugin, MANIFEST_FILE};
use super::error::PluginError;
use super::manifest::{parse_manifest, HookKind, PluginManifest, PrinterCompat};
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

/// Serializable view of a plugin for the `plugin_list` command.
#[derive(Debug, Serialize)]
pub struct PluginSummary {
    pub name: String,
    pub version: String,
    pub hooks: Vec<String>,
    /// `None` means "any printer"; `Some(list)` restricts to models.
    pub printers: Option<Vec<String>>,
    pub enabled: bool,
    pub last_error: Option<String>,
}

/// Owns every discovered plugin and dispatches hooks across them.
pub struct PluginHost {
    /// Name-sorted; later-discovered roots override earlier ones on a
    /// name clash (user plugins shadow bundled ones).
    plugins: Vec<LoadedPlugin>,
}

impl PluginHost {
    /// Empty host (no plugins). Useful as a default before roots are
    /// resolved, and in tests.
    pub fn empty() -> Self {
        Self {
            plugins: Vec::new(),
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
        }
    }

    /// Fold `value` through every enabled plugin that declares the
    /// hook, in name order. Plugin failures are isolated (recorded +
    /// the plugin auto-disabled) and never surface to the caller.
    pub fn dispatch<H: Hook>(&mut self, hook: &H, value: H::Value) -> H::Value {
        let kind = hook.kind();
        let mut current = value;
        for i in 0..self.plugins.len() {
            {
                let p = &self.plugins[i];
                if !p.enabled || p.runtime.is_none() || !p.manifest.hooks.contains(&kind) {
                    continue;
                }
            }
            // Borrow the runtime only for the invoke, then release it
            // so the error path can mutate the plugin's state.
            let (next, err) = {
                let runtime = self.plugins[i].runtime.as_ref().expect("checked above");
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

    /// Whether any enabled, loaded plugin declares `kind`. Lets a
    /// caller skip building a hook payload entirely when nothing would
    /// run (e.g. the orchestrator avoids re-parsing G-code).
    pub fn any_hook(&self, kind: HookKind) -> bool {
        self.plugins
            .iter()
            .any(|p| p.enabled && p.runtime.is_some() && p.manifest.hooks.contains(&kind))
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
                enabled: p.enabled,
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
                "name=\"{name}\"\nversion=\"1.0.0\"\nentry=\"main.lua\"\nhooks={hooks}\n"
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
