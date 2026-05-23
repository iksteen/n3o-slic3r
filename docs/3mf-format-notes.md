# `.3mf` format notes

Knowledge artifact from PR-2-4. Captures what we learned reading
BambuStudio + OrcaSlicer fixtures so the next person touching 3MF code
doesn't have to rediscover it.

## Container

`.3mf` is a zip archive with the entry layout:

```
[Content_Types].xml         # MIME registration; we don't parse it
_rels/.rels                 # main-part pointer; we don't parse it
3D/3dmodel.model            # main model (XML)
3D/Objects/object_N.model   # optional side-file models referenced from main
Metadata/                   # BBS/Orca metadata extensions (XML or JSON)
  model_settings.config       per-object + per-part settings
  project_settings.config     printer profile + cascade-side config
  plate_N.json                per-plate top-down info
  slice_info.config           when sliced (gcode-3mf only)
Auxiliaries/                # cover images, model pictures
```

Path matching is **case-insensitive** and the leading `/` may or may
not appear. `container.rs` canonicalizes both before lookup.

## 3MF Core spec (XML)

Namespace: `http://schemas.microsoft.com/3dmanufacturing/core/2015/02`.

```xml
<model unit="millimeter">
  <metadata name="Title">…</metadata>
  <resources>
    <object id="N" type="model">
      <mesh>                            <!-- leaf -->
        <vertices>
          <vertex x=… y=… z=…/>
        </vertices>
        <triangles>
          <triangle v1=… v2=… v3=…/>    <!-- v* are 0-based indices -->
        </triangles>
      </mesh>
    </object>
    <object id="M" type="model">
      <components>                      <!-- internal node -->
        <component objectid=N transform="…"/>
      </components>
    </object>
  </resources>
  <build>
    <item objectid=M transform="…" printable="1"/>
  </build>
</model>
```

Triangle winding is CCW. Coordinates are millimeters when `unit` is
`millimeter` (BBS always writes that).

### `transform` attribute

12 floats, space-separated. Represents a column-major 4×3 row-truncated
affine: `(a b c) (d e f) (g h i) (tx ty tz)` produces

```
| a d g tx |
| b e h ty |
| c f i tz |
| 0 0 0  1 |
```

Stack as `final = build_item × component_chain` for nested
`<components>`. Identity when absent.

## Production Extension

Namespace: `http://schemas.microsoft.com/3dmanufacturing/production/2015/06`
(conventional prefix `p:`).

```xml
<component p:path="/3D/Objects/object_1.model" objectid="3"/>
```

The `p:path` value is the canonicalized zip entry for the side-file
.model. BBS writes a leading `/`. Component references without `p:path`
resolve within the same .model file.

BBS uses `p:UUID` on objects, components, and build items — we parse
the elements but don't currently use the UUIDs.

## BBS / Orca metadata extensions

### `Metadata/model_settings.config`

Per-object + per-part settings keyed by the 3MF object id.

```xml
<config>
  <object id="9">                                <!-- matches 3dmodel.model -->
    <metadata key="name" value="…"/>
    <metadata key="extruder" value="1"/>          <!-- default -->
    <part id="1" subtype="normal_part">           <!-- maps to <component> in doc order -->
      <metadata key="name" value="Object_1"/>
      <metadata key="extruder" value="2"/>
      <metadata key="source_object_id" value="…"/>
      <metadata key="matrix" value="… 16 floats …"/>
    </part>
    …
  </object>
  <plate>
    <metadata key="plater_id" value="1"/>
    <model_instance>
      <metadata key="object_id" value="9"/>
    </model_instance>
  </plate>
  <assemble>
    <assemble_item object_id="9" …/>
  </assemble>
</config>
```

**Part-id order is document order** within an outer `<object>`. The
n-th `<part>` corresponds to the n-th `<component>` in the matching
3dmodel.model `<object>`. `apply_bbs_metadata` in `mod.rs` relies on
this ordering — confirmed against `fourcolor.3mf`.

The per-part `matrix` is a 4×4 transform that mirrors the
`<component>`'s 3MF Core transform. We currently honor the Core one
and ignore this duplicate.

### `Metadata/project_settings.config`

The cascade-side settings the project was authored against (printer
profile name, filament selections, layer height, …). Phase 2 surfaces
this as opaque text on `Project3mf.embedded_settings`. Phase 5
(Settings UI) parses it to *suggest* a matching cascade — the cascade
resolver itself never consumes 3MF settings.

## PrusaSlicer flavor (out of scope, MVP)

PrusaSlicer writes the same container shape but with different
metadata file names + schemas:

- `Metadata/Slic3r_PE_model.config`     (≅ model_settings.config; uses `<volume>` instead of `<part>`)
- `Metadata/Slic3r_PE_print_config.txt` (≅ project_settings.config)

`load_3mf` detects the Prusa marker and returns a typed Parse error
with the guidance: re-save through OrcaSlicer (BBS/Orca already
understand Prusa projects on import, so this is one keystroke for
the user). Adding direct support is ~half a day of XML mapping if a
real user asks.

## Fixtures we exercise

`examples/spike3/fourcolor.3mf` — 4-color Benchy (CC-BY-NC). Single
outer object id=9 with 8 components pointing into `Objects/object_1.model`.
Each component is a separate mesh; BBS metadata assigns extruders
1,2,3,4,1,2,3,4. The shape of every BBS multi-material project.

`external/OrcaSlicer/resources/handy_models/OrcaCube_v2.3mf` — bundled
Orca calibration cube + plug. Two top-level build items each pointing
at a single mesh; no per-part metadata. Tests the no-metadata path
and confirms that the ticket's "single mesh, single object" claim
was wrong (the file actually packages two distinct calibration
models).

## Things explicitly **not** yet read

- Auxiliaries (`Auxiliaries/Model Pictures/*.webp`, `.thumbnails/`,
  `Profile Pictures/`) — Phase 4 / Phase 5 UX.
- Painted-region / per-volume material data — Phase 5 / Phase 7.
- The `<assemble>` element — used by BBS for multi-plate assemblies;
  Phase 6 work.
- Per-plate detail (`Metadata/plate_N.json`, `top_N.png`, …) — these
  describe the *sliced* state, not the source project.
