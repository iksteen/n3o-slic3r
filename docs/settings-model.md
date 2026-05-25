# Settings Model

Target design for n3o-slic3r's printer / filament / process configuration system. Replaces the current flat-cascade-per-printer model.

**Status:** design under review. Implementation pending tickets (to be drafted from this doc).

**Backwards compatibility:** none. MVP, no projects in the wild yet.

**Compatibility with existing cascade:** 100%. The cascade format, resolver, and ladder (PR-1-2 / PR-1-3 / PR-1-4 / PR-1-5) are unchanged. What changes is how cascade inputs are assembled: previously bundled as one pre-baked TOML per printer; in this model composed at slice time from per-bucket vendor profiles + user overrides + instance state.

---

## 1. Buckets

Three orthogonal preset types, mirroring OrcaSlicer's `Preset.cpp` partitioning. Every `ConfigOptionDef` belongs to exactly one:

| Bucket | What it covers | Examples | Travels with project? |
|---|---|---|---|
| **Printer** | Physical hardware: machine envelopes, geometry, hardware-tied g-code | `nozzle_diameter`, `printable_area`, `machine_max_*`, `single_extruder_multi_material`, `start_gcode`, `end_gcode`, `change_filament_gcode` | No — printer presets live in the user's printer library only |
| **Filament** | Per-spool material properties | `nozzle_temperature*`, `bed_temperature*`, `filament_flow_ratio`, `filament_max_volumetric_speed`, `filament_diameter`, fan tables | No — referenced by ID, resolved against the user's filament library |
| **Process** | Slice strategy applied to geometry | `layer_height`, `wall_loops`, `sparse_infill_density`, `enable_support`, feature speeds, seams, brim, raft | Yes — process settings (including overrides) are what makes a project portable |

Reference: `docs/orcaslicer-settings-classification.md` for the OrcaSlicer-side derivation. Membership is data — sourced from OrcaSlicer's `printer_options()` / `filament_options()` / `print_options()` functions, exposed through the FFI as a per-`OptionDef` bucket tag.

Project file portability rule: **only process settings cross project boundaries.** Printer/filament references travel as IDs that the receiving machine resolves against its own library; if a referenced ID is missing, the user picks a local substitute at load time.

---

## 2. Hardware Topology

Every printer's hardware decomposes into:

```
Printer
└─ Extruder[N]                        (libslic3r's T0..T(N-1) cohort)
   ├─ Nozzle (currently installed)    (swappable: diameter, material, hotend rating)
   └─ Slot[M]                         (filament feeds — AMS slots, or 1 for direct)
```

**Extruder is what T<n> addresses in g-code.** Physical "extruder" packaging is irrelevant to slicing: a tool changer with 4 dockable physical heads and a single carriage with 4 side-by-side extruders both present as 4 extruders to libslic3r. Slot is below extruder — slots feed *into* an extruder; AMS slot switching is firmware-level (`M620`-style commands), not T<n>.

The only edge case is **mixing hotends** (Diamond, Mosquito Multi) where multiple extruder motors feed one nozzle and selection is by flow ratio rather than T<n>. Treated as 1 logical extruder with multi-feed (same shape as AMS) if/when supported.

**Special-case collapses:**

| Printer | Topology |
|---|---|
| A1 mini standalone | 1 extruder × 1 slot (direct), swappable nozzle |
| A1 mini + AMS Lite | 1 extruder × 5 slots (1 direct + 4 AMS), swappable nozzle |
| X1C / P1S + AMS (up to 4 banks) | 1 extruder × up to 17 slots (1 direct + up to 16 AMS), swappable nozzle |
| Snapmaker U1 | 4 extruders × 1 slot each, per-extruder nozzle |
| Prusa XL | 5 extruders × 1 slot each, per-extruder nozzle |
| IDEX (Sovol J1, Snapmaker dual) | 2 extruders × 1 slot each, per-extruder nozzle |
| Hypothetical Bambu H2D + AMS per side | 2 extruders × 5 slots each (1 direct + 4 AMS per side) |

