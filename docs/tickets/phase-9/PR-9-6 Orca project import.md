# PR-9-6 — import OrcaSlicer `.3mf` projects (geometry + settings)

Status: ✅ done (2026-06-01). Open
project imports a foreign Bambu/Orca `.3mf`: `core::orca_import` parses
`project_settings.config`, partitions keys by FFI bucket (machine
dropped), builds the n3o project (geometry via `load_3mf`, binds an
existing matching `PrinterInstance` — fallback flagged, never created),
carries Process/Filament settings as overrides, and surfaces an import
report dialog. Wired into `project_load`'s `ForeignProject` branch +
a `ProjectImported` event. Tested against the real `case-bambu-studio.3mf`.

> **Remaining:**
> 1. ✅ **Minimize overrides** (2026-05-31) — `import()` resolves our
>    cascade baseline for the bound printer/bed/filament and deltas
>    against it; redundant keys drop. On `case-bambu-studio.3mf` the
>    applied count falls 299 → 189 (110 redundant). The cascade panel now
>    shows genuine deviations, not the whole resolved config.
>
>    Enum-value validation: a foreign enum value our engine's
>    `OptionDef.enum_values` doesn't list (a fork-divergence; e.g. Bambu's
>    `ironing_pattern = "zig-zag"`, which OrcaSlicer's own `handle_legacy`
>    migrates to `rectilinear`) is dropped to `settings_incompatible` and
>    reported, not injected as an invalid override (4 on
>    `case-bambu-studio.3mf`).
>
>    **Process-preset adoption (2026-06-01, `91caea3`)** — the import now
>    maps the project's `print_settings_id` ("0.20mm Strength @BBL A1M") to
>    a bundled process slug and sets it as the plate's `quality_profile`
>    (per-plate process binding). The baseline then resolves against the
>    project's *own* process, so its preset values (e.g.
>    `outer_wall_speed = 60`) resolve natively instead of landing as
>    overrides — the real fix for the earlier lossy-vs-noisy bind. Falls
>    back to the instance's default process when we ship no match.
> 2. ✅ **Verify-via-gcode** (2026-06-01) — imported `case-bambu-studio.3mf`
>    (0.20mm Strength) slices its outer walls at 60 mm/s (`G1 F3600`), the
>    Strength preset value, not 200 — confirming the adopted process reaches
>    libslic3r's motion, not just the config echo. (`support_top_z_distance`
>    etc. likewise survive as overrides.)
>
> ✅ **Intent-based import (2026-06-01)** — import from
> `different_settings_to_system` (the project's own list of keys changed
> from its system preset) instead of the full computed delta.
> `OrcaProjectSettings::changed_from_system` flattens that BBS field (a
> list whose slots are `;`-joined keys) into a key set; `compute_overrides`
> takes an `only_keys` filter and imports just those (still bucket-filtered,
> machine dropped, enum/range-validated), letting everything else resolve
> from the adopted process. For `case.3mf` that's the **5** support keys
> (`enable_support`, `support_style`, `support_on_build_plate_only`,
> `support_top_z_distance`, `support_object_first_layer_gap`) — vs. the
> ~31 the full delta produced. Reported via `settings_from_change_list`.
> The field being **absent** (or `null`) falls back to the full delta;
> **present-but-empty** imports nothing (the project changed nothing).
> Guarded by enum/negative-sentinel validation done first (so the full-delta
> fallback path stays safe even for the invalid Bambu defaults a project
> never changed).

**Rescoped 2026-05-31** (project lead): the MVP import
item is **OrcaSlicer/BBS `.3mf` *project* import — geometry + the
project's settings** — not the user-facing preset importer. Preset /
profile import through the UI moves to **post-MVP** (the original
contents of this ticket; the offline `scripts/import_*.py` converters
that build the bundled profiles stay as dev tooling).

