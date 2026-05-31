# PR-9-4 — first-run onboarding

Status: ⬜ open. Frontend over existing backend.

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
