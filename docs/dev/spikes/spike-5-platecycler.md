# Spike 5: platecycler portability

## Assumption tested

The existing `platecycler` Python tool — the post-process pipeline
that combines multi-plate Bambu Studio `.gcode.3mf` files into a
single `.platecycler.3mf` for a Chitu PlateCycler — operates on
gcode produced by libslic3r through our FFI, not just on gcode
produced by Bambu Studio itself. If it doesn't, Phase 8's
compose-hook plugin starts from a known-broken base.

## Method

1. Clone `https://github.com/iksteen/platecycler` at commit
   `ceaece1e4f3251bab7294b9886683ed7c4820358` to `~/platecycler/`.
   Install via `python3 -m venv .venv && .venv/bin/pip install -e .`.
   Pillow is the only runtime dependency.

2. **Baseline.** Run platecycler against BBS's `.gcode.3mf` from
   PR-0.5-3 (`~/spike3-bbs/output.gcode.3mf`):

   ```bash
   ~/platecycler/.venv/bin/platecycler ~/spike3-bbs/output.gcode.3mf \
       -o ~/spike3-bbs/output.platecycler.3mf --force
   ```

   Expected: produces `output.platecycler.3mf`, prints
   `(1 plates, gcode md5 …)`.

3. **Libslic3r-body test.** Build a "Frankenstein"
   `.gcode.3mf` whose metadata wrapper comes from BBS but whose
   `Metadata/plate_1.gcode` is replaced with our libslic3r-emitted
   gcode (`/tmp/spike3.gcode` from PR-0.5-3):

   ```bash
   cd ~/spike3-bbs && mkdir -p frankenswap && cd frankenswap
   cp ~/spike3-bbs/output.gcode.3mf orig.zip
   unzip -o orig.zip -d extracted
   cp /tmp/spike3.gcode extracted/Metadata/plate_1.gcode
   md5sum extracted/Metadata/plate_1.gcode \
       | awk '{print $1}' > extracted/Metadata/plate_1.gcode.md5
   cd extracted && zip -r ../libslic3r-body.gcode.3mf .
   cd ..
   ~/platecycler/.venv/bin/platecycler libslic3r-body.gcode.3mf \
       -o output.platecycler.3mf --force
   ```

4. Compare the resulting plate-1 gcode body in each
   `.platecycler.3mf` to its source to confirm the body
   round-tripped intact and the swap-gcode macro got appended.

## Result

**PASS.** Both runs complete cleanly with no errors.

- **Baseline** produces a valid `.platecycler.3mf`, 1 plate, md5
  `5f0fa2f0735037cd5fda814a27ae80a9`.
- **Libslic3r-body** produces a valid `.platecycler.3mf`, 1 plate,
  md5 `7c1cf4c42389ad6426655fc79ef7f247`. The output's
  `Metadata/plate_1.gcode` is `head -20`-identical to our
  `/tmp/spike3.gcode` and grows by 24 lines at the tail — the
  embedded `DEFAULT_SWAP_GCODE` (ejector + reset macro) appended
  after the slice ends, exactly as `merged_gcode` is supposed to.
- All filament aggregate comments (`; filament used [mm]`,
  `; filament used [g]`, `; filament cost`, etc.) survive the
  transform unchanged at the bottom of the body.

Output metadata structure matches platecycler's documented
behavior (CLAUDE.md):

