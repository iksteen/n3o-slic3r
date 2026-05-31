# PR-9-8 — independence audit (exit gate)

Status: ⬜ open. **The phase's exit gate** — the proof that "standalone
at runtime" (PRD §5) is real, not aspirational.

**Scope.** An external tester, on a **clean Linux machine with no other
slicer software installed**, completes the full workflow from the
flatpak: install → configure both printers → slice → preview G-code →
send to printer → monitor. This is the §11 exit criterion and PRD §3.3
success criterion #7, run for real, not asserted.

**Acceptance criteria.**

- **Clean-box run.** On a machine with no OrcaSlicer / PrusaSlicer /
  Cura / system libslic3r present, the tester installs from the
  `.flatpakref` (PR-9-3) and completes: onboarding both the A1 mini and
  the U1, loading a multi-plate project, assigning plates to either
  printer, slicing, previewing the G-code in-app, sending to the
  printer, and monitoring — **with no manual G-code editing** (PRD §3.3
  #1). No step requires another slicer or a host-installed libslic3r.
- **Time-to-first-slice under 5 minutes** for the tester with a printer
  access code at hand (PRD §3.3 #3 / §11), measured and recorded.
- **PRD §3.3 success-criteria checklist run**, each marked met or with a
  filed gap:
  - multi-printer slice + send, prints complete without manual G-code
    editing (#1) — depends on **PR-9-1**;
  - settings-cascade visibility (5/5 testers identify the responsible
    layer within 10s) (#2) — the cascade-trace UI from Phase 4;
  - time-to-first-slice < 5 min (#3) — depends on **PR-9-4**;
  - beep-at-layer plugin write→drop→enable (#4) — Phase 8;
  - 50 MB G-code preview (slider, color modes, hover, stats) (#5) —
    Phase 6;
  - platecycler sequential plates (#6) — **reframed** to the post-slice
    macro-append proof (phase-8.md decision 2; close PRD §3.3's stale
    "compose hook" wording here, doc-update owed);
  - clean-box independence (#7) — this audit itself.
- **Findings filed**, not waved through: anything the audit surfaces
  becomes a tracked bug/follow-up. A failed criterion blocks the MVP
  candidate until fixed or explicitly deferred by the project lead.
- **WSL2 note** (scope decision 3): record whether the clean-box flow
  also runs under WSLg as a documented known-limitation, not a gate.

**Effort.** ~1 day (the run + writeup; fixes for what it finds are their
own work).

**Dependencies.** Everything the workflow touches: **PR-9-1**
(correct slices), **PR-9-2 / PR-9-3** (installable flatpak), **PR-9-4**
(onboarding), **PR-9-7** (getting-started the tester follows). Runs
**last**.

**Out of scope.**

- Fixing what the audit finds — failures spawn their own tickets; this
  ticket is the *gate*, not the remediation.
- Non-Linux platforms — post-MVP.
- A formal external beta program — one external tester on a clean box
  satisfies the §11 criterion; broader testing is post-MVP.
