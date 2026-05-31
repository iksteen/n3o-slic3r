# Phase 9 — tickets

Phase 9 (~2 person-weeks in the plan) is the **release phase** — the
last phase before the MVP candidate. Goal (`docs/Execution_Plan.md`
§11):

> Linux flatpak build, basic onboarding, release-readiness. Windows
> and macOS native builds are post-MVP.

This phase ships no new slicing capability. It turns the working app
into something an external person can install on a clean Linux box and
run to completion with no other slicer present — the PRD's "standalone
at runtime" principle (PRD §5) made literal by the **independence
audit** (PR-9-8), which is the phase's real exit gate.

Individual tickets live one-per-file in `phase-9/`. This file is the
index plus phase-level status, sequencing, and scope decisions.

## What already exists (don't rebuild)

- **The Tauri bundle builds.** `packaging/arch/` holds a working Arch
  `PKGBUILD` + a built `.pkg.tar.zst` and `.deb` staging from a prior
  packaging pass. The flatpak ticket (PR-9-2) starts from a known-good
  Tauri build that produces a desktop binary + bundled
  `libslic3r_ffi.so` + the `resources/` tree — not from zero.
- **The resources tree is consolidated.** `resources/{profiles,plugins}`
  ship via `tauri.conf.json` `bundle.resources`; dev resolves them
  through `N3O_SLIC3R_RESOURCES_ROOT`. The flatpak just needs the
  bundle's resource dir wired, no per-tree env in production.
- **Onboarding has a backend.** The add-printer wizard, the empty-state
  first-launch path, and the user printer-instance library
  (`core::printer::instance_storage`) already exist (Phase 5/7). PR-9-4
  is the guided first-run *flow* over that backend, not new storage.

## Scope decisions

1. **The slice-path correctness gate (PR-9-1) is pulled into Phase 9
   as a release blocker.** It is not a §11 deliverable — it traces to
   the cascade phase (Phase 1) — but CLAUDE.md flags it OPEN: a
   2026-05-30 U1 slice emitted baseline `hot_plate_temp=60` instead of
   the `Snapmaker U1` rule's `55`, which suggests the live slice path
   may not route through the resolver + adapter. MVP success criterion
   #1 ("prints complete without manual G-code editing") and the
   independence audit both depend on slices being correct, so this is
   resolved *before* the audit, not after. If you'd rather track it as
   a Phase 1 follow-up instead, it can move — but it stays a gate on
   PR-9-8 either way.

2. **OrcaSlicer profile importer (PR-9-6) is the phase's cut
   candidate.** Per §11 it saves ~4 days if cut. The app ships
   first-class profiles for both MVP printers, so the importer is an
   adoption convenience, not a workflow dependency. Built if schedule
   allows; first against the chopping block if it doesn't.

3. **WSL2 validation is folded in, not its own ticket.** §11 lists it
   best-effort. It rides along in the flatpak ticket (PR-9-2) as a
   "does it run under WSLg" smoke and in the audit (PR-9-8) as a
   documented known-limitation, not a blocker.

4. **Self-hosted distribution for MVP, Flathub post-MVP.** Per §11 the
   MVP ships a self-hosted `.flatpakref` + repo (faster iteration, no
   review wait). Flathub submission moves to the post-MVP list. PR-9-3
   owns the repo + ref, not a Flathub PR.

## Sequencing

`9-1` (correctness) and `9-2` (flatpak) are the two long poles and both
start immediately — they're independent. The rest:

- **9-3** (distribution) needs a flatpak artifact → after **9-2**.
- **9-4** (onboarding flow) — frontend over existing backend; independent.
- **9-5** (.3mf format finalize) — small; independent.
- **9-6** (profile importer) — standalone tool; independent (cut candidate).
- **9-7** (docs: getting-started + troubleshooting + release notes)
  needs the **9-2** install path and the **9-4** onboarding flow to
  document.
- **9-8** (independence audit) is **last** — the exit gate. Needs the
  flatpak (9-2/9-3), onboarding (9-4), correct slices (9-1), and the
  getting-started doc (9-7).

## Status by deliverable

| Deliverable | Status | Ticket |
|-------------|--------|--------|
| Slice-path / cascade-resolver correctness gate (release blocker) | ✅ done | [PR-9-1](phase-9/PR-9-1%20Slice-path%20correctness%20gate.md) |
| Linux flatpak build (manifest, runtime, bundled libslic3r + webview, GPU) | ⬜ open | [PR-9-2](phase-9/PR-9-2%20Linux%20flatpak%20build.md) |
| Distribution: self-hosted `.flatpakref` + repo | ⬜ open | [PR-9-3](phase-9/PR-9-3%20Distribution%20channel.md) |
| First-run onboarding (pick printers, prompt for access info) | ⬜ open | [PR-9-4](phase-9/PR-9-4%20First-run%20onboarding.md) |
| Project file format `.3mf` finalized (FR-MP-4) | ⬜ open | [PR-9-5](phase-9/PR-9-5%20Project%20file%20format.md) |
| OrcaSlicer profile importer (one-time migration; cut candidate) | ⬜ open | [PR-9-6](phase-9/PR-9-6%20Orca%20profile%20importer.md) |
| Documentation: getting-started, troubleshooting, release notes | ⬜ open | [PR-9-7](phase-9/PR-9-7%20Documentation%20and%20release%20notes.md) |
| Independence audit (clean Linux box, no other slicer) — exit gate | ⬜ open | [PR-9-8](phase-9/PR-9-8%20Independence%20audit.md) |

## Exit criteria (Execution_Plan §11)

- Flatpak installs and runs cleanly on **Ubuntu, Fedora, and Arch** with
  current flatpak runtimes (PR-9-2).
- **Onboarding completes in under 5 minutes** for a user who has the A1
  mini access code at hand (PR-9-4; PRD §3.3 time-to-first-slice).
- **Independence audit passes**: an external tester on a clean Linux
  machine completes the full workflow with no other slicer installed
  (PR-9-8).
- **All MVP success criteria from PRD §3.3 are met** (PR-9-8 is the
  checklist run).

## Doc updates owed

Per PRD §11.3 (living documents):

- **PRD §3.3** — ✅ done (2026-05-31). The platecycler success
  criterion reframed from "platecycler plugin (compose hook) ships with
  the MVP" to the **post-slice macro append** shipped in Phase 8
  (phase-8.md scope decision 2); the compose hook is noted post-MVP.
- **`Execution_Plan.md` §11** — update once the distribution decision
  (PR-9-3) and any cut (PR-9-6) are final, so the plan reflects what
  actually shipped.
