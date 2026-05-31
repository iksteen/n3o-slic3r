# Bundled plugins

First-party Lua plugins that ship with n3o-slic3r, a sibling of the
bundled `resources/profiles/` tree. Each plugin is a subdirectory with a
`plugin.toml` manifest and its Lua entry file.

At startup the app loads plugins from two roots, bundled first then the
user's `~/.local/share/n3o-slic3r/plugins/`, so a user plugin overrides
a bundled one of the same name.

In dev runs the app reads this directory via `N3O_PLUGIN_ROOT` (mirrors
`N3O_PROFILE_ROOT` for profiles); a packaged build reads the copy in its
resource dir.

**platecycler** ships here — the flagship plugin, bundled + installed
with the app (mapped in `tauri.conf.json` `bundle.resources`; loaded in
dev via `N3O_PLUGIN_ROOT=./resources/plugins`). Like all plugins it's **off by
default** (opt-in) — enable it from the Plugins panel.

The other examples (beep-at-layer, pause-at-layer, rewrite-bed-temp,
filament-summary) stay under `examples/plugins/` as reference material;
copy one into your user plugins dir to try it.