The `(extruder, nozzle, slot)` cell is the full per-print configuration unit. Most printers degenerate one or more axes.

**External-spool vs AMS:** Bambu printers expose an external/direct feed alongside the AMS. Both are slots in the topology — the printer just *has* N slots, the model doesn't distinguish their physical feed path. The constraint that a single print uses one feed path at a time (you can't mix external with AMS-fed slots in the same job) is enforced at the **UI / pre-slice gate** layer (validation rejects mixed-feed bindings), not baked into the topology. Same pattern handles future AMS variants and any other "subset-of-slots-active-at-once" rules.

**Bed** is a fourth axis but sits at the printer level (not extruder-scoped). MVP: one bed loaded per printer instance. Post-MVP extension: `Vec<LoadedBed>` for platecycler support, with per-plate bed binding picking from that list.

---

## 3. Context Dimensions

Cascade resolution context, available as `when.*` predicates:

| Dimension | Bucket(s) it affects | Source |
|---|---|---|
| `extruder` | Printer | Per-extruder `when.extruder = E` predicates (T<n> cohort index) |
| `nozzle` | Printer, Filament | Variant-indexed keys (OrcaSlicer's `*_options_with_variant`) — `when.nozzle = "0.4mm-hardened"` etc. |
| `slot` | Filament | Per-slot filament-bucket resolution — `when.slot = S` (filament-feed index within the slot's extruder) |
| `build_plate` | Printer, Filament, Process | Cross-cutting plate-conditional values (`bed_temperature[plate_type]` matrix in filament; first-layer adjustments in process; probe sequence in printer) |
| `print_mode` | Process | Sport / Standard / Silent — process-bucket profile selection |
| `ams_topology` | Filament, Printer | Lite vs. full vs. none — gates which keys apply |
| `filament.type` | All | The active filament's `base_type` (e.g. `PLA`, `PETG`) |
| `filament.identity` | All | The active filament's identity string |
| `printer.identity` | All | The printer model identity |

**Predicate disambiguation:**

- `when.extruder = E` is the T<n> cohort index — necessary for multi-extruder printers (tool changers, IDEX).
- `when.slot = S` is **filament-feed-index relative to the slot's extruder**. On A1+AMS, `slot = 2` means "AMS slot 2 on the (single) extruder". On U1, `slot = 0` for every extruder (each has one slot).
- Compound `when.extruder = E, when.slot = S` selects a specific filament feed on a specific extruder (e.g. Bambu H2D with AMS per side).

Current `when.slot = N` predicates in the existing A1 mini cascade are unambiguous (1 extruder) and migrate cleanly. New predicates on multi-extruder printers must specify both axes.

---

## 4. Storage Model

### Vendor library (bundled, immutable)

```
profiles/
├── printer/
│   └── <vendor>/<model>.toml         ← e.g. bbl/a1-mini.toml
├── filament/
│   └── <vendor>/<material>.toml      ← e.g. bbl/pla-basic.toml
└── process/
    └── <vendor>/<preset>.toml        ← e.g. bbl/0.20mm-standard.toml
```

Each file is a cascade fragment scoped to its bucket. The converter (`convert_bbs_profile.py`) emits these from OrcaSlicer's per-bucket JSON tree (`external/OrcaSlicer/resources/profiles/<vendor>/{machine,filament,process}/`).

### User library (mutable, in user config)

```
~/.config/n3o-slic3r/
├── printers/
│   └── <instance-id>.json            ← PrinterInstance (mandatory per physical printer)
├── filament-overrides.json           ← { vendor_id → { key → value } }
├── filaments/
│   └── <copy-id>.json                ← user-owned filament copies (from explicit Copy)
├── process-overrides.json
└── processes/
    └── <copy-id>.json                ← user-owned process copies
```

**PrinterInstance** (`printers/<id>.json`):

```json
{
  "id": "instance-uuid",
  "vendor_profile_ref": "bbl/a1-mini",
  "display_name": "Garage A1 mini",
  "connection": {
    "host": "192.168.1.42",
    "serial": "AC12345678",
    "access_code": "01234567",
    "dev_mode": true
  },
  "hardware_state": {
    "extruders": [
      {
        "installed_nozzle": "0.4mm-hardened",
        "slots": [
          { "filament": { "vendor_id": "bbl/pla-basic", "copy_id": null } },
          { "filament": { "vendor_id": "bbl/petg-hf", "copy_id": null } }
        ]
      }
    ],
    "current_bed": "bbl/textured-pei"
  },
  "config_overrides": {
    "machine_start_gcode": "...custom for this instance...",
    "thumbnail_size": "200x200"
  }
}
```

**filament-overrides.json**:

```json
{
  "bbl/pla-basic": {
    "nozzle_temperature_initial_layer": 215,
    "bed_temperature": { "textured-pei": 60 }
  },
  "bbl/petg-hf": {
    "filament_max_volumetric_speed": 18
  }
}
```

Edits to vendor filament profiles land here, keyed by vendor ID. Catalog displays just "Bambu PLA Basic" — a dot indicator shows when user-local overrides exist for that profile. Per-field reset deletes the key from this map; profile-level reset deletes the whole `{ vendor_id }` entry.

**Filament copies** (`filaments/<id>.json`):

```json
{
  "id": "copy-uuid",
  "vendor_profile_ref": "bbl/pla-basic",
  "display_name": "PLA for cold garage",
  "config_overrides": {
    "nozzle_temperature_initial_layer": 220,
    "bed_temperature": { "textured-pei": 65 }
  }
}
```

Created by explicit "Copy" action. Appears as a distinct catalog entry alongside the original. Inherits unspecified keys from the vendor profile.

**Process overrides + copies** mirror the filament storage shape exactly.

### Project (per `.3mf`)

```
project.3mf/
└── Metadata/
    └── n3o_project.json
```

```json
{
  "plates": [
    {
      "id": 1,
      "printer_instance_ref": "instance-uuid",
      "process_binding": {
        "vendor_id": "bbl/0.20mm-standard",
        "copy_id": null
      },
      "process_overrides": {
        "sparse_infill_density": "25%"
      },
      "object_overrides": {
        "obj-uuid-1": { "wall_loops": "3" }
      },
      "scene": { ... },
      "last_slice_snapshot": { ... }
    }
  ]
}
```

Notably **absent from the project**: printer profile data, filament profile data, slot bindings, nozzle bindings. Those resolve against the user's library at load time via the references.

If a `printer_instance_ref` points to an instance the local user doesn't have, the load surfaces a picker: "this project was made on printer X; pick a local printer for plate N." Same pattern for filament references.

---

## 5. Resolution at Slice Time

`build_slice_input` composes the cascade input per slice job, then runs the unchanged resolver against it.

**Inputs:**

- `Plate` (process bindings + overrides, scene, plate-bound printer instance)
- `PrinterInstance` (resolved from `Plate.printer_instance_ref`) — gives extruders, nozzles, slot bindings, bed, instance overrides
- Vendor printer profile (loaded from `vendor_profile_ref`)
- For each slot: vendor filament profile, filament-overrides for that vendor, optional filament copy
- Vendor process profile + process-overrides + optional copy + plate overrides + object overrides

**Composition (cascade tier order, lowest precedence first):**

1. Vendor printer profile (Printer bucket)
2. PrinterInstance `config_overrides` (Printer bucket)
3. Vendor filament profile (Filament bucket) — per slot
4. `filament-overrides[vendor_id]` (Filament bucket) — per slot
5. Filament copy `config_overrides` if bound (Filament bucket) — per slot
6. Vendor process profile (Process bucket)
7. `process-overrides[vendor_id]` (Process bucket)
8. Process copy `config_overrides` if bound (Process bucket)
9. `Plate.process_overrides` (Process bucket)
10. `object_overrides[obj_id]` (Process bucket)

The resolver doesn't know about buckets — it just resolves with the appropriate `when.*` predicates against the layered sources. The `build_slice_input` writes the bucket-tag on each source so we can enforce "filament overrides can only contain filament-bucket keys" gates at load time.

**Per-slot vector-key assembly:**

For each key declared as variant-or-slot-indexed (per OrcaSlicer's `*_options_with_variant` tables):

1. For slot `s` in `0..N`:
   1. Build a slot-scoped context: `(printer, extruder_of(s), nozzle_of(extruder_of(s)), s, filament_of(s))`
   2. Run the resolver against the composed cascade with this context
   3. Read the scalar value of the key from the resolved map
2. Assemble the scalars into a vector key (`filament_ids`, `nozzle_temperature_initial_layer`, etc.)
3. Emit the vector to libslic3r via the FFI

For non-variant keys, a single resolution against the project-level context produces the scalar value directly.

**Wire-format flattening:** when serializing to libslic3r's config (which uses linear extruder indexing for vector keys), `(extruder, slot)` flattens to a printer-defined linear extruder index. The PrinterInstance carries the flattening rule (driven by the vendor printer profile's `extruders_count` and `filament_map` semantics).

---

## 6. Edit Routing

User edits in the settings panel route by bucket and binding state:

| Field bucket | Currently bound to | Edit goes to |
|---|---|---|
| Printer (machine-level) | vendor profile (via instance) | `PrinterInstance.config_overrides` (in place) |
| Filament (vendor profile, no copy bound) | vendor profile | `filament-overrides[vendor_id][key]` (in place; dot indicator appears) |
| Filament (user copy bound) | copy | The copy's own `config_overrides` (in place) |
| Process (vendor profile, no copy bound) | vendor profile | `process-overrides[vendor_id][key]` (in place; dot indicator) |
| Process (user copy bound) | copy | The copy's own `config_overrides` (in place) |
| Process (per-plate variation) | (any) | `Plate.process_overrides` (plate-scoped, doesn't touch the overlay) |
| Slot → filament binding | n/a (a reference) | `PrinterInstance.extruders[T].slots[S].filament` |
| Nozzle binding | n/a (a reference) | `PrinterInstance.extruders[T].installed_nozzle` |
| Bed binding | n/a (a reference) | `PrinterInstance.hardware_state.current_bed` |

**In-place edit semantics:** edits don't create named entities; they just append to the relevant override map. Visual indicators (dot, asterisk) show that overrides are present. Per-field reset deletes the override key; profile-level reset deletes all overrides for that profile.

**Copy semantics:** explicit user action. Creates a fully named user-owned profile in `filaments/<id>.json` or `processes/<id>.json`. Bindings can target the copy. The copy is an independent catalog entry — original and copy can both be bound on different slots/plates.

**Edit blast-radius indicators:**

- Filament/process override on a vendor profile → affects every binding of that vendor profile across every printer instance and every plate.
- Filament/process override on a copy → affects every binding of that copy.
- Printer override on an instance → affects every plate using that instance.
- Per-plate process override → affects this plate only.

The settings panel surfaces a subtle "synced across N plates / N instances" indicator next to each field so the user knows the blast radius before editing.

---

## 7. UI Shape

`SettingsPanelHost` already follows the right structure: top-row selectors (PrinterPicker exists) then a content area below. The model changes are:

**Top-row selectors** (siblings of PrinterPicker):

- **PrinterPicker** (exists) — picks `PrinterInstance` from user's library. Drives everything downstream.
- **NozzlePicker** (new) — per extruder. Hides when the extruder has only one available nozzle.
- **BedPicker** (uses existing `BuildPlateSelector`, relocated to the top row) — picks `current_bed` on the printer instance.
- **MaterialBindingPanel** (exists, PR-5-6 UI) — slot → filament-identity binding, with auto-bind. Stays as the canonical filament binding UX.

All selector writes route to the `PrinterInstance` and re-render every plate panel that references the same instance.

**Content area** (below selectors):

- Process-bucket fields only, categorized as today (Quality / Strength / Speed / Support / etc.).
- No SlotTabStrip + sync-edit + vector rendering — removed. Per-slot filament settings are edited via the bound filament profile (in-place override or copy), not multiplexed through tabs.

**Visual indicators:**

- Dot/asterisk on a field when overrides are present (vendor-vs-overridden, or copy-vs-base)
- Tier tints (existing, PR-4-7) extended to show: vendor / instance-override / filament-override / process-override / plate / object
- "Synced across N plates" badge near printer/filament/nozzle/bed selectors when more than one plate uses the same instance

**Printer panel (`PrinterPanel.tsx`)** stays focused on runtime/driver concerns: Connect / Send / Stop / AMS-live-state. Not settings editing.

---

## 8. Map onto Current Code

What stays:

- **Cascade core** (`core/cascade/`) — schema, resolver, ladder, trace, all unchanged.
- **Converter** (`scripts/spikes/convert_bbs_profile.py`) — kept, but emits per-bucket fragments instead of one monolithic file.
- **MaterialBindingPanel + auto-bind** — keep shape, change storage target from `Plate.material_bindings` to `PrinterInstance.extruders[T].slots[S]`.
- **PrinterPicker** — keep, drives PrinterInstance selection.
- **BuildPlateSelector** — keep the component, relocate to top selectors row.
- **Pre-slice gate** (PR-210) — keep, extend validation to check vector-key correctness in addition to binding presence.
- **Dry-run send** (PR-212) — unaffected.

What changes shape:

- **`OptionDef` schema** — add `bucket: OptBucket::{Printer, Filament, Process}`. Scrape membership from OrcaSlicer's `printer_options()` / `filament_options()` / `print_options()` and expose through the FFI.
- **`PrinterProfile`** — split into vendor profile (catalog) + `PrinterInstance` (user library). The current `PrinterProfile` struct becomes the vendor side.
- **`build_slice_input`** — becomes the cascade composer described in §5. Walks per-slot to assemble vector keys.
- **`SettingsPanelHost`** — add NozzlePicker; relocate BuildPlateSelector to top row.
- **`SettingsPanel`** — remove SlotTabStrip + sync-edit + vector rendering. Show process-bucket fields only.
- **`Plate`** — drop `material_bindings`. Add `printer_instance_ref` (in place of `printer_ref`), `process_binding`, `process_overrides`, `last_slice_snapshot`.
- **Project save format** — emit only what's described in §4 (plate-scoped data + references to library entries).

What goes away:

- **`profiles/cascades/bambu-a1-mini-default.toml`** — replaced by runtime composition from per-bucket vendor fragments.
- **`Plate.material_bindings`** — moves to PrinterInstance.
- **`SlotTabStrip` + `SyncEdit` + vector rendering in SettingsPanel** — obsolete.
- **The current monolithic-cascade-validate path** — replaced by per-bucket validation at load time + composition-time bucket gating.

What gets added:

- **`OptBucket` enum + bucket tag** in OptionDef + FFI.
- **`PrinterInstance` type + user-library storage** (`core/printer/instance.rs`, `core/printer/library.rs`).
- **Per-bucket override maps** (`filament-overrides.json`, `process-overrides.json`) + their loaders/savers.
- **Filament/process copy support** (`filaments/<id>.json`, `processes/<id>.json`).
- **NozzlePicker** component.
- **Bucket-bound editing** plumbing in the settings panel field write path.
- **Per-slot resolver invocation** in `build_slice_input` for vector keys.

---

## 9. MVP Exclusions

Deliberately out of scope; tracked for post-MVP:

- **Per-plate bed binding** for platecycler — MVP has one bed per printer instance (single-element `current_bed`). Extension path documented in §2.
- **In-app filament/process copy UX** — Copy action via menu in MVP; richer template management (saved configurations, presets-of-presets) post-MVP.
- **Per-spool calibration history** — drying state, usage hours, per-physical-spool tuning. Phase 9+.
- **Compatibility expression evaluator** (`compatible_printers_condition`) — MVP uses explicit `compatible_printers` lists; expression eval comes when adding more vendors.
- **Vendor profile upgrade flow** — MVP ships bundled profiles as-is; future upgrades happen via app upgrade. No mid-session library refresh.
- **System-profile editing UI** — MVP allows in-place overrides on vendor profiles via the override maps; richer "edit the vendor profile" surfaces (with explicit fork prompts, diff views, etc.) post-MVP.
- **Multi-AMS topology beyond single AMS** — the topology model supports it (just more slots), but the UI/binding flows are designed for 1 AMS per printer in MVP. X1C-style 4-AMS-bank setups come later.
- **Mixed-feed pre-slice validation** — MVP enforces "all bound slots share a feed path" with a simple gate. Post-MVP could surface a richer UI explaining feed-path constraints to the user inline.
- **Mid-print extruder swaps on U1** (the physical dock/undock orchestration) — Phase 7b ticket scope. The slicer-side abstraction (extruder selection via T<n>) is in scope here; the physical swap choreography is a driver concern.

---

## 10. Open Questions

To be resolved during ticket drafting, not before:

1. **OptBucket scrape source of truth** — scrape from OrcaSlicer's `Preset.cpp` text at build time, or expose via the FFI as part of `option_defs()`? Latter is cleaner; depends on whether the upstream FFI surface allows extending `OptionDef` cheaply.
2. **Variant-key flattening rule storage** — does the vendor printer profile carry an explicit `extruder_flatten` mapping, or is it implicit from `extruder × slot` iteration order? OrcaSlicer uses `filament_map` for this; we can adopt the same shape.
3. **PrinterInstance ID format** — UUID, or human-readable slug derived from `display_name`? UUID survives renames; slugs are friendlier in file listings. Probably UUID with a `display_name` field separately.
4. **Where does PrinterInstance live when the user has zero printers configured?** Fallback to a "no printer" placeholder (and disable Send), or refuse to load the app? Probably "no printer" placeholder + an "Add printer" entry-point in the printer panel.

---

## Next Steps

1. Review this doc.
2. Draft tickets — likely shape:
   - **PR-S-1**: OptBucket enum + scrape + FFI surface.
   - **PR-S-2**: Per-bucket vendor fragment converter (`convert_bbs_profile.py` re-shape).
   - **PR-S-3**: `PrinterInstance` type + user library + load/save.
   - **PR-S-4**: Filament/process override storage + copy storage.
   - **PR-S-5**: `build_slice_input` cascade composer + per-slot vector-key assembly.
   - **PR-S-6**: Pre-slice gate extension (vector-key correctness).
   - **PR-S-7**: SettingsPanelHost top-row selectors (NozzlePicker added, BuildPlateSelector relocated).
   - **PR-S-8**: SettingsPanel rewrite — drop SlotTabStrip + sync-edit + vector rendering; show process-bucket only.
   - **PR-S-9**: Project format rewrite (no migration).
   - **PR-S-10**: Edit-routing + override indicators + "synced across N plates" badges. **Ice-boxed.** Depends on UI surfaces that don't exist yet — no filament editor, no machine settings editor, no process profile selector, no profile-creation flow. The bucket-wide override storage (`filament-overrides.json` / `process-overrides.json`) and the edit-routing dispatcher have no consumers until those surfaces land. Unblocks: filament/process/machine editor + profile-CRUD UI tickets (not yet drafted).
   - **PR-S-11**: Exit-criteria smoke (multi-filament A1 + AMS Lite, multi-instance project, copy-vs-vendor binding). Legs 1 + 2 landed as `phase_s_smoke.rs` (see `docs/phase-s-smoke.md`); leg 3 deferred behind the in-app filament/process copy mechanic (§9 MVP exclusion).
3. Sequence: PR-7c (filament sync) blocks on PR-S-3/4/5 at minimum. PR-7a-8 (real-print smoke) can run independently on the current model since the send pipeline is downstream of the cascade.
