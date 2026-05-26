#!/usr/bin/env python3
"""Four 20mm cubes in a 2×2 grid, each on its own extruder hint
(cube A → material 1, B → 2, C → 3, D → 4). Hand-authored 3mf
matching the shape n3o-slic3r's own writer produces — sibling to
`two-cubes-2mat.py`, sized for the Snapmaker U1's 4-toolhead
toolchanger so a single print exercises every tool change pair.

  3D/3dmodel.model                     — geometry + build items
  Metadata/model_settings.config       — BBS-flavor per-object +
                                         per-part extruder hints
  Metadata/n3o_project_settings.config — placeholder

Both object-level and part-level extruder metadata are emitted —
real BBS/Orca exports include both, so the fixture mirrors that
even though our parser only reads the part-level value today.

Layout: 2×2 grid centered around (120, 90) on the bed, 60 mm
spacing between cube centers. Z = 10 places each cube's bottom
face on the build plate (cubes are origin-centered, so Z=10 +
cube extents [-10, +10] = [0, 20] in world Z).

Output: `four-cubes-4mat.3mf` next to this script. Regenerate via
  python3 src-tauri/tests/fixtures/3mf/four-cubes-4mat.py
"""
import zipfile
from pathlib import Path
OUT = Path(__file__).with_suffix(".3mf")


def cube(size):
    h = size / 2
    faces = [
        ([-h, -h,  h], [ h, -h,  h], [ h,  h,  h], [-h,  h,  h]),
        ([-h,  h, -h], [ h,  h, -h], [ h, -h, -h], [-h, -h, -h]),
        ([ h, -h, -h], [ h,  h, -h], [ h,  h,  h], [ h, -h,  h]),
        ([-h,  h, -h], [-h, -h, -h], [-h, -h,  h], [-h,  h,  h]),
        ([ h,  h, -h], [-h,  h, -h], [-h,  h,  h], [ h,  h,  h]),
        ([-h, -h, -h], [ h, -h, -h], [ h, -h,  h], [-h, -h,  h]),
    ]
    v, t = [], []
    for f in faces:
        b = len(v)
        v.extend(f)
        t += [(b, b + 1, b + 2), (b, b + 2, b + 3)]
    return v, t


def obj_xml(oid, v, t):
    vx = "".join(f'<vertex x="{x}" y="{y}" z="{z}"/>' for x, y, z in v)
    tx = "".join(f'<triangle v1="{a}" v2="{b}" v3="{c}"/>' for a, b, c in t)
    return (
        f'  <object id="{oid}" type="model">\n'
        f'   <mesh>\n'
        f'    <vertices>{vx}</vertices>\n'
        f'    <triangles>{tx}</triangles>\n'
        f'   </mesh>\n'
        f'  </object>\n'
    )


v, t = cube(20.0)

# Build items: 4 cubes in a 2×2 grid, 60 mm apart, all on plate 1.
# (oid, x, y) — z fixed at 10 so the cube sits on the bed.
PLACEMENTS = [
    (1,  90.0,  60.0),  # A → M1 → T0
    (2, 150.0,  60.0),  # B → M2 → T1
    (3,  90.0, 120.0),  # C → M3 → T2
    (4, 150.0, 120.0),  # D → M4 → T3
]

# Per-object metadata: name + extruder hint.
PARTS = [
    (1, "Cube A (T0)", 1),
    (2, "Cube B (T1)", 2),
    (3, "Cube C (T2)", 3),
    (4, "Cube D (T3)", 4),
]


model = [
    '<?xml version="1.0" encoding="UTF-8"?>\n'
    '<model unit="millimeter" xml:lang="en-US" '
    'xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02">\n'
]
model.append(' <metadata name="Application">n3o-4cube-fixture-builder</metadata>\n')
model.append(' <resources>\n')
for oid, _, _ in PLACEMENTS:
    model.append(obj_xml(oid, v, t))
model.append(' </resources>\n <build>\n')
for oid, x, y in PLACEMENTS:
    model.append(
        f'  <item objectid="{oid}" '
        f'transform="1 0 0 0 1 0 0 0 1 {x} {y} 10" printable="1"/>\n'
    )
model.append(' </build>\n</model>\n')


ms = ['<?xml version="1.0" encoding="UTF-8"?>\n<config>\n']
for oid, name, ext in PARTS:
    ms.append(f'  <object id="{oid}">\n')
    ms.append(f'    <metadata key="name" value="{name}"/>\n')
    ms.append(f'    <metadata key="extruder" value="{ext}"/>\n')
    ms.append('    <part id="1" subtype="normal_part">\n')
    ms.append(f'      <metadata key="name" value="{name}"/>\n')
    ms.append(f'      <metadata key="extruder" value="{ext}"/>\n')
    ms.append('    </part>\n  </object>\n')
ms.append('  <plate>\n    <metadata key="plater_id" value="1"/>\n')
for oid, _, _ in PLACEMENTS:
    ms.append(
        f'    <model_instance>\n'
        f'      <metadata key="object_id" value="{oid}"/>\n'
        f'    </model_instance>\n'
    )
ms.append('  </plate>\n</config>\n')


with zipfile.ZipFile(OUT, "w", compression=zipfile.ZIP_DEFLATED) as z:
    z.writestr(
        "[Content_Types].xml",
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
        '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">\n'
        ' <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>\n'
        ' <Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/>\n'
        ' <Default Extension="config" ContentType="application/vnd.ms-package.3dmanufacturing-config+xml"/>\n'
        '</Types>\n',
    )
    z.writestr(
        "_rels/.rels",
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
        '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">\n'
        ' <Relationship Target="/3D/3dmodel.model" Id="rel-1" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/>\n'
        '</Relationships>\n',
    )
    z.writestr("3D/3dmodel.model", "".join(model))
    z.writestr("Metadata/model_settings.config", "".join(ms))
    z.writestr(
        "Metadata/n3o_project_settings.config",
        '<?xml version="1.0" encoding="UTF-8"?>\n'
        '<n3o_project version="1" writer="n3o-4cube-fixture-builder">\n'
        '</n3o_project>\n',
    )
print(f"wrote {OUT}")
