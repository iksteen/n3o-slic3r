"""Orca-side typo keys, folded to canonical at profile-import time.

A handful of upstream OrcaSlicer profiles misspell option names. libslic3r
silently drops unknown keys, so in OrcaSlicer the correctly-spelled sibling
(or the engine default) is what actually takes effect — the typo never does.
The importers fold each typo onto its canonical spelling here so the
generated fragments only ever carry canonical keys and the runtime cascade
never has to remap (remapping per-filament at slice time once zeroed a
sibling filament's first-layer temp). Canonical wins on collision, matching
what Orca uses; a lone typo is recovered to the canonical key.
"""

from __future__ import annotations

from typing import Any

TYPO_REMAP = {
    "detraction_speed": "deretraction_speed",
    "inital_layer_height": "initial_layer_height",
    "nozzle_temperature_intial_layer": "nozzle_temperature_initial_layer",
    "tree_support_bramch_diameter_angle": "tree_support_branch_diameter_angle",
}


def fold_typo_keys(doc: dict[str, Any]) -> None:
    """Rename Orca-side typo keys to canonical in place; canonical wins."""
    for typo, canonical in TYPO_REMAP.items():
        if typo in doc:
            value = doc.pop(typo)
            doc.setdefault(canonical, value)
