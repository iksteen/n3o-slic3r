# Bundled plugins

First-party Lua plugins that ship with n3o-slic3r, a sibling of the
bundled `profiles/` tree. Each plugin is a subdirectory with a
`plugin.toml` manifest and its Lua entry file.

At startup the app loads plugins from two roots, bundled first then the
user's `~/.local/share/n3o-slic3r/plugins/`, so a user plugin overrides
a bundled one of the same name.

In dev runs the app reads this directory via `N3O_PLUGIN_ROOT` (mirrors
`N3O_PROFILE_ROOT` for profiles); a packaged build reads the copy in its
resource dir.

The example plugins (beep-at-layer, pause-at-layer, rewrite-bed-temp,
platecycler) land here in later Phase 8 tickets.
