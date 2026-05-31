# 3MF test fixtures

Hand-authored 3MF files (with generator scripts checked in) for
loader / scene / slice integration tests. Each `<name>.3mf` is
produced by running its sibling `<name>.py` — the generator is the
source of truth, the binary is checked in only so test runs don't
need a Python interpreter.

## Files

| Fixture | Source | Use |
|---|---|---|
| `two-cubes-2mat.3mf` | `two-cubes-2mat.py` | Two 20mm cubes, each with a BBS-flavor `<metadata key="extruder">` hint (cube A → material 1, cube B → material 2). Exercises the per-object extruder-hint → material→slot auto-bind path on multi-material printers (Snapmaker U1). |
| `four-cubes-4mat.3mf` | `four-cubes-4mat.py` | Four 20mm cubes in a 2×2 grid, one per material (M1..M4). Sized for the Snapmaker U1's 4-toolhead toolchanger — a single print exercises every tool-change pair (PR-7b-9 smoke). |
| `cube-halves-2mat.3mf` | `cube-halves-2mat.py` | Single 20mm cube split into two volumes — lower half → M1, upper half → M2 — grouped via BBS-style `<components>` with per-`<part>` extruder hints. Intended for the Bambu A1 mini external-spool smoke (one material from AMS, the other from external spool, single in-print swap). |
| `case-bambu-studio.3mf` | *(none — real export)* | A **real Bambu Studio project** (designed by the project lead), not script-generated. Two objects (body + logo) grouped via `<components>` on an A1 mini, plus non-default settings: tree supports on build plate only, a top-Z-distance tweak, a support/object first-layer-gap tweak, and a `layer_config_ranges.xml` height-range modifier. The real-world fixture for OrcaSlicer/Bambu **project import** (PR-9-6) — geometry + `project_settings.config` settings. Bambu Studio and OrcaSlicer share this format. |

## Regenerating

```bash
python3 src-tauri/tests/fixtures/3mf/<fixture>.py
```

Scripts have no external dependencies — stdlib `zipfile` +
`pathlib` only.

## Adding a new fixture

1. Author the generator as `<name>.py` (write any geometry,
   per-object `extruder` metadata, plate stanza, etc.).
2. Run it once to produce `<name>.3mf`.
3. Commit both. Add a row to the table above.
4. Document non-obvious shape choices inside the script's
   docstring — readers shouldn't have to rerun the script + diff
   the zip to understand what edge case the fixture targets.
