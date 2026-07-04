# Review — `src/` (TypeScript / React frontend)

Fresh architectural + over-engineering ("ponytail") review, 2026-07, ahead of
public release. Six area reviewers (state layer, IPC boundary, component
architecture, settings/cascade UI, domain UI, ponytail sweep); every finding
was adversarially re-verified against the code, the backend commands it calls,
and the tests. One candidate (a "duplicated error-to-string helper") was
refuted and dropped.

**14 findings: 0 high, 4 medium, 10 low.** No memory-unsafety or data-loss
class here (that's the backend's domain), but two real correctness bugs — the
settings panel shows **stale resolved values** after a bed/nozzle change, and
the viewport re-renders + re-blits a full GPU frame on every slice-progress and
driver-heartbeat tick — plus ~450 LOC of dead components/wrappers/types to
delete before release. The state architecture (event-invalidated query cache,
Rust-owned resolution, dedup'd event router) is sound; the findings are places
that bypass it or drifted from it.

Item format: `ID — title` · files · action · *verify:*. Checkboxes unchecked;
nothing here is applied or committed.

---

## Phase 1 — Documentation truth-up

- [ ] **FE-D1 — `DevicesView` header describes a removed implementation**
  `src/driver/DevicesView.tsx:16-20`.
  Says "the driver-reported currently engaged slot highlight is not wired yet" and "webcam is stubbed: a disabled-camera icon + Not implemented." Both false — `DeviceMonitor` feeds the loadout from `loadoutFromReport(status)` (live MQTT report incl. the `active` engaged-slot highlight) and mounts a real `CameraPanel`/`useCameraStream`. Violates "docs describe the present, not removals."
  *Action:* rewrite to the current wiring (loadout from the live report with engaged-slot highlight; live camera via `useCameraStream`), or drop the stale bullets.
  *verify:* header matches `DeviceMonitor`.

---

## Phase 2 — Delete / simplify (release cleanup)

Each grep-verified to have no production caller (frontend + backend + tests
checked). "Kept alive only by its own test" is the release-cleanup target
("tests leaning on removed prod methods").

- [ ] **FE-X1 — Dead components `BambuAmsStrip` + `U1ToolheadStrip` (~220 LOC)** *(MEDIUM; ponytail)*
  `src/driver/BambuAmsStrip.tsx`, `src/driver/U1ToolheadStrip.tsx`, cf. `src/driver/DeviceLoadout.tsx`.
  Zero production importers; only their own tests pull `chipsFromAms`/`cellsFromU1`. The Devices monitor renders via `DeviceLoadout`/`loadoutFromReport` + `DeviceStats`, and the strips' active-slot + color-from-hex projection was reimplemented inside `loadoutFromReport` (`amsLoadoutRow` vs `chipFromTray`). Two copies of the same derivation, one dead.
  *Action:* delete both components + their tests. If the projection helpers are worth keeping, first collapse them and `loadoutFromReport` onto one shared derivation.
  *verify:* Devices view renders unchanged; `tsc` + vitest green.

- [ ] **FE-X2 — Dead module `buildContextJson.ts` + orphaned cascade types (client-builds-context vestige)** *(MEDIUM; ponytail)*
  `src/settings/buildContextJson.ts`, `src/settings/__test__/buildContextJson.test.ts`, `src/settings/resolve.ts`.
  `buildContextJson()` + helpers (`overridesToFileSpec`, `DEFAULT_BUILD_PLATE`, `DEFAULT_FILAMENT`, `tomlEscape`) build a full `ContextJson` client-side — the *old* resolve path. The live panel calls `plate_cascade_resolve` with just `{ plateId }` (`resolve.ts:230-233`); the backend owns the context. Only its own test imports it. The types it consumes (`ContextJson`, `OverrideFileSpec`, `FilamentProfileJson`, `BuildPlateJson`) have no other consumer. This is the "TS re-derives the context the backend owns" anti-pattern, now dormant.
  *Action:* delete `buildContextJson.ts` + its test; prune the orphaned types (and the stale `ContextJson` JSDoc at `SettingsPanel.tsx:74`) from `resolve.ts`.
  *verify:* panel resolves unchanged; `tsc` green with the types gone.

- [ ] **FE-X3 — Dead invoke wrappers** *(ponytail)*
  `src/driver/invokes.ts:49` (`driverList` — no caller anywhere; connection layer uses `driverStatus` + events), `src/printer/printerCommands.ts:64,74` (`setActivePrinter` — doc falsely claims "kept for App.tsx's first-mount default," App never calls it; `printerInstanceSetBed` — duplicate of the live `setInstanceBed`, `printerInstance.ts:180`), `src/settings/processFragment.ts:69,96-112` (`setInstanceQualityProfile`, `getUserProcess` + `UserProcess`), `src/settings/overrideCommands.ts:55` + `projectOverrideCommands.ts:35` (`clearAllObjectOverrides`/`clearAllProjectOverrides` — referenced only by tests; the "wires the reset-all button" comment describes a button that doesn't exist).
  *Action:* delete each wrapper + its now-orphaned test; drop the now-unreferenced backend commands (`driver_list`, `scene_set_active_printer`) from `generate_handler!` if nothing else uses them. **Cross-ref:** `getUserProcess` pairs with backend **T-X2** (`user_process_get`).
  *verify:* `tsc` + vitest green; `grep` confirms zero remaining references.

- [ ] **FE-X4 — Dead re-export `defaultWindow` in `LayerSlider`** *(ponytail)*
  `src/preview/LayerSlider.tsx:231-233`. `export { defaultWindow }` with a comment claiming panel consumers use it, but the only consumer (`PreviewWorkspace.tsx:22`) and the tests import it from `./layerWindow` directly.
  *Action:* remove the re-export + comment.

- [ ] **FE-X5 — `Modal` speculative props no caller exercises** *(ponytail)*
  `src/ui/Modal.tsx:27,29,58,73-75`. `ModalBackdrop.cardStyle` (forwarded to `style`) has zero callers; `ariaModal` (default true) is never overridden so it only ever renders `aria-modal={true}`; `ModalCloseButton.ariaLabel`/`title` are never passed. YAGNI knobs.
  *Action:* delete `cardStyle` + its forwarding; hardcode `aria-modal` on the card; drop the unused close-button overrides (keep `ariaLabel` only if per-modal a11y labels are wanted — none differ today).

- [ ] **FE-X6 — Duplicated Vec3 primitives between `WgpuViewport` and `useSplitSession`** *(ponytail)*
  `src/viewport/WgpuViewport.tsx:73-85` (sub/scale/dot/cross/vlen/norm) vs `src/viewport/useSplitSession.ts:51-59` (`cross3`/`normalize3`, identical math). `WgpuViewport` already imports `planeBasis`/`worldOf` from `useSplitSession`, so a shared vec module is the established seam.
  *Action:* co-locate the Vec3 helpers in one module, import in both, delete the duplicates.

---

## Phase 3 — Correctness & robustness

- [ ] **FE-C1 — Cascade resolve goes stale after a build-plate or nozzle-diameter change** *(MEDIUM)*
  `src/settings/SettingsPanelHost.tsx:273-276,298,320-327`, `src/state/sceneSnapshot.ts:22-44`, backend `src-tauri/src/core/project/resolve.rs:160,180-188`, `src-tauri/src/core/printer/mod.rs:341,359`.
  `usePlateCascadeResolve` is keyed on `${quality_profile}|${printer_instance_id}|${processGen}`. But the backend composes the cascade from the bound instance's `bed.identity` and installed-nozzle loadout (source layers `build_plate`/`nozzle`), and changing bed (`setInstanceBed`) or nozzle (`setExtruderNozzleDiameter`) emits only `printer:instance_changed` — which is **not** in `SCENE_SNAPSHOT_EVENTS` and doesn't change any key in the dep string. So after the user picks a different build plate or nozzle in the panel header, every setting row + the cascade ladder keep showing the previous fragment's resolved values (bed temp, per-nozzle layer-height/speed defaults) until an unrelated re-resolve. Directly violates the "TS renders the Rust-resolved cascade" job by displaying wrong effective values. The host already computes `instanceBed` (`:298`) and `installedNozzleKey` (`:320-327`).
  *Action:* fold `instanceBed` + `installedNozzleKey` into the resolve dep string (both in scope), or add `printer:instance_changed` (filtered to the bound id) to the resolve's invalidation inputs.
  *verify:* change the build plate in the panel header; bed-temp and nozzle-derived rows update immediately without a plate switch.

- [ ] **FE-C2 — `WgpuViewport` fires a full GPU frame + framebuffer IPC blit on every App re-render** *(MEDIUM)*
  `src/App.tsx:565-571`, `src/viewport/WgpuViewport.tsx:1017-1028,225-239`, `src/viewport/useSplitSession.ts`.
  App builds the `split` prop as a fresh object literal each render whose `connectors` is `split.connectors.map(...)` — a new array reference every render (even when inactive: `[].map()` still yields a fresh array). `WgpuViewport`'s split-redraw effect lists `split?.connectors` in its deps, so it re-runs every render and calls `renderRef.current?.()` → a `viewport_frame` IPC (full offscreen GPU render + `width*height*4` byte blit to the webview). App subscribes at top level to high-churn stores (`useSliceJob` `slice.state`, `useDriverConnections` status heartbeats bumping `STATE_VERSION`), and `WgpuViewport` isn't memoized — so the viewport re-blits a full frame on every slice-progress tick and every driver heartbeat, scene and camera unchanged, split tool not even active.
  *Action:* depend on a stable signature of the split state (memoized string) instead of the raw array, or memoize `connectors` in `useSplitSession`/App so identity is stable when unchanged; alternatively `React.memo` `WgpuViewport` with a props comparator. Redraw should fire only when split geometry actually changes.
  *verify:* start a slice (or connect a printer) with a static scene; confirm `viewport_frame` isn't invoked per progress/heartbeat tick (log/trace the command).

- [ ] **FE-C3 — Dirty flag: a late-resolving initial fetch can overwrite a newer event** *(LOW; arch)*
  `src/project/useProjectSession.ts:52-67`.
  Hand-rolls the dirty flag outside the query cache: fires `invoke("project_is_dirty")` and, in parallel, subscribes to `project:dirty_changed` taking the payload directly, with no fetch-vs-event ordering guard (the `active` flag only guards unmount). If the initial fetch resolves after an event, its stale value clobbers the event's — the title-bar unsaved marker reads wrong until the next flip. The initial-fetch `.catch(()=>{})` also silently swallows a bootstrap failure. (Reachable window is narrow — the hook mounts once at app boot — hence low, but it's the exact race the query cache already solves.)
  *Action:* route dirty through the query cache (`{fetch: project_is_dirty, invalidateOn: ["project:dirty_changed"]}`); its inFlight/requeue coalescing guarantees the last settle reflects the latest event. Drop the bespoke `useState`+`useEffect`+`invoke`.
  *verify:* the marker tracks rapid dirty→clean→dirty flips at boot correctly.

- [ ] **FE-C4 — `listenPlateEdits` is needlessly `async`; the await opens a mount/unmount cleanup race** *(LOW; ponytail/arch)*
  `src/project/editEvents.ts:61-76`, consumers `src/slice/useLastSliceOutput.ts:59-76`, `src/preview/useSlicePreviewBridge.ts:111-125`.
  Declared `async`/returns a Promise but the body is fully synchronous (three `onEvents` + a sync unsubscribe — its own comment says so). Both consumers must do the `let unlisten=null; void(async()=>{unlisten=await …})(); return ()=>unlisten?.()` dance. Under StrictMode the cleanup runs before the microtask resolves (`unlisten` still null → nothing unsubscribed), then the microtask assigns to the orphaned first-mount registration → first mount's handlers leak for the app lifetime. Becomes a real production leak the day it gains genuine async work.
  *Action:* make `listenPlateEdits` synchronous (return the combined unsubscribe, like `onEvents` already does); both effects call it inline: `useEffect(() => listenPlateEdits(onPlate, onAll), [])`. Removes the wrapper, both await-dances, and the race.
  *verify:* dev StrictMode double-mount leaves exactly one live registration.

- [ ] **FE-C5 — `useSlicePreviewBridge` async listener can leak if unmounted before `listenPlateEdits` resolves** *(LOW; arch)*
  `src/preview/useSlicePreviewBridge.ts:91-125`.
  Assigns `unlisten` only after the promise resolves; cleanup reads it synchronously, so an unmount-before-resolve leaks the listener. Doesn't fire today (mounted once at App level, App never unmounts) but latent the moment an unmountable component uses the bridge. Subsumed by FE-C4's synchronous rewrite; otherwise adopt the `cancelled`-flag pattern App already uses for `setupLogSinks` (`App.tsx:209-219`).
  *Action:* fixed for free by FE-C4, or add the cancel-safe flag.
  *verify:* covered by FE-C4's check.

---

## Accepted invariants — re-confirmed, not re-flagged

Held to the settled list and re-checked: slot labels Rust-owned; cascade
resolution Rust-owned (FE-C1/FE-X2 are *violations* of this invariant, not
challenges to it); model materials = abstract slot indices; Objects panel
primitives-only; header device controls follow the active plate; topbar
buttons-only; `queryCache` no-GC and `eventRouter` app-lifetime entries are
accepted deferrals (no standalone finding); the frontend `driverId↔instance` map
(C-14) is intended bookkeeping; `.n3o` native / 3MF import-only; R-8's god-file
split is done (no generic "split App.tsx" re-flag). Refuted-and-dropped:
`DropZone` "duplicates `driverErrorMessage`" (it doesn't).
