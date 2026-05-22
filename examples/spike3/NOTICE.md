# Attribution for `fourcolor.3mf`

This directory contains a third-party 3D model used as a test fixture
for spike PR-0.5-3 (Bambu A1 mini AMS slice). It is **not** part of
the n3o-slic3r software and is **not** redistributed as part of the
shipped product — it lives in `examples/` for development use only.

## Model

**4 Colors Benchy AMS Test (v2)** by *jansonne* on MakerWorld.

- Source: <https://makerworld.com/en/models/2494791-4-colors-benchy-ams-test>
- Designer: jansonne (MakerWorld user 865450034)
- License: **Creative Commons Attribution-NonCommercial 4.0 International (CC BY-NC 4.0)**
- License text: <https://creativecommons.org/licenses/by-nc/4.0/>

The model itself is derived from a CC0 work:

- **Original 3DBenchy Public Domain CAD STEP File** by *Stemfie3D*
  on MakerWorld, license **CC0**
- Source: <https://makerworld.com/models/1272656-original-3dbenchy-public-domain-cad-step-file>
- Designer profile: <https://makerworld.com/@Stemfie3D>

Attribution data was extracted from the 3MF's own metadata block
(`3D/3dmodel.model`) — the file embeds the Designer, License, and
upstream CopyRight fields and would have to be modified to remove
them. See `unzip -p fourcolor.3mf 3D/3dmodel.model | grep -E
'Designer|License|CopyRight'` for verification.

## Why CC BY-NC is OK for this use

CC BY-NC permits use, copying, and distribution for non-commercial
purposes with attribution. n3o-slic3r is open-source software
(AGPL-3.0-or-later) developed without commercial intent; using
this 3MF as a developer test fixture, with clear attribution and
without redistribution in any shipped product, falls inside the
license's permitted-uses scope.

If the project ever ships a packaged release (Phase 9, flatpak)
that bundles example assets, this 3MF **must not** be included.
Replace it with a CC0 or CC BY (without -NC) equivalent at that
point, or omit example assets from the release entirely.
