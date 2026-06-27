#!/usr/bin/env python3
"""A single 20mm cube exported as two foreign 3MFs with identical geometry:

  cube-plain.3mf     — no per-object config override
  cube-override.3mf  — a per-object `layer_height = 0.25` override stored at the
                       <object> level of Metadata/model_settings.config, exactly
                       how a foreign Orca export records an object-scoped setting

The import-override end-to-end test (`phase_s_smoke`) imports both and asserts
the override populates `scene.object_overrides` and visibly changes the slice
(fewer layers at 0.25mm than the default ~0.2mm). Same geometry in both so the
layer-count comparison isolates the override.

Output: the two .3mf files next to this script. Regenerate via
  python3 src-tauri/tests/fixtures/3mf/cube-override-pair.py
"""
import zipfile
from pathlib import Path

HERE = Path(__file__).parent


def cube(size):
    h = size / 2
    faces = [
        ([-h, -h, h], [h, -h, h], [h, h, h], [-h, h, h]),
        ([-h, h, -h], [h, h, -h], [h, -h, -h], [-h, -h, -h]),
        ([h, -h, -h], [h, h, -h], [h, h, h], [h, -h, h]),
        ([-h, h, -h], [-h, -h, -h], [-h, -h, h], [-h, h, h]),
        ([h, h, -h], [-h, h, -h], [-h, h, h], [h, h, h]),
        ([-h, -h, -h], [h, -h, -h], [h, -h, h], [-h, -h, h]),
    ]
    v, t = [], []
    for f in faces:
        b = len(v)
        v.extend(f)
        t += [(b, b + 1, b + 2), (b, b + 2, b + 3)]
    return v, t


v, t = cube(20.0)
vx = "".join(f'<vertex x="{x}" y="{y}" z="{z}"/>' for x, y, z in v)
tx = "".join(f'<triangle v1="{a}" v2="{b}" v3="{c}"/>' for a, b, c in t)
model = (
    '<?xml version="1.0" encoding="UTF-8"?>\n'
    '<model unit="millimeter" xml:lang="en-US" '
    'xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02">\n'
    ' <metadata name="Application">n3o-cube-override-fixture-builder</metadata>\n'
    ' <resources>\n'
    f'  <object id="1" type="model">\n   <mesh>\n    <vertices>{vx}</vertices>\n'
    f'    <triangles>{tx}</triangles>\n   </mesh>\n  </object>\n'
    ' </resources>\n <build>\n'
    # Centered on the A1 mini's 180x180 bed; z=10 seats the 20mm cube on z=0.
    '  <item objectid="1" transform="1 0 0 0 1 0 0 0 1 90 90 10" printable="1"/>\n'
    ' </build>\n</model>\n'
)


def model_settings(override):
    ms = ['<?xml version="1.0" encoding="UTF-8"?>\n<config>\n']
    ms.append('  <object id="1">\n')
    ms.append('    <metadata key="name" value="Cube"/>\n')
    ms.append('    <metadata key="extruder" value="1"/>\n')
    if override:
        # Object-scoped config override — the reader folds every non-identity
        # <object>-level metadata key into the object's overrides.
        ms.append('    <metadata key="layer_height" value="0.25"/>\n')
    ms.append('    <part id="1" subtype="normal_part">\n')
    ms.append('      <metadata key="name" value="Cube"/>\n')
    ms.append('      <metadata key="extruder" value="1"/>\n')
    ms.append('    </part>\n  </object>\n')
    ms.append('  <plate>\n    <metadata key="plater_id" value="1"/>\n')
    ms.append('    <model_instance>\n      <metadata key="object_id" value="1"/>\n    </model_instance>\n')
    ms.append('  </plate>\n</config>\n')
    return "".join(ms)


CONTENT_TYPES = (
    '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
    '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">\n'
    ' <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>\n'
    ' <Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/>\n'
    ' <Default Extension="config" ContentType="application/vnd.ms-package.3dmanufacturing-config+xml"/>\n'
    '</Types>\n'
)
RELS = (
    '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
    '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">\n'
    ' <Relationship Target="/3D/3dmodel.model" Id="rel-1" '
    'Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/>\n'
    '</Relationships>\n'
)


def write(name, override):
    out = HERE / name
    with zipfile.ZipFile(out, "w", compression=zipfile.ZIP_DEFLATED) as z:
        z.writestr("[Content_Types].xml", CONTENT_TYPES)
        z.writestr("_rels/.rels", RELS)
        z.writestr("3D/3dmodel.model", model)
        z.writestr("Metadata/model_settings.config", model_settings(override))
    print(f"wrote {out}")


write("cube-plain.3mf", override=False)
write("cube-override.3mf", override=True)
