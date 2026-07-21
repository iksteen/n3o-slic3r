#!/usr/bin/env python3
"""Parse a sliced (OrcaSlicer/Bambu-flavor) G-code into compact per-layer
toolpath polylines for the web demo's Three.js tube preview.

Output JSON:
  { features: [name, ...],                      # legend, index = feature id
    bbox: [minx,miny,minz, maxx,maxy,maxz],
    layers: [ { z, paths: [ [featIdx, x0,y0, x1,y1, ...], ... ] }, ... ] }

Each path is one continuous extrusion run at that layer's Z (a travel/retract
breaks it). Coordinates are rounded to 3 decimals to shrink the file. Extrusion
is detected via relative E (Bambu/Orca emit M83): a G1 with X/Y movement and
E > 0 is extruding; anything else just moves the head (breaks the run).

Usage: python3 demo/extract_toolpaths.py demo/assets/sample.gcode demo/assets/toolpaths.json
"""
import json
import re
import sys

# The prime/purge tower isn't part of the model — skip it so the demo frames
# and centers on the actual print, not a big box off to the side.
SKIP_FEATURES = {"Prime tower"}

FEAT_RE = re.compile(r"^; FEATURE:\s*(.+?)\s*$")
Z_RE = re.compile(r"^; Z_HEIGHT:\s*([0-9.]+)")
NUM = r"([-0-9.]+)"
G1_RE = re.compile(r"^G[01] ")


def field(line, letter):
    m = re.search(letter + NUM, line)
    return float(m.group(1)) if m else None


def main():
    src, dst = sys.argv[1], sys.argv[2]
    features, feat_idx = [], {}

    def fid(name):
        if name not in feat_idx:
            feat_idx[name] = len(features)
            features.append(name)
        return feat_idx[name]

    layers = []           # [{z, paths:[[fid, x0,y0, ...]]}]
    cur = None            # current layer dict
    x = y = z = 0.0
    feature = "Custom"
    run = None            # current extrusion polyline (list starting with fid)

    def flush_run():
        nonlocal run
        if run is not None and len(run) > 3:   # need >=2 points
            cur["paths"].append(run)
        run = None

    for line in open(src, "r", errors="ignore"):
        zm = Z_RE.match(line)
        if line.startswith("; CHANGE_LAYER"):
            flush_run()
            continue
        if zm:
            flush_run()
            z = float(zm.group(1))
            cur = {"z": z, "paths": []}
            layers.append(cur)
            continue
        fm = FEAT_RE.match(line)
        if fm:
            flush_run()
            feature = fm.group(1)
            continue
        if not G1_RE.match(line) or cur is None:
            continue
        nx, ny = field(line, "X"), field(line, "Y")
        nz = field(line, "Z")
        e = field(line, "E")
        if nz is not None:
            z = nz
        moving = (nx is not None and nx != x) or (ny is not None and ny != y)
        px, py = x, y
        if nx is not None:
            x = nx
        if ny is not None:
            y = ny
        if moving and e is not None and e > 0.0 and feature not in SKIP_FEATURES:
            # Extruding segment px,py -> x,y.
            if run is None:
                run = [fid(feature), round(px, 3), round(py, 3)]
            run.append(round(x, 3))
            run.append(round(y, 3))
        else:
            flush_run()   # travel / retract / pure-Z: break the run

    # bbox over all path points
    lo = [1e9, 1e9, 1e9]
    hi = [-1e9, -1e9, -1e9]
    for ly in layers:
        for p in ly["paths"]:
            for i in range(1, len(p), 2):
                px, py = p[i], p[i + 1]
                lo[0], hi[0] = min(lo[0], px), max(hi[0], px)
                lo[1], hi[1] = min(lo[1], py), max(hi[1], py)
        lo[2], hi[2] = min(lo[2], ly["z"]), max(hi[2], ly["z"])

    out = {
        "features": features,
        "bbox": [round(v, 3) for v in lo + hi],
        "layers": layers,
    }
    with open(dst, "w") as f:
        json.dump(out, f, separators=(",", ":"))
    npaths = sum(len(l["paths"]) for l in layers)
    print(f"wrote {dst}: {len(layers)} layers, {npaths} paths, features={features}")


if __name__ == "__main__":
    main()
