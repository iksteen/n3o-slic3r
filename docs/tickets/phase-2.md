# Phase 2 — tickets

Phase 2 (3D viewport + model loading, ~4 person-weeks) builds the
visual + interaction layer of the slicer: a renderer-agnostic Rust
scene state, the Three.js viewport that reflects it, model loading
(STL / OBJ / .3mf), object transform operations, the object library,
auto-arrange, and the bed mesh.

Phase 2 starts *early* (parallel with Phase 3 if scheduling allows)
because the performance risk lives here. The renderer-vs-state
separation (PRD FR-3D-7 / AD-8) is load-bearing — the renderer is
swappable from Three.js to wgpu without rewriting state management.

Source: `docs/Execution_Plan.md` §4. Stated goal:

> Functional 3D scene with model load, transform operations, and bed
> visualization. Start early because perf risk lives here.

Individual tickets live one-per-file in `phase-2/`. This file is the
index plus phase-level status and notes.

## Status by deliverable

| Deliverable | Status | Ticket |
|-------------|--------|--------|
| Scene state types (Rust authoritative) | ✅ done | [PR-2-1](phase-2/PR-2-1%20Scene%20state%20types.md) |
| Scene Tauri command + event surface | ✅ done | [PR-2-2](phase-2/PR-2-2%20Scene%20command%20event%20surface.md) |
| STL + OBJ loaders | ❌ open | [PR-2-3](phase-2/PR-2-3%20STL%20and%20OBJ%20loaders.md) |
| `.3mf` project loader | ❌ open | [PR-2-4](phase-2/PR-2-4%203mf%20project%20loader.md) |
| Object transform operations | ❌ open | [PR-2-5](phase-2/PR-2-5%20Object%20transform%20operations.md) |
| Bed mesh + exclusion zones | ❌ open | [PR-2-6](phase-2/PR-2-6%20Bed%20mesh%20and%20exclusion%20zones.md) |
| Object library / scaffolding panel | ❌ open | [PR-2-7](phase-2/PR-2-7%20Object%20library%20panel.md) |
| Auto-arrange | ❌ open | [PR-2-8](phase-2/PR-2-8%20Auto-arrange.md) |
| Three.js renderer (scene + orbit + camera modes) | ❌ open | [PR-2-9](phase-2/PR-2-9%20Three.js%20renderer.md) |
| Three.js gizmo (move / rotate / scale) | ❌ open | [PR-2-10](phase-2/PR-2-10%20Three.js%20gizmo.md) |
| Rust scene-state perf gate (renderer-side FPS + Three.js↔wgpu pivot deferred to Phase 9) | ⚠️ scoped down | [PR-2-11](phase-2/PR-2-11%20Perf%20and%20pivot.md) |
| Phase 2 exit-criteria smoke | ❌ open | [PR-2-12](phase-2/PR-2-12%20Exit-criteria%20smoke.md) |

## Architecture invariant (AD-8 / FR-3D-7)

The **state-vs-renderer separation** is non-negotiable:

- **`core/scene/` (Rust) is authoritative.** All scene state — mesh
  registry, transforms, hierarchy, selection, gizmo state, camera
  state, exclusion zones — lives here. Mutations go through Tauri
  commands; state changes flow out via Tauri events.

- **The renderer is a view.** Three.js (MVP) or wgpu (future
  fallback) subscribes to scene events, applies them to its local
  mirror, and emits user-intent through commands. It does *not*
  hold authoritative state. If the renderer crashes / restarts /
  pivots, the Rust state is the source of truth.

- **Performance budget on the state side: ≤5 ms p99 for selection,
  transform, diff computation on a 1000-object scene** (per AD-8).
  Validated in PR-2-11.

- **Renderer-side FPS** (20M-triangle scene, ≥30 fps) is
  *deferred to Phase 9* — Phase 2 ships Three.js + WebGL without
  a formal FPS gate, since the dev rig (Nvidia 5070 Ti) is too
  far from "modest hardware" to make a meaningful Phase 2
  decision. Real perf evaluation needs modest test hardware,
  which lives with Phase 9 release prep. The state-vs-renderer
  separation is the load-bearing guarantee that lets us pivot
  later if Phase 9 surfaces a problem.

Resist the urge to let the frontend hold authoritative state, even
"just for this one case." Every exception erodes the swap-out
guarantee that lets us pivot away from Three.js if perf doesn't hit
the bar.

## Dependency graph

```
PR-2-1 (state types)
  ├── PR-2-2 (commands + events: built on state types)
  ├── PR-2-6 (bed mesh: extends state types)
  └── PR-2-7 (object library: catalog over state)

PR-2-2 (commands + events) ──► PR-2-3 (STL/OBJ: pushes meshes via commands)
                              ├── PR-2-4 (.3mf: extends loader infra)
                              ├── PR-2-5 (transforms: command set)
                              └── PR-2-9 (renderer: subscribes to events)

PR-2-5 (transforms)   ──► PR-2-8 (auto-arrange: composes transforms)
PR-2-9 (renderer)     ──► PR-2-10 (gizmo: renderer feature)
                       └── PR-2-11 (perf: tests the renderer)

PR-2-3 + PR-2-9 + PR-2-5 ──► PR-2-12 (exit smoke needs full loop)
```

## Exit criteria for the phase (from Execution Plan §4)

- Load a 50MB STL and a Bambu-Studio-authored .3mf, manipulate
  them, save and reload position.
- Performance target met (20M-triangle scene at ≥30 fps on
  integrated-GPU laptop) **or** pivot decision made and scheduled.
- Scene-state Rust module's command/event surface fully covered by
  tests that run without any renderer attached. A stub viewer that
  just logs events should produce sensible output for typical
  interaction sequences (load → select → transform → deselect).

## Cut candidates (from Execution Plan)

If pressed for time:

- **Auto-arrange (PR-2-8)** → manual placement only saves ~4 days.
- **Mirror operation (sub-deliverable of PR-2-5)** → saves ~1 day.
- **Ortho camera toggle (sub-deliverable of PR-2-9)** → saves ~1 day.

## Notes on what's *not* in Phase 2

- **Slicing** — Phase 3. Phase 2 only displays models; slicing the
  scene comes after the closed slice loop lands.
- **Settings UI** — Phase 4. The viewport coexists with the
  Settings panel, but the panel itself is Phase 4 work.
- **Multi-plate scenes** — Phase 5. Phase 2 ships a single-plate
  viewport.
- **G-code preview** — Phase 6.
- **Painted regions / per-volume materials** — Phase 5 / Phase 7.
  Phase 2 surfaces multi-volume objects (the 3MF reader handles
  them) but doesn't paint them; that lives with filament-sync.
- **Seeded / quality profiles** — explicitly post-MVP per
  `docs/PRD.addendum.01.Seeded-Profiles.md`. Slots into the
  PR-1-4 user-tier mechanism without resolver changes; needs UI
  (Phase 4) + bundle layout (post-MVP). Phase 2 doesn't address
  them.
