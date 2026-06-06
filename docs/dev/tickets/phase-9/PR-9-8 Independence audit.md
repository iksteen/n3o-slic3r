# PR-9-8 — independence audit (exit gate)

Status: ✅ met (2026-06-07) — clean-box independence + the full
open→slice→print→monitor workflow are proven on a clean WSL2 box for
**both** printers, and an external (non-lead) tester reached **send** on
his own clean machine in ~5 minutes. The run surfaced one finding (Bambu
Developer-Mode discoverability), now filed **and fixed**. **The phase's
exit gate** — proof that "standalone at runtime" (PRD §5) is real, not
aspirational.

**Scope.** An external tester, on a **clean Linux machine with no other
slicer software installed**, completes the full workflow from the
flatpak: install → configure both printers → slice → preview G-code →
send to printer → monitor. This is the §11 exit criterion and PRD §3.3
success criterion #7, run for real, not asserted.

**Evidence / status (2026-06-06).** The substantive gate is met. A full
workflow — open → slice → send → monitor — was completed on a **clean
WSL2 distro** (no slicer and no build toolchain present; the flatpak's
bundled libslic3r + webview carry the entire slice path) for **both**
the A1 mini and the U1. That retires criterion #7 (clean-box
independence — no host slicer or libslic3r) and exercises the
send/monitor leg under the most adversarial supported environment (WSLg
GPU/compositor + WSL2 NAT-to-LAN-printer networking). The §3.3 feature
criteria are proven in their phases: #2 cascade visibility (Phase 4),
#4 plugin lifecycle (Phase 8), #5 50 MB preview (Phase 6), #6
platecycler auto-eject (Phase 8, hardware-validated), #1 multi-printer
slice + send completing without manual G-code editing (PR-9-1 + native
runs, incl. live A1 mini + U1 sends).

**External-tester run (2026-06-07).** A non-lead tester installed the
flatpak on his own clean machine and reached **send** in ~5 minutes —
time-to-first-slice well under the PRD §3.3 #3 budget. It surfaced one
real finding: the Bambu **Developer Mode** requirement (recent firmware
rejects third-party MQTT commands without it — err_code 84033543) was
documented but not discoverable in-app, costing the tester ~5 extra
minutes. **Filed and fixed** — n3o now parses the command-rejection
`err_code` and surfaces actionable Developer-Mode guidance instead of
swallowing the rejection (the printer's on-screen error was previously
invisible to the app). With the external run done and the finding
resolved, the gate is **met**.

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
  - platecycler auto-eject at print end (#6) — the post-slice
    macro-append proof (phase-8.md decision 2; PRD §3.3 already
    reframed off the "compose hook" wording, 2026-05-31);
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
