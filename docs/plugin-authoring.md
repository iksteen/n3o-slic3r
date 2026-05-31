# Writing n3o-slic3r plugins

n3o-slic3r plugins are small Lua scripts that reshape the slice/send
pipeline without touching Rust. A plugin can rewrite resolved settings
before slicing, edit the G-code after slicing, or transform the bytes
just before they go to a printer — all through typed, sandboxed APIs.

This guide covers everything you need to write one: the layout, the
manifest, the hooks, the APIs, the sandbox, and a walkthrough of each
bundled example.

> **Not available (deferred post-MVP):** a project-level *compose* hook
> (cross-plate transforms) and *automatic hot reload* of plugin files.
> Plugins load on launch; after editing one, relaunch the app or use the
> Plugins panel's reload. See `docs/tickets/phase-8.md`.

---

## 1. Layout

A plugin is a directory containing a `plugin.toml` manifest and one Lua
entry file:

```
my-plugin/
├── plugin.toml
└── main.lua
```

Drop that directory into your user plugins folder:

```
$XDG_DATA_HOME/n3o-slic3r/plugins/my-plugin/      (or ~/.local/share/...)
```

(Bundled example plugins live in `examples/plugins/` in the repo; copy
one to the user folder, or point `N3O_PLUGIN_ROOT` at a folder of
plugins for a dev run.)

**Plugins are off by default.** Dropping one in makes it *available*, not
*active* — enable it from the Plugins UI (see §4). Loading happens at
launch; there's no file watcher yet, so relaunch (or use the panel's
reload) after adding or editing a plugin.

---

## 2. The manifest (`plugin.toml`)

```toml
name = "beep-at-layer"          # unique id, kebab-case (a-z 0-9 -)
version = "0.1.0"               # semver
entry = "main.lua"             # entry Lua, relative to this dir (no `..`)
hooks = ["post_slice"]          # which hooks the plugin defines
description = "Beep (M300) at a chosen layer."   # optional, one line

# Optional. Which printer models this plugin applies to. Omit (or use
# ["any"]) for all printers. A plugin scoped to a model is NEVER
# dispatched for another — enforced by the host, not just convention.
printer_compatibility = ["Bambu Lab A1 mini"]

# Optional. Which cascade levels the plugin can be enabled/configured at.
# Vocabulary: "global", "project", "plate". Omit = all three. A plugin
# that's only meaningful machine-wide declares scopes = ["global"].
scopes = ["global", "project", "plate"]

# Optional. The opt-in default. Plugins are OFF unless this is true.
# Leave it out (or false) for an opt-in plugin; set true only for a
# plugin that should run the moment it's installed.
enabled_by_default = false

# Optional. Declared settings — rendered as controls in the Plugins UI
# and delivered to the hooks as the `settings` global (see §7).
[settings.layer]
type = "number"                 # string | number | bool | enum
default = 1
label = "Layer"                 # optional display label

[settings.mode]
type = "enum"
values = ["Temperature", "Retraction", "Flow"]   # required for enum
default = "Temperature"
```

Validation rejects (with a clear error, surfaced in the panel): a
non-kebab `name`, a bad `version`, an `entry` that escapes the dir or
isn't `.lua`, an empty/unknown `hooks` entry, an unknown `scopes` value,
and a setting whose `default` doesn't match its `type`.

---

## 3. The hooks

Define a global Lua function for each hook you declared. All are
optional to *define* — a declared-but-undefined hook is a no-op.

### `on_pre_slice(cascade, context, filament)`
Runs after the cascade resolves, before libslic3r slices. Read or
**write** resolved settings.

