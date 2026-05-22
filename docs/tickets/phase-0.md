# Phase 0 — tickets

Phase 0 (Foundation, ~2 person-weeks) is the foundation work that
makes the rest of the plan executable: project scaffold, FFI link,
core module boundaries, logging, CI, smoke procedure.

Source: `docs/Execution_Plan.md` §2. Stated exit criteria:

> App launches on the project lead's primary dev machine. Frontend
> shows libslic3r version. CI green on Linux.

Individual tickets live one-per-file in `phase-0/`. This file is the
index plus phase-level status and notes.

## Status by deliverable

| Deliverable | Status | Ticket |
|-------------|--------|--------|
| Tauri 2.x project scaffolded, React + TS frontend wired | ✅ done | — (commit `5f4a34d`) |
| Tailwind frontend wired | ✅ done | [PR-0-1](phase-0/PR-0-1%20Add%20Tailwind.md) |
| orca-slicer-ffi linked into the Tauri core | ✅ done | — (vendored at `crates/slic3r-ffi`) |
| Tauri command exposes `slic3r_version()` to the frontend; UI displays it | ✅ done | — (`slicer_info()` + `App.tsx`) |
| Logging infrastructure (`tracing` crate) wired into Rust core | ✅ done | [PR-0-2](phase-0/PR-0-2%20Wire%20tracing%20as%20Rust%20logging%20backend.md) |
| Repo structure matches PRD §8.2 module boundaries | ✅ done | [PR-0-3](phase-0/PR-0-3%20Stub%20core%20module%20structure.md) |
| Linux CI building | ⏳ in progress | [PR-0-4](phase-0/PR-0-4%20Linux%20CI%20workflow.md) |
| Phase 0 exit smoke documented | ✅ done | [PR-0-5](phase-0/PR-0-5%20Phase%200%20exit-criteria%20smoke.md) |

## Notes on what's *not* in Phase 0

Worth restating so future readers don't confuse phase boundaries:

- **Cascade resolver** — Phase 1.
- **3D viewport** — Phase 2.
- **End-to-end slice through a settings UI** — Phase 3.
- **Settings panel UI with cascade ladder** — Phase 4.
- **Plate-printer binding, multi-printer projects** — Phase 5.
- **G-code preview** — Phase 6.
- **Printer connectivity + filament sync** — Phase 7.
- **Plugin system** — Phase 8.
- **Flatpak + release prep** — Phase 9.

If a Phase 0 ticket starts pulling in any of the above, that's
scope creep. Cut the ticket back, or move it to the appropriate
phase.

## Phase 0.5 reminder

After Phase 0 closes, Phase 0.5 (~1 person-week) runs five
engine-validation spikes before Phase 1 commits to the cascade
design. Spike 4 (coEnums) is already done; the other four
(cascade adapter end-to-end, mixed-nozzle-size slice, A1 mini AMS
slice, platecycler portability) are tracked in
`docs/tickets/phase-0.5.md` and its `phase-0.5/` ticket directory.

**Spike 1 constraint discovered during PR-0-5.** The slice example
currently fails `Print::validate()` against FullPrintConfig defaults
(use_relative_e_distances=1 with empty layer_gcode). Spike 1 — the
end-to-end cascade adapter slice — must therefore drive its first
successful gcode-out from a real **OrcaSlicer device profile**
(e.g., the Bambu A1 mini or Snapmaker U1 JSON shipped in
`external/OrcaSlicer/resources/profiles/`), converted into our
cascade format. Hand-rolled "minimum viable config" shortcuts are
not allowed — the whole point of Phase 0.5 is to validate that real
device profiles route through our adapter cleanly, including all
the dispatch-quirk normalizations listed in `docs/profiles.md`
"What stays libslic3r-shaped".
