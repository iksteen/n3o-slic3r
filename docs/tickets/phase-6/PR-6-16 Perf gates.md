# PR-6-16 — Perf gates

Status: ❌ open.

**Scope.** Mechanize the FR-GP-9 perf budgets as automated
assertions: 50MB G-code parsed + IR built in <5s; layer
slider scrub at 60fps; memory <1.5GB. Mirrors Phase 1's
PR-1-10 and Phase 2's PR-2-11.

**Acceptance criteria.**

- **Rust criterion benchmark** (`src-tauri/benches/preview_perf.rs`):
  - Bench 1: `build_preview` on a 5MB synthetic G-code →
    asserts <500ms. Bench 2: same on a 50MB fixture →
    asserts <3s. Both gated behind `#[ignore]` so the
    default `cargo bench` runs the small one; CI runs both.
  - Bench 3: `compute_layer_stats` + `compute_job_stats` on
    50MB IR → asserts <500ms.
  - Bench 4: `encode_colors` on 50MB IR for each color mode
    → asserts <200ms each.

- **Fixture strategy** (per the index's open question):
  recommend a 5MB real fixture checked into the repo
  (e.g. a 20mmbox sliced at 0.08mm with high perimeter
  count) + a generate-at-test-time 50MB fixture (slice a
  bigger model in the `#[ignore]`'d bench's setup). The
  generated fixture path lives in `target/preview-bench/`
  and is cached across runs. Document the regenerate
  recipe in the bench's module doc.

- **Frontend frame-time check** (vitest with manual
  driving):
  - Test that mounts the renderer with a 50MB fixture
    loaded, scrubs the layer slider rapidly (programmatic
    `value` changes), measures `requestAnimationFrame`
    deltas between scrub events. Asserts 95th percentile
    < 17ms (60fps).
  - This needs a real WebGL context — vitest with jsdom
    won't cut it. Either:
    - **Playwright integration test** that mounts the app
      and drives the slider via DOM interaction. Heavy but
      accurate.
    - **Skip in CI, add a manual perf check script** the
      project lead runs on the reference hardware.
  - Recommendation: ship the Playwright path even if
    skipped in CI, so the apparatus is in place for
    nightly runs. Mark `it.skip` until CI infrastructure
    catches up.

- **Memory budget:** assert that after loading the 50MB
  fixture, the Rust process's resident memory is under
  ~700MB (gives 800MB headroom on the 1.5GB total budget
  for the WebGL side). Use `procfs` on Linux,
  conditional-compile gate for other platforms.

- **CI integration:** the bench's small fixtures run on
  every PR (`cargo bench --quick` or equivalent). The big
  ones run nightly. Failing a perf gate posts a CI
  comment with the regressed measurement vs the budget.

- Tests:
  - Each bench has a corresponding assertion test that
    runs the bench programmatically and verifies the
    timing budget.
  - Memory-budget test runs the 50MB load + memory probe
    in sequence.

**Effort.** ~1 day. Most of the work is the fixture +
benchmark setup; the assertions themselves are mechanical.

**Dependencies.** PR-6-4 (IR), PR-6-5 (colors), PR-6-6
(stats), PR-6-8 (renderer for the frame-time check).

**Out of scope.**

- Per-color-mode perf optimization (if a mode misses
  budget, fix it in that ticket, not here).
- WebGL memory profiling (browser-level perfMonitor is
  too noisy for CI; visual inspection during the
  Playwright run is enough).
- Long-tail percentile analysis (95th is sufficient for
  MVP).

**Cut candidate.** Frame-time Playwright test → save ~1
day. Land the Rust perf gates only; the frontend perf
verifies manually. Acceptable cut if Playwright
integration is more work than expected.
