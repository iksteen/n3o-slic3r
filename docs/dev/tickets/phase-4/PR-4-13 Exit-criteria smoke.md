# PR-4-13 — Phase 4 exit-criteria smoke

Status: ✅ shipped — `src-tauri/tests/phase4_smoke.rs` (3 backend integration tests covering introspection coverage + A1 mini/U1 capability filter outcomes + render-budget gate). `docs/dev/phase-4-smoke.md` ships the full procedure with the manual UX-study script and the visual gates to walk against the mockup until App.tsx integration lands in Phase 5. Frontend per-ticket vitest covers the pure helper contracts (88 cases across 10 test files). Total green: 262 Rust + 88 frontend tests.

**Scope.** End-to-end smoke that exercises Phase 4's exit criteria
as a documented procedure + an automated half. Mirrors
`docs/dev/phase-3-smoke.md` / `phase-2-smoke.md` / `phase-1-smoke.md` /
`phase-0-smoke.md`.

The two Phase 4 exit criteria are **UX outcomes**, not just
test-suite assertions:

> 5-user UX test passes: given a project where a value differs
> from default, 5/5 users identify the source layer within 10
> seconds — by reading the inline breadcrumb or by hovering for
> the ladder.

> A1 mini and U1 both render their full settings panel correctly:
> A1 mini hides toolchange options, U1 hides purge volumes matrix;
> both show priming tower geometry settings; U1 shows 4-slot tab
> strip while A1 mini shows single pane.

The UX test is necessarily human-driven; the rest can land in CI.

**Acceptance criteria.**

- `docs/dev/phase-4-smoke.md` documents:
  1. **Automated half** — `cargo test --workspace` + `npm test`
     including the new settings-panel perf gate from PR-4-4 +
     the per-ticket vitests from PR-4-2..PR-4-12.
  2. **Manual half — printer profile coverage:** open the panel
     with A1 mini → verify toolchange options absent, single-pane
     layout. Switch to U1 → verify purge volumes matrix absent,
     4-slot tab strip visible, sync-edit defaults ON.
  3. **Manual half — UX disclosure test:** the protocol for the
     5-user study (script + the project fixture they're shown +
     the success metric: identify source layer within 10s).
     This is informational — the actual study happens once and
     the result is captured back into the doc as a "passed on
     YYYY-MM-DD with N=5" line.
  4. **Manual half — per-object override walk-through:** select
     an object, switch to Object tab, override `wall_filament`,
     verify the badge appears on Project tab with the object's
     filament color.

- `src-tauri/tests/phase4_smoke.rs` (automated half — chain the
  pieces that can be tested without a renderer):
  1. **Backend introspection coverage** (PR-4-1): assert
     `slicer_options_for_printer(A1 mini)` hides at least the
     known-toolchanger keys (`purge_volumes_matrix`,
     `toolchange_gcode`) and surfaces the right scope flags
     for known options (`bed_temp` project, `support_filament`
     object, `wall_filament` region).
  2. **Cascade trace bulk fetch perf** (PR-4-7/8): assert
     `cascade_trace` for the bundled A1 mini cascade completes
     in < 30 ms on the workspace's reference machine (debug
     mode in CI; release locally — same convention as
     `cascade_perf`).
  3. **Per-object override round-trip** (PR-4-9): set, clear,
     clear-all via the Tauri command surface; assert
     `cascade_resolve_with_overrides` honors each tier.
  4. **Support toggle round-trip** (PR-4-12): set
     `enable_support = true` on an object, re-slice via the
     PR-3-2 orchestrator, parse the output via PR-3-6, assert
     at least one `;TYPE:support` feature line is present. Set
     to `false`, re-slice, assert zero support feature lines.

- `src/settings/__test__/exit_smoke.test.ts` — vitest covering
  the frontend halves that don't need a real renderer:
  - Mount the panel with a stubbed A1 mini → assert
    toolchange-category absent and slot-tab-strip absent.
  - Mount with a stubbed 4-slot U1 → assert slot-tab-strip
    present.
  - Mount with a project that has 3 project-tier overrides →
    assert the panel-header badge reads "3".
  - Mount with an object that overrides `wall_filament` →
    switch to Project tab → assert that row shows the
    objects-overriding badge.

- CI: the automated halves run in the existing
  `cargo test --workspace` + `npm test` steps; no new CI jobs.

**Effort.** ~2 days. The doc + the `phase4_smoke.rs` integration
test + the vitest mount tests. The UX study itself is an
out-of-band activity scheduled separately.

**Dependencies.** Every Phase 4 ticket. This is the last one to
land.

**Out of scope.** The 5-user UX study itself (an activity, not a
deliverable). Phase 5's project-save round-trip.

**The smoke is the project's gate for cascade visibility.** If a
future change accidentally hides the breadcrumb on a row, or
breaks the cascade ladder's portal positioning, or causes the
objects-overriding badge to miss an object, the smoke fails. That
preserves the project's primary differentiator across refactors.

**Design reference.** The manual-half walkthrough should verify
the production panel visually matches `docs/dev/design/index.html`
(or the standalone bundle) on the relevant surfaces:

- Settings row left-edge **rule** appears on hover with the
  winning layer's hue (palette from `docs/dev/design/data.jsx`:
  default 220 / printer 18 / build_plate 95 / filament 175 /
  user 235 / project 285 / object 340). At rest the rule is
  transparent.
- Project-tier overrides show the `.authored-project` purple
  tint persistently (bold name, hue 285). Object-tier overrides
  show the `.authored-object` rose tint and win over the
  project tint (hue 340).
- **No breadcrumb chip strip** in the shipped panel — that's
  the `accountability === "breadcrumb"` tweak from the mockup
  and must not appear.
- Hover ladder positions to the left of the row when there's
  room, falls back right; closes after ~120 ms when the cursor
  leaves.
- Objects-overrides badge shows up to 3 filament-color dots
  plus a `+N` overflow when more than 3 objects override a
  setting.
- Reset button (counter-clockwise arrow) appears only when the
  active tier has an authored value.
- Project / Object tabs auto-fall-back to Project when the
  selected object goes away.

Visual divergence between production and mockup is fine where
production has data the mockup lacks (the full ~800-option
list, real capability filtering, validation badges) — but the
core cascade-visibility affordances are mockup-canonical.
