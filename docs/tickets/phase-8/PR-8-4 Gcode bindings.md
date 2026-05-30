# PR-8-4 — Typed G-code Lua bindings

Status: ❌ open.

**Scope.** Expose the typed G-code model (`core/gcode`) to Lua so a
post-slice plugin manipulates a structured sequence — `Move` /
`Comment` / `LayerChange` / `ToolChange` / `Other` — instead of raw
strings (FR-PL-4). Read access to every line's fields plus
insert/replace/remove/append, with re-serialization back to G-code via
the existing `gcode::to_string`. This is the data layer the post-slice
hook (PR-8-5) hands to plugins.

Owns **FR-PL-4** (structured G-code API).

**Acceptance criteria.**

- A `Gcode` Lua userdata (`core/plugin/bindings/gcode.rs`) wrapping the
  parsed `Vec<gcode::Line>` for one plate. Constructed host-side from
  `gcode::parse_lines` / `parse_str`; handed to the hook; mutated
  in-place; re-serialized with `gcode::to_string` after the chain.

- Read API (Lua):
  - `g:len()` / `#g` — line count.
  - `g:line(i)` — 1-based; returns a line view.
  - `g:lines()` — iterator over line views.
  - `g:layers()` — iterator yielding `{ index, z, first_line,
    last_line }` groups, segmented on `LayerChange` (maps
    `gcode::Line::LayerChange` + `LayerSource`).
  - A **line view** exposes `kind` (`"move"|"comment"|"layer_change"
    |"tool_change"|"other"`) and kind-specific read fields:
    - move: `x`,`y`,`z`,`e`,`f`, `command` (`G0`/`G1`/`G2`/`G3`),
      `feature` (the `FeatureType` as a string, when annotated),
      `travel` (bool: `E` absent/≤0).
    - comment: `text`, `style`, `semantic` (the `SemanticComment`
      variant name when classified).
    - layer_change: `z`, `source`.
    - tool_change: `tool` index.
    - other: `raw`.
  - Field reads map directly off the `model.rs` types — no
    re-parsing. Decide view representation (lightweight Lua table
    snapshot per accessed line vs. userdata proxy); document the
    choice and the cost. Default: per-line userdata proxy so large
    files don't materialize N tables eagerly.

- Mutation API (Lua):
  - `g:insert(i, line)` — insert before index `i`.
  - `g:replace(i, line)` — replace at `i`.
  - `g:remove(i)` — delete at `i`.
  - `g:append(line)` — push at the tail (the operation platecycler
    needs in PR-8-7).
  - A `line` argument may be a **raw G-code string** (parsed via
    `gcode::parse_lines`, may expand to several lines) or a
    constructed table (`{ kind = "comment", text = "…" }`). Reject
    malformed constructed lines with a clear Lua error.
  - Mutations are bounds-checked; out-of-range index → Lua error, not
    a panic.

- Round-trip fidelity: a plugin that reads and writes nothing back
  leaves `to_string` byte-identical to the parser's input (the
  serializer already guarantees this; the binding must not perturb it).

- Tests (Rust-side, driving Lua through `PluginRuntime`):
  - Count layers in a fixture; assert against the known layer count.
  - Append a comment; re-serialize; assert the tail line is present
    and the rest is unchanged.
  - Insert a raw `M601` pause string at a layer boundary; assert it
    lands at the right index and parses back.
  - Out-of-range `insert`/`remove` raise a Lua error caught as
    `PluginError::Runtime`.
  - No-op pass is byte-identical round-trip.

**Effort.** ~2 days. The userdata proxy + the string/table line
constructor are the fiddly parts.

**Dependencies.** PR-8-1 (`PluginRuntime`), `core/gcode` (existing
typed model + serializer). Independent of PR-8-2/8-3 — can be built in
parallel and wired together at PR-8-5.

**Out of scope.**

- Running the binding inside a real post-slice dispatch / slice
  pipeline → PR-8-5.
- Settings, filament, or send-payload bindings → PR-8-6, PR-8-8.
- Mutating arc moves' geometry semantically (plugins can read
  `G2`/`G3` fields and replace lines wholesale; no arc-aware editing
  helper for MVP).
