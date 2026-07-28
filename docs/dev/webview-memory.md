# Webview memory: the 16 GB kill, and what's left

WebKit kills its own web process past a footprint ceiling:

```
Unable to shrink memory footprint of process (16450 MB) below the kill thresold (16384 MB). Killed
```

(the typo is WebKit's, in `MemoryPressureHandler`). Sessions died this way after
running unattended — usually noticed after the screen had been off. The UI
process dies; the backend survives.

Two causes were found and fixed. A third, smaller one is open.

## The mechanism that made it fatal

**WebKit suspends the JS heap's collector while a page isn't painted, and
nothing suspends our producers.** A steady allocation drip is invisible while
you're looking at the window — GC keeps up, RSS shows sawtooth — and becomes a
straight line to the kill ceiling once the screen locks.

Measured twice, identically:

| | rate |
|---|---|
| overnight, screen off, locked | 236 MB/min → killed at 16 GB |
| deliberate screen lock, 21 min | 235 MB/min (638 → 5575 MB) |

The Rust process stayed flat at ~450 MB throughout both. The fatal state is
therefore *always* webview-side, and any producer that keeps allocating while
frozen can reach it. `src/state/pageActivity.ts` is the defence: it detects
"not painted" from a `requestAnimationFrame` heartbeat (rAF stops exactly when
GC stops; `document.visibilityState` is a weaker proxy — under a Wayland
compositor a locked screen can leave the document "visible") and consumers
decline to produce until it resumes.

## Cause 1: telemetry re-rendered the whole app

Moonraker/Klipper pushes a status notification several times a second (temps
drifting a tenth of a degree). `useDriverConnections`' global listener bumped its
state version on *every* one, which invalidated the connection-summary snapshot
cache, which changed the snapshot identity, which re-rendered App's entire tree:

```
driver:status_update = 8/s  →  16-17 App renders/s  →  ~4 MB/s of garbage
```

Fixed by notifying only when the connection state actually changed, plus
coalescing telemetry-only events to 500 ms in the status bridge
(`spawn_status_bridge`). Structural changes — connection state, job state —
still emit immediately. After: **0 renders per event** with events still
arriving, and RSS flat.

## Cause 2: no bound on production while frozen

Fixing one producer left the class open — the camera stream was already a second
instance (4 fps of JPEG blobs, push-based, no backpressure). The event router
now holds latest-value-only events while frozen (one per driver, so memory is
bounded however long the freeze lasts) and replays them on the first painted
frame; the camera drops frames nobody can see. Events carrying a *transition*
are never held: a slice finishing while the screen is locked still has to load
its preview.

## Open: ~20-40 MB retained per slice

Real but slow, and **not** the thing that was killing sessions. A restart clears
it; ~600 slices to reach the ceiling.

Measured on a fresh app, floor = post-GC minimum, six slices per batch:

| batch | floor before → after | per slice |
|---|---|---|
| 1 | 398 → 564 MB | 28 MB |
| 2 | 564 → 714 | 25 |
| 3 (preview path disabled) | 685 → 932 | 41 |
| 4 (progress dispatch dropped) | 950 → 1049 | 17 |

It does not plateau across 18 slices, so it is retention rather than heap
high-water. Transient peaks (+300 MB per slice) *are* collected; the floor
isn't.

### Ruled out

- **Frame/image buffers.** New regions after a slice are all different sizes
  (3–46 MB), which is allocator arenas, not uniform RGBA frames.
- **The preview path** (`previewLoad` + `PreviewWorkspace` mount). Disabling
  both made the measurement *worse* — see the noise caveat below.
- **Leaked subscriptions.** Router handler counts go 24 → 39 during a slice
  (preview mounting) → 25 after: one net, and it's a query-cache entry, which
  never unsubscribes by design.
- **The cascade resolve** (`usePlateCascadeResolve`): local state, replaced per
  fetch, doesn't refetch per slice.
- **Preview response + per-layer stats**: kilobytes.
- **The FFI / Rust side.** Different address space; it cannot allocate into the
  webview heap, and per-slice IPC is a path plus a summary. The Rust process's
  own curve is the *benign* shape for comparison: 443 → ~1192 MB over the first
  few slices, then flat at 1192 for 15 minutes and +36 MB over six more. That is
  allocator high-water, and it is what the webview curve does *not* look like.

### Why it stalled

Round-to-round comparison is unreliable: each batch started from a different
heap size (398 / 564 / 685 / 950 MB) and the increment appears to scale with
heap size, so single-variable bisection sits inside the noise band. Batch 3
removing a suspect and getting a *bigger* number is the tell.

Isolating further needs a heap profiler with retainer paths, which this
WebKitGTK build can't give: the inspector's Timelines offers only "JavaScript &
Events", no allocations instrument, and `WEBKIT_INSPECTOR_SERVER` opens a socket
that speaks a private protocol (not HTTP/CDP — curl gets nothing), so it can't
be driven programmatically.

### Next step if revisited

Run the frontend in Chrome against a dev-only Tauri shim and synthesise the
slice event stream (a `plate_started`, ~600 `plate_progress` at 20/s, a
`plate_finished`). Chrome's profiler then gives retained-object classes and
retaining paths directly, with no dependency on real slicing. The leak should
reproduce there because nothing about it needs the real backend — only the event
stream and the components that react to it.

## Measurement recipe

The trap is reading a single RSS sample: GC sawtooth is ±300 MB. Always compare
*floors* (post-collection minima) sampled for a minute or more after the
activity stops.

```bash
web=$(pgrep -f WebKitWebProcess | head -1)          # NB: a second one exists
rust=$(pgrep -f "target/(debug|release)/n3o-slic3r$" | head -1)  # when devtools are open
awk '{printf "%.0f MB\n", $2*4/1024}' /proc/$web/statm           # RSS, cheap to poll

# allocation shape: private-anon regions by size (frame buffers vs arenas)
awk '/^[0-9a-f]/{n=$6} /^Rss:/{if($2>2048 && n=="") print $2}' /proc/$web/smaps | sort -rn

# is it JS at all? private-anon vs file-backed
grep -E "^(Rss|Anonymous)" /proc/$web/smaps_rollup
```

To get render rate and event names out of the webview without devtools: a
temporary counter in App plus a router-side per-name tally, reported by
`fetch()` to a local `python3 -m http.server` sink, read from its access log.
That is what named `driver:status_update` as the 8 Hz source. Note HMR does not
always reach a running page — verify the probe reports *before* trusting any
bisection result, or the whole exercise measures an unchanged app.
