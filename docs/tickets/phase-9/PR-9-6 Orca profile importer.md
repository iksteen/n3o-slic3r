# PR-9-6 — OrcaSlicer profile importer (one-time migration)

Status: ⬜ open. **Phase cut candidate** (scope decision 2; saves ~4
days).

**Scope.** A one-time migration tool that reads OrcaSlicer `.json`
profile bundles and maps them to our cascade layers — so a user coming
from OrcaSlicer can bring their tuned profiles instead of re-authoring.
This is **not a runtime dependency**: the app ships first-class
profiles for both MVP printers and works fully without ever importing
(PRD §5, standalone at runtime). Import is an adoption convenience.

**Acceptance criteria.**

- Reads OrcaSlicer `.json` profile bundles (machine / process /
  filament) and maps each to the corresponding **cascade layer** in our
  rule model (`docs/profiles.md` is the design of record for the target
  shape). The mapping reuses the existing `scripts/import_*` converters'
  field knowledge where it overlaps rather than re-deriving it.
- Produces cascade fragments under the user library (or a staging dir),
  not the bundled `resources/profiles/` tree — imported profiles are
  user data.
- **Lossy mapping is surfaced, not silent** — settings with no cascade
  home, or values that don't translate, are reported to the user
  (a summary of what imported, what was dropped, and why), not dropped
  quietly (memory: `no_silent_caps` in spirit).
- Round-trips at least one real A1 mini and one real U1 profile bundle
  from upstream OrcaSlicer/Snapmaker Orca into usable layers, verified
  by slicing with an imported profile and checking the output.
- The tool is invocable as a one-time action (CLI subcommand or a
  menu item), clearly framed as migration — not a live "OrcaSlicer
  integration".

**Effort.** ~4 days.

**Dependencies.** PR-9-5 (target project/profile shape settled);
`docs/profiles.md` (cascade design); the `scripts/import_*` converters
(field reference). No hardware dependency.

**Out of scope.**

- Live / continuous sync with an OrcaSlicer install — this is a
  one-time read, never a runtime link (PRD §5).
- Importing PrusaSlicer / Cura / other formats — OrcaSlicer `.json`
  only for the MVP.
- Importing OrcaSlicer *projects* (3MF) — profiles only.

**If cut:** document in the release notes (PR-9-7) that existing
OrcaSlicer users re-author or hand-edit cascade fragments for the MVP,
and move the importer to the post-MVP list.
