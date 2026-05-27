#!/usr/bin/env python3
"""A single 20mm cube whose lower 10mm and upper 10mm are distinct
volumes with different extruder hints — lower → material 1, upper →
material 2. Sibling to `two-cubes-2mat.py` but exercising the
multi-volume single-object shape (BBS `<components>` group with per-
`<part>` extruder metadata) rather than separate build items.

  3D/3dmodel.model                     — two leaf <object>s (lower +
                                         upper meshes) wrapped by a
                                         third <object> that uses
                                         <components> to group them
  Metadata/model_settings.config       — one outer <object> with two
                                         <part> children, each carrying
                                         a BBS-flavor extruder hint
  Metadata/n3o_project_settings.config — placeholder

Geometry: each half is a 20×20×10 mm box. Lower spans local
Z ∈ [-10, 0], upper spans Z ∈ [0, 10]. Build item places the group at
(90, 90, 10) so the combined cube sits on the build plate with
world Z ∈ [0, 20] and the material seam at world Z=10.

Intended use: external-spool smoke print on the Bambu A1 mini —
one material from an AMS slot, the other from the external spool,
with a single in-print swap to verify ams_mapping routing.

Output: `cube-halves-2mat.3mf` next to this script. Regenerate via
  python3 src-tauri/tests/fixtures/3mf/cube-halves-2mat.py
"""
import zipfile
from pathlib import Path
OUT = Path(__file__).with_suffix(".3mf")


def box(xspan, yspan, zspan):
    """Axis-aligned box mesh given (lo, hi) pairs on each axis."""
    x0, x1 = xspan
    y0, y1 = yspan
    z0, z1 = zspan
    faces = [
        ([x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]),  # +Z
        ([x0, y1, z0], [x1, y1, z0], [x1, y0, z0], [x0, y0, z0]),  # -Z
        ([x1, y0, z0], [x1, y1, z0], [x1, y1, z1], [x1, y0, z1]),  # +X
        ([x0, y1, z0], [x0, y0, z0], [x0, y0, z1], [x0, y1, z1]),  # -X
        ([x1, y1, z0], [x0, y1, z0], [x0, y1, z1], [x1, y1, z1]),  # +Y
        ([x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1]),  # -Y
    ]
    v, t = [], []
    for f in faces:
        b = len(v)
        v.extend(f)
        t += [(b, b + 1, b + 2), (b, b + 2, b + 3)]
    return v, t


def mesh_object_xml(oid, v, t):
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


lower_v, lower_t = box((-10.0, 10.0), (-10.0, 10.0), (-10.0, 0.0))
upper_v, upper_t = box((-10.0, 10.0), (-10.0, 10.0), (0.0, 10.0))


model = [
    '<?xml version="1.0" encoding="UTF-8"?>\n'
    '<model unit="millimeter" xml:lang="en-US" '
    'xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02">\n'
]
model.append(' <metadata name="Application">n3o-cube-halves-fixture-builder</metadata>\n')
model.append(' <resources>\n')
# Leaf meshes — referenced by the group object via <components>.
model.append(mesh_object_xml(1, lower_v, lower_t))
model.append(mesh_object_xml(2, upper_v, upper_t))
# Group object: no mesh of its own, just composes 1 + 2.
model.append(
    '  <object id="3" type="model">\n'
    '   <components>\n'
    '    <component objectid="1"/>\n'
    '    <component objectid="2"/>\n'
    '   </components>\n'
    '  </object>\n'
)
model.append(' </resources>\n <build>\n')
# Single build item — the group. Translation places its origin at
# bed center (90, 90) and Z=10 so the bottom face of the lower half
# sits on the build plate.
model.append(
    '  <item objectid="3" '
    'transform="1 0 0 0 1 0 0 0 1 90 90 10" printable="1"/>\n'
)
model.append(' </build>\n</model>\n')


# model_settings: one outer object (the group, id=3) with two parts.
# Part ids match the leaf <object> ids in document order — that's the
# convention our loader expects when it zips parts against the
# flattened ProjectObjects.
ms = ['<?xml version="1.0" encoding="UTF-8"?>\n<config>\n']
ms.append('  <object id="3">\n')
ms.append('    <metadata key="name" value="Cube halves (2-mat)"/>\n')
ms.append('    <part id="1" subtype="normal_part">\n')
ms.append('      <metadata key="name" value="Lower half (M1)"/>\n')
ms.append('      <metadata key="extruder" value="1"/>\n')
ms.append('    </part>\n')
ms.append('    <part id="2" subtype="normal_part">\n')
ms.append('      <metadata key="name" value="Upper half (M2)"/>\n')
ms.append('      <metadata key="extruder" value="2"/>\n')
ms.append('    </part>\n')
ms.append('  </object>\n')
ms.append('  <plate>\n    <metadata key="plater_id" value="1"/>\n')
ms.append(
    '    <model_instance>\n'
    '      <metadata key="object_id" value="3"/>\n'
    '    </model_instance>\n'
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
        '<n3o_project version="1" writer="n3o-cube-halves-fixture-builder">\n'
        '</n3o_project>\n',
    )
print(f"wrote {OUT}")
