# PR-9-7 — documentation + release notes

Status: ⬜ open.

**Scope.** The user-facing docs a first-time installer needs, plus
release notes and known issues. Hard constraint (PRD §5, §11.5): the
docs must **not reference any other slicer as a required tool** — the
app is standalone.

**Acceptance criteria.**

- **Getting started** (Linux flatpak install path): from the
  `.flatpakref` install command (PR-9-3) → first-run onboarding
  (PR-9-4) → load a model → slice → preview → send. Written against the
  flatpak, not a dev checkout.
- **Troubleshooting**: the failure modes a clean-box user actually hits
  — flatpak permission/GPU issues (no 3D viewport), printer not
  reachable on the LAN (the `--share=network` scope + user-side
  setup), a slice that won't start. Each with a concrete check.
- **Plugin authoring guide**: already shipped (`docs/plugin-authoring.md`,
  PR-8-10) — link it from getting-started; don't rewrite. Confirm it
  still reads correctly against the `resources/plugins/` layout.
- **Release notes + known issues** for the MVP candidate: what works,
  the two supported printers, the explicit non-goals (Windows/macOS,
  Flathub, hot reload, compose hook — all post-MVP), and any limitation
  surfaced by PR-9-2 (WSL2) or a cut PR-9-6 (no profile import).
- No doc references OrcaSlicer/PrusaSlicer/etc. as something the user
  must install (mentioning OrcaSlicer as the *source* of an optional
  one-time import, PR-9-6, is fine).

**Effort.** ~1.5 days.

**Dependencies.** PR-9-2 (install path), PR-9-3 (the ref command),
PR-9-4 (onboarding flow to document). Best written near the end so it
matches what shipped.

**Out of scope.**

- A full manual / settings reference — the in-app cascade trace is the
  settings explainer (Phase 4); docs cover install + first run +
  troubleshooting, not every setting.
- Localization — English only for the MVP.
