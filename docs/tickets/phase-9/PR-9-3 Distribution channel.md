# PR-9-3 — distribution: self-hosted `.flatpakref` + repo

Status: 🟢 publish tooling done + validated (2026-05-31); awaiting the
host URL + deploy (operational, the maintainer's step).

> **Outcome.** `packaging/flatpak/publish.sh` is the signed release path
> (vs. `build.sh`, the unsigned dev path): it does a GPG-signed
> `flatpak-builder` export into `.publish-repo/`, signs the repo summary
> + static deltas, exports the public key, and generates the
> `.flatpakref` (URL + embedded key). Validated end-to-end with a
> placeholder URL: the repo summary is signed (`summary.sig` present),
> and the ref's embedded `GPGKey` decodes to the project key fingerprint.
>
> - **Hosting:** maintainer's own HTTPS server/domain (per the PR-9-3
>   decision). `N3O_FLATPAK_REPO_URL` is a required env param; the script
>   prints the rsync/scp upload steps. The actual deploy + a clean-machine
>   install-from-the-real-URL are the remaining operational steps (folds
>   into the PR-9-8 audit's clean-box run).
> - **Signing:** a **dedicated** project key
>   (`B3D305B4…0335DC53`, UID "n3o-slic3r release signing key"), separate
>   from any personal key. Public key tracked at
>   `packaging/flatpak/n3o-slic3r-signing-key.asc`. Passphraseless for
>   scriptable signing (rotation/protection notes in `PUBLISHING.md`).
> - **Docs:** `packaging/flatpak/PUBLISHING.md` (key, hosting, per-release
>   publish, install command, AGPL source pointer).
> - The install path is signed (no `--no-gpg-verify`); the ref's
>   `RuntimeRepo` pulls the GNOME 50 runtime from Flathub automatically.

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
