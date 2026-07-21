#!/usr/bin/env python3
"""Assemble the self-contained tube-preview page: inline the extracted
toolpath data + the WebGL viewer into demo/shell.html's placeholders.

Usage: python3 demo/build_preview.py
Output: demo/dist/preview.html (single self-contained file).
"""
import pathlib

root = pathlib.Path(__file__).parent
data = (root / "assets" / "toolpaths.json").read_text()
viewer = (root / "viewer.js").read_text()
shell = (root / "shell.html").read_text()

body = shell.replace("__DATA__", data).replace("__VIEWER__", viewer)
dist = root / "dist"
dist.mkdir(exist_ok=True)

# Standalone page for hosting on a plain static server (needs the doctype).
(dist / "preview.html").write_text("<!doctype html><meta charset=utf-8>\n" + body)
# Artifact version — claude.ai wraps it in its own <!doctype>/<head>/<body>.
(dist / "preview.artifact.html").write_text(body)
print(f"wrote {dist}/preview.html + preview.artifact.html ({len(body)} bytes)")
