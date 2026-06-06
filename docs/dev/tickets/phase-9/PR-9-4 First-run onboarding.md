# PR-9-4 — first-run onboarding

Status: ✅ satisfied by existing UI (2026-05-31), reduced scope per the
project lead. The onboarding need is already met; inline connection
setup and sequenced multi-printer onboarding are deferred (non-blocking).

> **What already covers this** (audited 2026-05-31):
> - **First-launch empty state** — `src/printer/PrintersEmptyState.tsx`,
>   gated by `App.tsx`'s `noPrinters` (empty instance library): a
>   "Set up your first printer" hero + CTA into the add-printer modal,
>   instead of a bare canvas.
> - **Add-printer modal** — `src/printer/AddPrinterModal.tsx`: searchable
>   bundled catalog (incl. **A1 mini** + **U1**, A1 mini default), name +
>   AMS config, creates a real `PrinterInstance` via
>   `printer_instance_create`; the new instance binds to the active plate.
> - **Connection setup exists** — not in the add-printer modal (by
>   design) but in `PrinterSettingsModal`'s Connection tab (IP + access
>   code for Bambu / IP + Moonraker port for U1, with validation + a
>   "Test connection" button), persisted to `PrinterInstance.connection`.
>   So "prompt for access info" is a *separate post-create step*, not a
>   gap in capability — and connection matters only for **sending**, not
>   slicing, so time-to-first-slice (the §11 / PRD §3.3 gate) is met by
>   the add-printer flow alone.
>
> **Deferred (non-blocking, post-MVP polish):**
> - Collecting connection/access info **inline** in the add-printer flow
>   rather than via the settings modal afterwards.
> - A **sequenced** "add A1 mini, then U1" first-run (today you add one,
>   then click "Add printer" again — fine for MVP).
>
> Project lead's call: "there's already an add printer dialog … it
> doesn't do the printer connection setup but that's okay for now."

**Scope.** A guided first-launch flow: the user picks their printer(s)
from a list that includes the **A1 mini** and **U1**, and is prompted
for the access info each needs to connect. The goal is the §11 / PRD
§3.3 gate: **time-to-first-slice under 5 minutes** for a user who has
their printer access code at hand.

This builds the *flow* over backend that already exists — the
add-printer wizard, the empty first-launch state, and the user
instance library (`core::printer::instance_storage`, Phase 5/7). No new
storage layer.

**Acceptance criteria.**

- On a first launch with an **empty** instance library, the app shows
  an onboarding flow (not a bare empty canvas): "pick your printers"
  from the bundled catalog (A1 mini, U1 at minimum), then per chosen
  printer, prompt for the **access info** that printer's driver needs
  (A1 mini: LAN access code / serial per the Phase 7a send path; U1 per
  its driver).
- Picking a printer writes a real `PrinterInstance` via the existing
  wizard/storage path — onboarding is a guided front-end on the same
  write, not a parallel one. Credentials persist in the per-printer
  user-library instance `.toml` (their intended home; memory:
  `no_credentials_in_project_file`), **not** in shareable project
  files.
- After onboarding the user lands in a usable project with their
  printer(s) bound and can load a model → slice without further setup.
- The flow is **skippable / re-enterable**: a user can dismiss it and
  reach it again later (the add-printer entry point already exists);
  onboarding doesn't trap first launch.
- Don't defer the UI (memory: `dont_defer_frontend`) — this ticket *is*
  the frontend; ship it wired, not as a backend-only stub.

**Effort.** ~2 days.

**Dependencies.** Phase 5 (instance library) + Phase 7a/7b (per-printer
access info / drivers) — both done. Feeds PR-9-7 (the getting-started
doc walks this flow) and PR-9-8 (the audit times it).

**Out of scope.**

- New driver/transport work — onboarding collects the access info the
  existing drivers already consume.
- Cloud accounts / login — there are none (no network except to
  user-configured printers, PRD §11.5).
- A printer beyond the two MVP targets in the catalog.
