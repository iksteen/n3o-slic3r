# PR-9-5 — project file format `.3mf` finalized

Status: 🟡 done bar a pre-MVP format review (2026-05-31). The File menu
is wired and the format is documented; the custom format itself gets a
deliberate review before MVP sign-off (project lead's call).

> **Outcome.** The audit found the backend was further along than this
> ticket assumed — `project_save`/`project_save_as`/`project_load`, the
> `.3mf` writer/reader with the `Metadata/n3o_project.json` entry,
> `FORMAT_VERSION="1"` + schema-mismatch reject, credentials exclusion,
> and round-trip tests all already existed. The two real gaps, now
> closed:
> - **File menu UI** — the commands had no user-facing entry point. The
>   project dropdown now bears the project's filename (`Untitled.3mf`
>   when unsaved) and has **Open project…**, **Save project**, **Save
>   project as…**, wired to native file dialogs +
>   `project_load`/`project_save`/`project_save_as`. "Save project" on an
>   unsaved project falls through to Save As; Save As updates the
>   source path and the label (a new `project:saved` session-event
>   subscription drives the refetch). (`src/plugins/TopBarPluginMenus.tsx`,
>   `src/App.tsx`, `src/project/projectFile.ts`,
>   `src/project/useProjectSession.ts`.)
> - **Format documentation** — `docs/3mf-format-notes.md` gained an
>   n3o-project-container section (the `ProjectFile` schema, the mesh
>   round-trip, credentials exclusion, versioning / no-forward-compat),
>   marked **PROVISIONAL** pending the pre-MVP review.
>
> **Deferred (non-blocking):** save/open failures currently
> `console.error` (matching the codebase's printer-create handler); a
> user-facing error surface is a later polish. And the **format review**
> before MVP — see the doc's PROVISIONAL banner and the
> `project_3mf_format_provisional` memory.

**Scope.** Finalize the project file format per **FR-MP-4**: the `.3mf`
extension is the project's on-disk container, settled and documented so
release (and the importer, PR-9-6) build against a fixed shape. The
machinery exists (3MF I/O, autosave, the multi-printer project model);
this ticket *closes* the format rather than building it.

**Acceptance criteria.**

- The project save/open path uses the **`.3mf`** extension and a
  documented internal layout: objects + transforms, per-plate printer
  assignment, the cascade overrides, slot/filament bindings — whatever
  the project model persists today, written down as the format of
  record (FR-MP-4). `docs/3mf-format-notes.md` is the place if it
  isn't already.
- **Credentials are excluded** from the project `.3mf` — they live in
  the per-printer user-library instance `.toml` (memory:
  `no_credentials_in_project_file`). A project file is shareable; a
  round-trip through save→open on another install must not leak access
  codes and must rebind cleanly to that install's printers.
- A **round-trip test**: save a multi-printer, multi-plate project,
  reopen it, and confirm object transforms, plate→printer assignments,
  and overrides survive byte-for-meaning (not necessarily byte-identical).
- The format note states what is **versioned** and how an older project
  opens in a newer app (at minimum: a version marker + a "we don't
  promise forward-compat in the MVP" statement, if that's the call).

**Effort.** ~0.5 day (mostly documenting + a round-trip test; the I/O
exists).

**Dependencies.** Phase 3 (3MF I/O) + Phase 5 (multi-printer project
model) — done. Feeds PR-9-6 (importer targets this format).

**Out of scope.**

- A migration/upgrade tool for older project files — there is no prior
  released format to migrate from.
- The OrcaSlicer `.json` *profile* import — that's PR-9-6 and is about
  profiles, not project files.
