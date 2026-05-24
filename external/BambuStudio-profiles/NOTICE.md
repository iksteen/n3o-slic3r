# BambuStudio profile snapshot

Vendored subset of the [BambuStudio](https://github.com/bambulab/BambuStudio)
machine / process / filament profiles for the Bambu A1 mini 0.4mm
nozzle PLA workflow. Source of truth for the bundled A1 mini cascade
at `profiles/cascades/bambu-a1-mini-default.toml` (per the safety
investigation flagged before the first real-print smoke).

## Why vendored and not submoduled

A git submodule against BambuStudio would pull in ~500 MB of C++
source we don't need. Our concern is the JSON profile tree under
`resources/profiles/BBL/`, which is small (~50 KB for the A1 mini
chain). Vendoring the leaf set + inheritance parents keeps the
build hermetic and lets CI gate on upstream drift via a small
diff check, without paying for the full upstream repo.

## Upstream snapshot

| Repo | `bambulab/BambuStudio` |
| Branch | `master` |
| HEAD SHA at snapshot | `e150b502b3d2` |
| Snapshot date | 2026-05-21 |

## File set

The 11 JSON files cover the full inheritance closure for the A1
mini 0.4mm PLA workflow:

- `BBL/machine/Bambu Lab A1 mini 0.4 nozzle.json` — leaf machine
- `BBL/machine/Bambu Lab A1 mini 0.4 nozzle template machine_start_gcode.json`
- `BBL/machine/Bambu Lab A1 mini 0.4 nozzle template machine_end_gcode.json`
- `BBL/machine/Bambu Lab A1 mini 0.4 nozzle template change_filament_gcode.json`
- `BBL/machine/Bambu Lab A1 mini 0.4 nozzle template layer_change_gcode.json`
- `BBL/machine/Bambu Lab A1 mini 0.4 nozzle template time_lapse_gcode.json`
- `BBL/machine/fdm_bbl_3dp_001_common.json` — machine inheritance parent
- `BBL/machine/fdm_machine_common.json` — root machine parent
- `BBL/process/0.20mm Standard @BBL A1M.json` — leaf process
- `BBL/process/fdm_process_single_0.20.json`
- `BBL/process/fdm_process_single_common.json`
- `BBL/process/fdm_process_common.json` — root process parent
- `BBL/filament/Bambu PLA Basic @BBL A1M.json` — leaf filament
- `BBL/filament/Bambu PLA Basic @base.json`
- `BBL/filament/fdm_filament_pla.json`
- `BBL/filament/fdm_filament_common.json` — root filament parent

## Why these specific files

The A1 mini machine JSON references five sibling **template files**
via its `include:` array — BambuStudio splits G-code macros out
into per-template files so multiple printer variants can share
them. The "machine" subdir snapshot includes the leaf + the five
templates.

Each leaf profile (machine/process/filament) chains up to a root
common ancestor via the `inherits:` field. The vendored set
captures every node in those three chains so the converter can
resolve any field without further network calls.

## Drift tracking

Bump the SHA above + re-vendor when the converter's drift check
fires in CI. The check is a small `diff -r` between our local
snapshot and a freshly-fetched HEAD of the upstream paths above —
runs on every PR that touches `profiles/cascades/` or
`scripts/spikes/`. If upstream changes, regen the cascade
(`scripts/spikes/convert_bbs_profile.py`), review the diff vs the
previous cascade, and commit both the new vendor snapshot + the
new cascade.

## License

BambuStudio is licensed AGPL-3.0-or-later, same as n3o-slic3r —
see `LICENSE` in this directory (verbatim copy of the upstream
file at the snapshot SHA above).

Profile JSONs are content data, not source code; they are
nonetheless covered by the upstream license. The AGPL's
network-distribution clause is satisfied for the bundled
cascade in our `profiles/` directory because it ships with
attribution and the project source is itself AGPL.
