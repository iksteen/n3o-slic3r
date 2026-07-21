#!/usr/bin/env python3
"""Split a 3MF's build parts into per-part meshes for the demo's model viewer.

Keeps each build object separate (rather than fusing them) so the viewport can
colour them by their assigned material — here: the OrangeCon case body vs. the
embossed logo insert.

Parses the .model geometry + `model_settings.config` (part names + extruders)
directly, so it needs no libslic3r FFI. The build item's transform is applied so
world coordinates match load_3mf.

Usage: python3 demo/split_parts.py demo/assets/model.3mf demo/assets/parts.json
"""
import json
import sys
import xml.etree.ElementTree as ET
import zipfile


def strip_ns(root):
    for el in root.iter():
        el.tag = el.tag.rsplit("}", 1)[-1]
    return root


def parse_transform(s):
    """3MF 4x3 row-major transform ("m00..m22 tx ty tz") → (basis rows, translation)."""
    v = [float(x) for x in s.split()]
    return v[0:9], v[9:12]


def apply(basis, t, x, y, z):
    m = basis
    return (
        m[0] * x + m[3] * y + m[6] * z + t[0],
        m[1] * x + m[4] * y + m[7] * z + t[1],
        m[2] * x + m[5] * y + m[8] * z + t[2],
    )


def main():
    src, dst = sys.argv[1], sys.argv[2]
    z = zipfile.ZipFile(src)
    model = strip_ns(ET.fromstring(z.read("3D/3dmodel.model")))
    cfg = strip_ns(ET.fromstring(z.read("Metadata/model_settings.config")))

    objects = {o.get("id"): o for o in model.iter("object")}

    # The single build item: which assembly, and its world transform.
    item = model.find(".//build/item")
    root_id = item.get("objectid")
    _, item_t = parse_transform(item.get("transform", "1 0 0 0 1 0 0 0 1 0 0 0"))

    # Part metadata (name + extruder) keyed by part id == referenced object id.
    part_meta = {}
    for cobj in cfg.iter("object"):
        obj_ext = "1"
        for md in cobj.findall("metadata"):
            if md.get("key") == "extruder":
                obj_ext = md.get("value")
        for part in cobj.findall("part"):
            name, ext = f"Part {part.get('id')}", obj_ext
            for md in part.findall("metadata"):
                if md.get("key") == "name":
                    name = md.get("value")
                if md.get("key") == "extruder":
                    ext = md.get("value")
            part_meta[part.get("id")] = (name, int(ext))

    # Assembly components: each references a mesh object + its own transform,
    # composed with the item transform. (Here components are identity.)
    parts = []
    for comp in objects[root_id].findall(".//component"):
        oid = comp.get("objectid")
        cbasis, ct = parse_transform(comp.get("transform", "1 0 0 0 1 0 0 0 1 0 0 0"))
        obj = objects[oid]
        verts = obj.findall(".//vertex")
        positions, lo, hi = [], [1e9] * 3, [-1e9] * 3
        world = []
        for vtx in verts:
            x, y, z_ = float(vtx.get("x")), float(vtx.get("y")), float(vtx.get("z"))
            # component transform, then item transform.
            cx, cy, cz = apply(cbasis, ct, x, y, z_)
            wx, wy, wz = apply([1, 0, 0, 0, 1, 0, 0, 0, 1], item_t, cx, cy, cz)
            world.append((wx, wy, wz))
            for k, val in enumerate((wx, wy, wz)):
                lo[k] = min(lo[k], val)
                hi[k] = max(hi[k], val)
        for wx, wy, wz in world:
            positions += [round(wx, 3), round(wy, 3), round(wz, 3)]
        indices = []
        for tri in obj.findall(".//triangle"):
            indices += [int(tri.get("v1")), int(tri.get("v2")), int(tri.get("v3"))]
        name, ext = part_meta.get(oid, (f"Part {oid}", 1))
        parts.append({
            "name": name,
            "extruder": ext,
            "bbox": [round(v, 3) for v in (lo + hi)],
            "positions": positions,
            "indices": indices,
        })
        print(f"  part {oid}: {name!r} extruder {ext} — {len(verts)} verts, {len(indices)//3} tris")

    with open(dst, "w") as f:
        json.dump(parts, f, separators=(",", ":"))
    print(f"wrote {dst} ({len(parts)} parts)")


if __name__ == "__main__":
    main()
