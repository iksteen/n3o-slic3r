# PR-9-3 — distribution: self-hosted `.flatpakref` + repo

Status: ⬜ open.

**Scope.** Stand up the MVP distribution channel: a self-hosted flatpak
repo + a `.flatpakref` users can install from. Per `Execution_Plan.md`
§11 and phase-9 scope decision 4, the MVP is **self-hosted** (faster
iteration, no review wait); Flathub submission is post-MVP.

**Acceptance criteria.**

- A **flatpak repo** (`flatpak build-export` + `flatpak build-update-repo`,
  or `ostree`) hosting the PR-9-2 build, served over HTTPS from a
  documented location.
- A **`.flatpakref`** (and a one-line `flatpak install` command) that a
  user can run on a clean machine to install the app and its runtime.
- The repo is **GPG-signed** and the `.flatpakref` carries the public
  key, so installs verify (no `--no-gpg-verify` in the documented path).
- A documented **publish procedure**: build → export → sign → update
  repo → bump the ref, so a release can be cut repeatably (referenced
  from PR-9-7's release notes).
- AGPL-3.0 compliance: the repo or app surface points at the
  corresponding source (the linkage forces it; PRD §10).

**Effort.** ~1 day.

**Dependencies.** PR-9-2 (needs a flatpak artifact to export).

**Out of scope.**

- **Flathub submission** — post-MVP (the review + manifest-policy work
  is deferred; phase-9 scope decision 4).
- Auto-update infrastructure beyond what flatpak's repo model gives
  for free.
- Mirrors / CDN — a single documented host is enough for the MVP.
