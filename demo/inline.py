#!/usr/bin/env python3
"""Inline the demo Vite bundle (index.html + one JS chunk + one CSS) into a
single self-contained HTML — for publishing as an artifact and for one-file
hosting. The build has no code-splitting (single entry), so one <script>/<style>
suffices.

Usage: python3 demo/inline.py
Output: demo/dist/app.single.html  and  demo/dist/app.artifact.html (no doctype)
"""
import base64
import pathlib
import re

root = pathlib.Path(__file__).parent
app = root / "dist" / "app"
html = (app / "index.html").read_text()

# The brand logo is referenced by an absolute url("/brand-icon.svg") in the CSS,
# which 404s at any subpath (the artifact, file://). Inline it as a data URI so
# it works anywhere.
svg = (root.parent / "public" / "brand-icon.svg").read_bytes()
logo_uri = "data:image/svg+xml;base64," + base64.b64encode(svg).decode()

# Inline <script type="module" ... src="./assets/x.js"> -> inline module.
def js_sub(m):
    src = m.group(1).lstrip("./")
    code = (app / src).read_text()
    return f'<script type="module">{code}</script>'

html = re.sub(r'<script type="module"[^>]*src="([^"]+)"[^>]*></script>', js_sub, html)

# Inline <link rel="stylesheet" ... href="./assets/x.css"> -> <style>.
def css_sub(m):
    href = m.group(1).lstrip("./")
    return f"<style>{(app / href).read_text()}</style>"

html = re.sub(r'<link rel="stylesheet"[^>]*href="([^"]+)"[^>]*>', css_sub, html)

# Point the brand logo (and any other absolute /brand-icon.svg ref) at the
# inlined data URI.
html = html.replace("/brand-icon.svg", logo_uri)

# Force a consistent dark theme before the app mounts. Hosted inside the
# artifact iframe, prefers-color-scheme and the wrapper's data-theme can
# disagree and leave the app half-themed (unreadable labels); pinning the app's
# own theme to dark (its natural slicer look) sidesteps that entirely.
force_theme = (
    "<script>try{localStorage.setItem('n3o.theme','dark');"
    "document.documentElement.dataset.theme='dark';}catch(e){}</script>"
)
html = html.replace('<script type="module">', force_theme + '<script type="module">', 1)

out = pathlib.Path(__file__).parent / "dist"
(out / "app.single.html").write_text(html)
# Artifact form: strip the doctype/<html>/<head>/<body> wrapper (claude.ai adds
# its own); keep the <title>, inlined <style>, <div id=root>, and <script>.
body = html
for tag in ["<!doctype html>", "<!DOCTYPE html>"]:
    body = body.replace(tag, "")
body = re.sub(r"</?html[^>]*>", "", body)
body = re.sub(r"</?head[^>]*>", "", body)
body = re.sub(r"</?body[^>]*>", "", body)
(out / "app.artifact.html").write_text(body.strip())
print(f"wrote {out}/app.single.html ({len(html)} bytes) + app.artifact.html")
