# PR-0-4 — Linux CI workflow

Status: ⏳ in progress (workflow committed as `749b95b`; first
successful run pending).

**Scope.** GitHub Actions workflow that builds the workspace and
runs the test suite on every push and PR. Caches the OrcaSlicer
deps tree because that's the long pole.

**Acceptance criteria.**

- `.github/workflows/build.yml` exists.
- Triggers: `push` to `main`, `pull_request` against `main`,
  `workflow_dispatch`.
- Runs on `ubuntu-latest`.
- Steps in order:
  1. Checkout with `submodules: recursive`.
  2. Install system deps (gtk3, dbus, webkit2gtk, mesa, cmake,
     ninja, GCC, pkgconf, ...). Use `apt` directly.
  3. Cache `external/OrcaSlicer/deps/build/OrcaSlicer_dep/` keyed
     by the OrcaSlicer submodule SHA + the deps build script
     contents. Cache miss → run `./scripts/build.sh deps`.
  4. Cache `~/.cargo` and `target/` keyed by `Cargo.lock` +
     submodule SHA.
  5. Install Rust stable (`dtolnay/rust-toolchain@stable`).
  6. Install Node 20.
  7. `npm ci`.
  8. `cargo test --workspace --release` — builds the FFI .so via
     cmake, builds the Rust workspace, runs the 16 integration
     tests in `crates/slic3r-ffi/tests/api.rs`.
  9. `npx tsc --noEmit` on the renderer.
  10. `npm run build` — Vite production build.
- A first run completes successfully on a fresh push.
- CI badge in `README.md` reflecting the latest status.

**Effort.** 1–2 days. Most of the time is cache tuning to keep the
deps build to cold runs only — without that, every CI run is ~17 min
on the deps step alone.

**Dependencies.** None code-wise. Verification requires the remote
to accept pushes (already true; remote at
`git@github.com:iksteen/n3o-slic3r.git`).

**Out of scope.** Windows and macOS runners (PRD §3.2 explicit
non-goal for MVP). Release-artifact upload (flatpak build) — Phase 9.
GUI smoke tests with xvfb — also post-MVP.

**Known follow-ups.** OrcaSlicer's `build_linux.sh -d` enforces a
≥10G RAM precheck; GitHub-hosted runners advertise ~7G. Worked
around in `scripts/build.sh` by passing `-r` to skip the precheck
(commit `5b4128f`). If the deps build OOMs at link time, follow up
by lowering ninja parallelism (`NINJA_STATUS=…`, `-j2`) or by
splitting deps into stages.
