# Phase 0.5 — tickets

Phase 0.5 (Engine validation spikes, ~1 person-week) runs five
focused experiments before Phase 1 commits to the cascade design.
Each spike is small, throwaway, and produces a written finding
document. The goal is to validate assumptions cheaply — a passed
spike unblocks the downstream phase; a failed spike triggers a plan
revision.

Source: `docs/Execution_Plan.md` §2.5. Stated exit criteria:

> Five findings documents committed to the repo (one per spike),
> each with: assumption tested, method, result, implications for
> downstream phases. Any failed spike has a corresponding
> plan-revision PR open, not deferred.

Individual tickets live one-per-file in `phase-0.5/`. This file is
the index plus phase-level status and notes.

## Status by spike

| Spike | Status | Ticket |
|-------|--------|--------|
| Cascade adapter end-to-end | ✅ done | [PR-0.5-1](phase-0.5/PR-0.5-1%20Cascade%20Adapter%20end-to-end.md) (finding: `docs/spikes/spike-1-cascade-adapter.md`) |
| Mixed-nozzle-size slice (Prusa XL) | ✅ done | [PR-0.5-2](phase-0.5/PR-0.5-2%20Mixed-nozzle-size%20slice.md) (finding: `docs/spikes/spike-2-mixed-nozzle.md`; toolchange criterion confirmed by PR-0.5-3) |
| Bambu A1 mini AMS slice | ⚠️ done with Phase-5 prerequisite | [PR-0.5-3](phase-0.5/PR-0.5-3%20Bambu%20A1%20mini%20AMS%20slice.md) (finding: `docs/spikes/spike-3-bambu-ams.md`; BBS comparison surfaces ~10× more tool-changes than BBS — Phase 5 must solve before hardware validation) |
| coEnums known limitation impact | ✅ done | — (surfaced via `1bb3503`; documented in `docs/libslic3r-workarounds.md` §5) |
| platecycler portability | ❌ open | [PR-0.5-5](phase-0.5/PR-0.5-5%20platecycler%20portability.md) |

Findings docs land at `docs/spikes/spike-<n>-<slug>.md`. The Spike 4
finding is inline in `docs/libslic3r-workarounds.md`; the others
get dedicated files.

## Findings doc template

Every finding doc follows the same shape so they're skimmable as a
set:

```markdown
# Spike <n>: <slug>

## Assumption tested
<one paragraph — the precise claim being verified>

## Method
<numbered steps that someone else could re-run>

## Result
<pass / partial / fail, with the concrete evidence>

## Implications for downstream phases
<one paragraph per downstream phase impacted; explicit
recommendations: proceed as planned / revise the plan in X way /
defer Y>

## Artifacts
<paths to scripts, test inputs, captured gcode, screenshots>
```

If a spike fails, the implications section names the plan-revision
PR (link it once filed).

## Notes on what's *not* in Phase 0.5

Spikes are throwaway. None of the spike code is expected to live
past Phase 1; the only durable artifacts are the finding documents
and any updates to `docs/profiles.md`,
`docs/libslic3r-workarounds.md`, or the FFI shim that the spikes
prompted.

In particular:

- **Don't try to make the spike's resolver production-quality.** The
  Phase 1 resolver gets a clean-room reimplementation informed by
  the finding doc.
- **Don't carry spike code into `src-tauri/src/core/` permanently.**
  Spike code can live there during the spike (it's where you'll be
  testing the FFI from), but delete or stub it back at spike end.
- **Don't bundle multiple spikes into one PR.** Each spike's PR
  carries its scripts, fixtures, and finding doc. Reviewer can
  consume them independently.

## Exit criteria for Phase 0.5

- Four new finding docs at `docs/spikes/spike-{1,2,3,5}-*.md`
  (Spike 4's finding is in `libslic3r-workarounds.md` §5).
- Any failed spike has a plan-revision PR open against
  `docs/PRD.md` or `docs/Execution_Plan.md`.
- `docs/libslic3r-workarounds.md` reflects any newly discovered
  quirks (and removes any of the existing five that turned out to
  be already fixed upstream).
- The Phase 0.5 milestone (`M0.5 — Engine assumptions validated` in
  `docs/Execution_Plan.md`) ticks over.

## Cut candidates (from `Execution_Plan.md`)

If pressed for time:

- **PR-0.5-5 (platecycler)** can slip to "early Phase 8." Cost:
  Phase 8 starts cold against an unknown.
- **Spike 4 (coEnums)** is already done; no decision needed.

PR-0.5-1, PR-0.5-2, PR-0.5-3 are not cut candidates — each
de-risks a downstream phase whose architecture depends on the
answer.