**Scope.** Open an OrcaSlicer / Bambu Studio `.3mf` project and
reconstruct an n3o project from it: the model geometry, the plate
layout + per-object placement, and **the settings the project was
authored with** — so a switching user opens their existing job and gets
a usable, slice-ready project, not just bare meshes.

**What already exists** (this is an extension, not a from-scratch build):
- **Geometry import** — the scene loader reads STL/OBJ/3MF meshes into
  the active plate (`src/viewport/ViewportCanvas.tsx` → `scene_load_*`).
- **`model_settings.config`** — `src-tauri/src/core/threemf/bbs_meta.rs`
  already parses per-object/part `name` + `extruder` (filament-index)
  assignment and per-plate assignment for multi-material model import.
- **3MF container + core spec** — `core/threemf/{container,core_spec}.rs`.

**The gap** (the core of this ticket): `Metadata/project_settings.config`
— the flattened print / filament / printer settings the project carries
— is **not** yet mapped into our project. (`bbs_meta.rs`'s doc names it
but there's no parser/mapping.) Importing "with settings" means reading
it and landing those values in the n3o project model.

**Acceptance criteria.**

- **Hooked into "Open project" — no separate menu item** (project lead,
  2026-05-31). Opening a Bambu Studio / OrcaSlicer `.3mf` via the
  existing File → Open project transparently *imports* it: `project_load`
  loads a native n3o project as before, and routes a foreign BBS/Orca
  project (the `ForeignProject` case) through the importer instead of
  erroring. Produce an n3o project:
  - geometry + per-object placement across **plates** (reusing the
    existing geometry + `model_settings.config` paths);
  - each plate's **printer + filament binding** inferred from the
    project's machine/filament settings (best-effort map to a bundled
    `PrinterInstance` / filament, with a clear fallback when the printer
    isn't one we ship);
  - the project's **settings** landed as **overrides** on the right
    cascade tier (project / plate), via the adapter's logical-key
    vocabulary — not silently dropped.
- **Lossy mapping is surfaced, not silent** — settings keys with no
  home in our model, an unrecognized printer, or values that don't
  translate are reported in an import summary ("imported N objects across
  M plates; bound to Bambu A1 mini; 7 settings applied, 3 unmapped: …"),
  not dropped quietly (memory: `no_silent_caps`; `no_hardcoded_libslic3r_classifications`
  — classify keys via the FFI schema, don't curate lists).
- **Round-trips a real OrcaSlicer project**: open a real A1 mini (and, if
  available, a U1) `.3mf` exported from OrcaSlicer/Snapmaker Orca, then
  **slice it and check the output** reflects the imported settings
  (verify-via-gcode), not just that meshes loaded.
- Clearly a **one-time import**, never a live OrcaSlicer link (PRD §5).

**Effort.** ~3–4 days (the geometry + `model_settings` half exists; the
`project_settings.config` → cascade-override mapping + printer/filament
inference + the report are the new work).

**Dependencies.** The 3MF reader + `bbs_meta.rs`; the cascade adapter's
logical-key vocabulary (to land settings as overrides); PR-9-5 (our
project model is the import target). No hardware dependency.

**Out of scope.**

- **User-facing preset / profile import** (machine/process/filament
  `.json` → cascade fragments in the user library) — **post-MVP**. The
  prior version of this ticket; the field-knowledge inventory done
  2026-05-31 still applies when it's picked up (reuse the `import_*.py`
  key sets + FFI bucket classification; add a user-profile-library
  overlay).
- Live / continuous sync with an OrcaSlicer install (PRD §5).
- PrusaSlicer / Cura projects — OrcaSlicer/BBS `.3mf` only for the MVP.
- Painted-region / per-volume material data, the `<assemble>` element
  (`docs/3mf-format-notes.md` "not yet read") — unless a real test
  project needs them.

**If cut:** document in the release notes (PR-9-7) that OrcaSlicer users
re-create projects manually for the MVP, and move project import to the
post-MVP list alongside preset import.
