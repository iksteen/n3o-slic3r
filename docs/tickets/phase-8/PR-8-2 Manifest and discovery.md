# PR-8-2 — Plugin manifest + discovery

Status: ✅ done.

**Note on the implementation.** Discovery validates manifests but
**defers reading the entry `.lua` source to load time** (the host's
job) — it only confirms the entry file exists. The two-step
`RawManifest` → `validate()` → `PluginManifest` split keeps serde
permissive and gives every failure a typed `ManifestError`. `semver`
is used only to validate the `version` string (stored verbatim).

**Scope.** Define how a plugin declares itself and how the app finds
plugins on disk. A plugin is a directory containing a `plugin.toml`
manifest plus its Lua source; this ticket parses + validates the
manifest and scans the plugins folder into a list of discovered
plugins (valid and invalid). No runtime, no dispatch, no hot reload
yet — just "what plugins exist and what do they declare."

Owns **FR-PL-2** (plugin manifest).

**Acceptance criteria.**

- Manifest schema (`core/plugin/manifest.rs`, serde + `toml`):
  ```toml
  name = "platecycler"            # unique id, kebab-case
  version = "0.1.0"               # semver string
  entry = "main.lua"              # Lua file, relative to plugin dir
  hooks = ["post_slice"]          # subset of pre_slice/post_slice/pre_send
  printer_compatibility = ["any"] # ["any"] or a list of printer model strings
  description = "…"               # optional, one line

  [settings.swap_gcode]           # optional plugin-declared settings
  type = "string"                 # string | number | bool | enum
  default = "M400\n…"
  label = "Swap G-code"
  ```
  - `PluginManifest` struct with `name`, `version`, `entry`, `hooks:
    Vec<HookKind>`, `printer_compatibility`, `description`,
    `settings: BTreeMap<String, SettingDecl>`.
  - `HookKind` enum: `PreSlice`, `PostSlice`, `PreSend`. **No
    `Compose`** — deferred (see `phase-8.md`). An unknown hook name in
    the manifest is a validation error naming the allowed set.
  - `SettingDecl` carries `type`, `default`, `label`, and (for enum)
    `values`. Consumed by the cascade UI in PR-8-9; here it just
    parses + validates.

- Manifest validation (returns a typed `ManifestError`, not a panic):
  - `name` non-empty, kebab-case, unique within a discovery scan
    (duplicate names → both flagged).
  - `version` parses as semver.
  - `entry` is a relative path that stays inside the plugin dir (no
    `..` escape), exists, and ends in `.lua`.
  - `hooks` non-empty and all known.
  - `settings` default values type-match their declared `type`.

- Discovery (`core/plugin/discovery.rs`):
  - Plugins root: an app-data `plugins/` dir (resolve via Tauri's path
    API in the command layer; the core fn takes a `&Path` so it's
    unit-testable). Each immediate subdirectory with a `plugin.toml`
    is a candidate.
  - `discover(root: &Path) -> Vec<DiscoveredPlugin>` where
    `DiscoveredPlugin { dir, manifest: Result<PluginManifest,
    ManifestError> }` — a malformed manifest yields an `Err` entry,
    **never aborts the whole scan** (one broken plugin can't hide the
    others; its error surfaces in the panel later).
  - Reads the entry `.lua` source into the struct (or defers to load
    time — pick one; document it). Does **not** execute it.

- No `PluginRuntime` construction here, no Tauri commands, no folder
  watching. (Loading discovered plugins into runtimes + dispatch is
  the host's job; hot reload is a later ticket.)

- Tests:
  - Parse a fully-populated valid manifest; assert the struct.
  - Each validation failure (bad semver, unknown hook, `..` in entry,
    missing entry file, duplicate names, type-mismatched setting
    default) is rejected with the right `ManifestError`.
  - `discover` over a temp-dir fixture with one good plugin + one
    malformed manifest returns one `Ok` and one `Err`, scan intact.

**Effort.** ~1 day.

**Dependencies.** PR-8-1 (the module exists; `PluginError` may gain a
`Manifest` variant or `ManifestError` stands alone — pick the simpler).

**Out of scope.**

- Loading the Lua into a `PluginRuntime` and running anything → PR-8-3.
- Hot reload / folder watching → PR-8-10.
- The settings actually appearing in the cascade UI → PR-8-9.
- Resolving the real on-disk plugins root via Tauri paths is a thin
  command-layer wrapper; the core discovery fn is path-injectable for
  tests.