| File | Input → Output |
|---|---|
| `Metadata/plate_1.gcode` | re-emitted, with swap-gcode appended |
| `Metadata/plate_1.gcode.md5` | refreshed |
| `Metadata/model_settings.config` | compacted (7 KB → 0.5 KB), objects stripped |
| `Metadata/slice_info.config` | compacted (2.7 KB → 2.4 KB), single plate |
| `Metadata/plate_1.png` etc. | regenerated as a 1×1 collage (no degradation) |
| `3D/3dmodel.model` | geometry stripped (5.9 KB → 4.2 KB) |
| `3D/Objects/`, `3D/_rels/3dmodel.model.rels` | removed (Cycler doesn't need geometry) |

## Why the gcode-body test is a meaningful baseline despite using BBS's metadata wrapper

Platecycler's transform pipeline operates almost entirely on the
`Metadata/*.config` XML/JSON files (slice_info, model_settings,
filament aggregates, prediction). It touches the gcode *body* in
exactly two places:

1. `merged_gcode` concatenates plate files at the file level —
   pure byte-stream operation, dialect-agnostic.
2. `rendered_plate_thumbnail` (fallback when source thumbnails
   are blank) traces extrusion moves (`G1/G2/G3` with `E>0`) to
   render a plate preview. Uses standard G-code primitives that
   any FFF emitter writes the same way.

Neither path is sensitive to libslic3r-vs-BBS dialect divergences
(comment styles, CONFIG_BLOCK formatting, etc.). The metadata
wrapper *is* — but PR-0.5-3's table already enumerated the
metadata files Phase 5's `.gcode.3mf` wrapper needs to emit
(blocking/probably-blocking/cosmetic). As long as the wrapper
matches BBS's shape (which is what PR-0.5-3 prescribes),
platecycler will consume the output.

## Known gaps (not tested in this spike)

- **Single-plate test only.** PR-0.5-3 only produced a
  single-plate fixture. Platecycler's defining feature
  (concatenating across plates, summing filament/prediction
  totals) needs a multi-plate `.gcode.3mf` to exercise fully.
  This is fine for the spike — the per-plate transform pipeline
  is the same whether N=1 or N=N — but the integration of
  cross-plate merging with libslic3r-body gcode isn't proven
  until Phase 8 / Phase 5.

- **Thumbnail-fallback path** (`rendered_plate_thumbnail`)
  unexercised. Our test inputs all carry BBS thumbnails, so
  Pillow's gcode-trace fallback didn't fire. If Phase 5's
  `.gcode.3mf` wrapper omits thumbnails (cosmetic per PR-0.5-3's
  table), this fallback will run against libslic3r-emitted
  gcode. The G-code primitives it parses are standard but worth
  smoke-testing once a thumbnail-free libslic3r-body fixture
  exists.

- **Auto-slice path** (`bambu_slice`) unexercised. Platecycler
  supports unsliced `.3mf` inputs by shelling out to Bambu Studio
  CLI (`com.bambulab.BambuStudio` flatpak or `bambu-studio` on
  PATH). This is independent of our FFI's behavior — if a user
  hands platecycler an unsliced `.3mf`, BBS does the slice,
  produces its own `.gcode.3mf`, and platecycler processes that.
  Mentioned for completeness; not relevant to portability.

## Implications for downstream phases

- **Phase 5 (Multi-printer + drivers).** The Phase 5
  `.gcode.3mf` wrapper (PR-0.5-3's shopping list) just needs to
  emit BBS-shaped metadata. Once it does, platecycler consumes
  the output without additional adapter work. The minimum
  metadata shape for platecycler compatibility (CLAUDE.md
  invariants):
  - `Metadata/plate_N.gcode` numbered `1..N` with no gaps.
  - `Metadata/plate_N.json` with `bbox_all`, `filament_ids`,
    `bed_type`.
  - `Metadata/slice_info.config` with per-plate `<filament>`
    elements carrying `used_g`/`used_m` and the plate's
    `prediction` metadata.
  - `Metadata/model_settings.config`, `3D/3dmodel.model`,
    `Metadata/project_settings.config` as opaque pass-through
    blobs.

- **Phase 8 (Compose-hook plugin).** No re-implementation needed.
  The plugin can shell out to platecycler with the multi-plate
  job's `.gcode.3mf`, same as a CLI user would today. If we want
  in-process invocation, `platecycler.run_with(...)` is the
  documented library entry point and accepts pre-sliced
  `.gcode.3mf` paths directly. Tighter integration (no
  subprocess) would mean depending on Pillow as a Python runtime
  dep in our app — preferable to avoid; subprocess is fine.

- **Phase 0.5 wrap-up.** All four open Phase 0.5 tickets
  (PR-0.5-1, -2, -3, -5) are now done. PR-0.5-4 (coEnums) was
  already closed inline at `1bb3503`. M0.5 milestone passes.

## Artifacts

- `~/platecycler/` — clone at `ceaece1e4f3251bab7294b9886683ed7c4820358`
  (not checked into n3o-slic3r; CC0-equivalent open-source).
- `~/spike3-bbs/output.platecycler.3mf` — baseline output.
- `~/spike3-bbs/frankenswap/libslic3r-body.gcode.3mf` —
  Frankenstein input.
- `~/spike3-bbs/frankenswap/output.platecycler.3mf` — output
  from libslic3r-body input.
- Neither output is checked in (transient; derived from CC BY-NC
  source via the same license chain noted in
  `examples/spike3/NOTICE.md`).
