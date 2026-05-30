//! Plugin discovery: scan a plugins folder into a list of plugins,
//! valid and invalid.
//!
//! Each immediate subdirectory of the root that contains a
//! `plugin.toml` is a candidate. A malformed manifest yields a
//! [`DiscoveredPlugin`] with an `Err` rather than aborting the scan —
//! one broken plugin must never hide the others (its error surfaces in
//! the Plugins panel later).
//!
//! Discovery validates manifests but does **not** read or execute the
//! entry Lua — the host loads that into a runtime when it builds. The
//! one cross-manifest check, duplicate names, is applied here after the
//! per-directory parse.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::manifest::{parse_manifest, ManifestError, PluginManifest};

/// Manifest filename expected in each plugin directory.
pub const MANIFEST_FILE: &str = "plugin.toml";

/// One discovered plugin directory and the result of parsing its
/// manifest.
#[derive(Debug)]
pub struct DiscoveredPlugin {
    pub dir: PathBuf,
    pub manifest: Result<PluginManifest, ManifestError>,
}

/// Scan `root` for plugin directories. Returns one entry per
/// subdirectory containing a `plugin.toml`, sorted by path for a
/// deterministic order. A missing/unreadable `root` yields an empty
/// list (no plugins installed is not an error).
pub fn discover(root: &Path) -> Vec<DiscoveredPlugin> {
    let mut found = Vec::new();
    let read = match std::fs::read_dir(root) {
        Ok(r) => r,
        Err(_) => return found,
    };
    for entry in read.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let manifest_path = dir.join(MANIFEST_FILE);
        if !manifest_path.is_file() {
            // Not a plugin directory; skip silently.
            continue;
        }
        let manifest = match std::fs::read_to_string(&manifest_path) {
            Ok(src) => parse_manifest(&src, &dir),
            Err(e) => Err(ManifestError::Io(e.to_string())),
        };
        found.push(DiscoveredPlugin { dir, manifest });
    }
    found.sort_by(|a, b| a.dir.cmp(&b.dir));
    flag_duplicate_names(&mut found);
    found
}

/// Demote every successfully-parsed plugin whose name collides with
/// another to a `DuplicateName` error, so neither silently wins.
fn flag_duplicate_names(found: &mut [DiscoveredPlugin]) {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for plugin in found.iter() {
        if let Ok(manifest) = &plugin.manifest {
            *counts.entry(manifest.name.clone()).or_default() += 1;
        }
    }
    for plugin in found.iter_mut() {
        if let Ok(manifest) = &plugin.manifest {
            if counts.get(&manifest.name).copied().unwrap_or(0) > 1 {
                let name = manifest.name.clone();
                plugin.manifest = Err(ManifestError::DuplicateName(name));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_plugin(root: &Path, subdir: &str, manifest: &str) {
        let dir = root.join(subdir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(MANIFEST_FILE), manifest).unwrap();
        std::fs::write(dir.join("main.lua"), "-- lua").unwrap();
    }

    fn manifest_named(name: &str) -> String {
        format!(
            r#"name="{name}"
version="1.0.0"
entry="main.lua"
hooks=["post_slice"]"#
        )
    }

    #[test]
    fn empty_or_missing_root_yields_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(discover(tmp.path()).is_empty());
        assert!(discover(&tmp.path().join("does-not-exist")).is_empty());
    }

    #[test]
    fn finds_good_and_keeps_bad_in_the_scan() {
        let tmp = tempfile::tempdir().unwrap();
        write_plugin(tmp.path(), "good", &manifest_named("good"));
        // Malformed: unknown hook.
        write_plugin(
            tmp.path(),
            "bad",
            r#"name="bad"
version="1.0.0"
entry="main.lua"
hooks=["compose"]"#,
        );
        // A non-plugin directory (no plugin.toml) is ignored.
        std::fs::create_dir_all(tmp.path().join("not-a-plugin")).unwrap();

        let found = discover(tmp.path());
        assert_eq!(found.len(), 2, "should find exactly the two plugin dirs");
        // Sorted by path: "bad" before "good".
        assert!(found[0].manifest.is_err());
        assert_eq!(found[1].manifest.as_ref().unwrap().name, "good");
    }

    #[test]
    fn duplicate_names_are_both_flagged() {
        let tmp = tempfile::tempdir().unwrap();
        write_plugin(tmp.path(), "a", &manifest_named("dup"));
        write_plugin(tmp.path(), "b", &manifest_named("dup"));
        let found = discover(tmp.path());
        assert_eq!(found.len(), 2);
        for plugin in &found {
            assert!(
                matches!(plugin.manifest, Err(ManifestError::DuplicateName(_))),
                "both colliding plugins should be flagged"
            );
        }
    }
}