- `cascade` — the resolved libslic3r settings, a read/write table:
  `cascade.bed_temperature = "60"`. Values are strings (libslic3r's
  vocabulary); an integer is stringified, a float is rejected (use
  `tostring()` so you control formatting). Assigning `nil` is a no-op
  (you can't delete a setting). New keys are added.
- `context` — `{ printer_model, plate, toolhead_count }`.
- `filament` — the read-only filament loadout (§6).

> **Naming note.** The first argument is the *cascade* settings. If your
> plugin also has its own declared settings, name this argument something
> other than `settings` (e.g. `cascade`, as above) so the plugin's own
> `settings` global (§7) isn't shadowed.

### `on_post_slice(gcode, plate, filament)`
Runs after each plate slices, before preview/send. Edit the G-code.

- `gcode` — the typed G-code, a mutable handle (§5).
- `plate` — `{ plate_id, printer_model, bed_type, object_count }`
  (`bed_type`/`object_count` may be absent).
- `filament` — the read-only filament loadout (§6).

### `on_pre_send(payload, target) -> string | nil`
Runs before a driver sends a payload. Return replacement bytes (a Lua
string) to rewrite the buffer, or `nil`/nothing to leave it unchanged.

- `payload` — `{ kind = "gcode" | "gcode_3mf", bytes = <string> }`.
  A `gcode_3mf` payload (Bambu's zip bundle) is **skipped** by the host —
  pre-send only sees raw `gcode` (U1) today.
- `target` — `{ driver_kind = "bambu" | "u1", plate_id }`.

**Isolation.** A hook that errors (Lua error, timeout, bad return) is
caught: its edit is discarded, the plugin is disabled for the session,
its error shows in the Plugins panel, and the slice/send proceeds as if
the plugin weren't there. One plugin can't break another or the host.

---

## 4. Activation — the cascade

Whether a plugin *runs* resolves through three levels, lower overriding
higher:

```
global   →   project   →   plate
```

- **Global** (binary on/off): the Plugins panel from the **n3o-slic3r
  brand menu** ("Global plugins…"). Persisted to
  `~/.config/n3o-slic3r/config.toml`, applies to every project.
- **Project** (tri-state on/off/inherit): the **project menu** →
  "Plugins…". Stored in the project's `.3mf`.
- **Plate** (tri-state): the **settings panel's "Plugins" tab**. Per
  plate, in the `.3mf`.

`inherit` means "use the level above." The first explicit value walking
**plate → project → global** wins; if nothing is set anywhere, the
manifest's `enabled_by_default` (default false) decides.

A plugin's **settings** are editable at a level **only where it's
explicitly `on`** there — and the resolved setting values overlay the
same way (global → project → plate). `printer_compatibility` is a
separate gate: an incompatible plugin never runs regardless of
activation.

---

## 5. The typed G-code API (`gcode`)

The post-slice `gcode` handle is the parsed G-code as typed lines — not
a string. Indices are **1-based**.

**Read:**
- `#gcode` / `gcode:len()` — line count.
- `gcode:line(i)` — the i-th line as a table, or `nil` out of range.
- `for line in gcode:lines() do … end` — iterate lines (read-only;
  don't mutate the buffer mid-iteration — use `layers()` for that).
- `for layer in gcode:layers() do … end` — iterate layers; each is
  `{ index, z, first_line, last_line }`. Positions recompute live, so
  inserting ordinary lines at several layers while iterating stays
  aligned (don't insert/remove `LayerChange` lines mid-iteration).

A **line table** carries a `kind` and kind-specific fields:
- `move` — `x, y, z, e` (numbers, may be absent), `f` (feedrate),
  `command` (`"G0"`/`"G1"`/`"G2"`/`"G3"`), `travel` (bool).
- `comment` — `text` (the raw comment incl. delimiter), `style`
  (`"semicolon"`/`"parens"`), `semantic` (e.g. `"layer"`, `"bed_temp"`,
  when recognized).
- `layer_change` — `index`, `z`, `source` (`"marker"`/`"heuristic"`).
- `tool_change` — `tool` (extruder index).
- `other` — `raw`.

**Mutate** (the value is a raw G-code string — parsed, may expand to
several lines — or a constructed `comment`/`other` table):
- `gcode:append(v)` — add at the end.
- `gcode:insert(i, v)` — insert before line `i`.
- `gcode:replace(i, v)` — replace line `i`.
- `gcode:remove(i)` — delete line `i`.

```lua
-- string form (any G-code; move/tool/layer lines must be strings)
gcode:append("M300 S440 P200")
-- constructed comment / other
gcode:append({ kind = "comment", text = "post-processed by my-plugin" })
gcode:insert(1, { kind = "other", raw = "M73 P0" })
```

---

## 6. The filament loadout (`filament`)

A **read-only** snapshot of the bound per-slot filament for this slice
(material→slot mapping, not a live driver readout). Handed to
`on_pre_slice` / `on_post_slice` as the third argument.

- `filament:slots()` — array of `{ index, extruder, slot, feed,
  identity, type, color, vendor, bound }`. `index` is the 1-based
  filament index (material `index` emits `T<index-1>`); `feed` is
  `"direct"`/`"ams"`; unbound slots have `bound = false` and `nil`
  identity fields.
- `filament:slot(i)` — the i-th slot, or `nil`.
- `filament:count()` — slot count.
- `filament:printer()` — `{ model, toolhead_count }`.

```lua
function on_post_slice(gcode, plate, filament)
  for _, s in ipairs(filament:slots()) do
    if not s.bound then return end          -- bail if any slot is empty
  end
  -- … all slots loaded …
end
```

Assignment raises — it's a one-way view; the host's state is never
writable from Lua.

---

## 7. Your settings (`settings`)

Your manifest-declared settings are delivered as a **read-only**
`settings` global, typed per the declaration:

```lua
-- given [settings.layer] type="number" and [settings.tag] type="string"
settings.layer    -- a number
settings.tag      -- a string
```

`settings` is resolved per slice from the cascade (the value the user
set at the deepest explicitly-on level, else your manifest default). In
`on_pre_slice`, remember the naming note in §3 — don't name the first
hook argument `settings`.

---

## 8. The sandbox

Plugins run in a restricted Lua 5.4 VM:

- **Available:** `string`, `table`, `math`, `coroutine`, the base
  library (`print`, `pairs`, `ipairs`, `pcall`, `type`, `tostring`, …),
  and `os.time` / `os.clock` (timing only).
- **Removed:** `io`, the real `os` (`execute`/`getenv`/`remove`/…),
  `package`/`require`, `debug`, the dynamic loaders
  (`load`/`loadstring`/`loadfile`/`dofile`), and the metatable escape
  hatches `rawset`/`getmetatable`/`setmetatable` (so the read-only views
  above can't be bypassed). No filesystem, no network, no shelling out.
- **Budgets:** a per-call instruction budget (~50M — a runaway loop is
  aborted in a fraction of a second) and a 64 MiB memory cap.

Keep hooks fast and side-effect-free; their only effect should be
through the typed APIs.

---

## 9. Walkthroughs (the bundled examples)

All live in `examples/plugins/`.

**`beep-at-layer`** (post-slice, `layer` number setting): iterate layers,
insert an `M300` beep at the boundary of the chosen layer.
```lua
function on_post_slice(gcode, plate)
  for layer in gcode:layers() do
    if layer.index == settings.layer then
      gcode:insert(layer.first_line, "M300 S440 P200")
    end
  end
end
```

**`pause-at-layer`** (post-slice): same shape, inserting `M0` (adjust to
your firmware's pause).

**`rewrite-bed-temp`** (pre-slice): clamp the resolved bed temperature
into a band by writing the cascade setting.
```lua
function on_pre_slice(cascade, ctx)
  if tonumber(cascade.bed_temperature) and tonumber(cascade.bed_temperature) > 60 then
    cascade.bed_temperature = "60"
  end
end
```

**`platecycler`** (post-slice, `printer_compatibility = ["Bambu Lab A1
mini"]`): append the Chitu PlateCycler eject/swap macro **inside** the
executable block (just before `; EXECUTABLE_BLOCK_END`, where Bambu
firmware will run it), idempotently via a sentinel comment, so the
finished plate auto-ejects at print end.

**`filament-summary`** (post-slice): a read-only demo — prepend a
header listing the bound loadout (see §6).

---

## 10. Debugging + reloading

- A plugin that fails to load, or errors at runtime, is **kept** in a
  disabled/errored state and shows its error in the Plugins panel — it
  never crashes the host or silently vanishes.
- After fixing a plugin's file, **reload** it from the panel (or relaunch
  the app). Re-enabling a previously-errored plugin clears the stale
  error and re-attempts the load.
- `print(...)` goes to the app's log; use it sparingly for tracing.

---

## What's not here

- **Compose hook** (cross-plate / `.3mf`-level transforms) — deferred
  post-MVP. The MVP hooks are pre-slice, post-slice, pre-send.
- **Automatic hot reload** — deferred post-MVP. Load on launch + manual
  reload.
- **io / os / network / arbitrary code loading** — removed by the
  sandbox, by design.
