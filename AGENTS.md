# AGENTS.md — operational runbook for n3o-slic3r

Durable project context (architecture, domain facts, build) lives in
**`CLAUDE.md`** and `docs/dev/`. This file is the practical runbook for
*operating* the app from an agent session: launching it and capturing
screenshots headlessly.

## Running the app

n3o-slic3r is a **Tauri 2** app: a Rust backend (`src-tauri/`) driving a
**WebKitGTK** webview that loads the React frontend from a **Vite** dev
server on **port 1420**.

```bash
npm run tauri dev
```

This is the blessed entrypoint. It:
- runs `beforeDevCommand` (`npm run dev` → Vite on 1420),
- builds + launches the Rust backend (incremental; fast once warm),
- loads **`.env`** via `dotenv` (the `npm run tauri` script does this) —
  critically setting `WEBKIT_DISABLE_DMABUF_RENDERER=1` and
  `N3O_SLIC3R_RESOURCES_ROOT=./resources`.

### The Wayland gotcha (don't skip the env)

Launching the **bare debug binary** (`./target/debug/n3o-slic3r`) directly
on a Wayland session **crashes** with:

```
Gdk-Message: Error 71 (Protocol error) dispatching to Wayland display.
```

The fix is `WEBKIT_DISABLE_DMABUF_RENDERER=1` (the dmabuf renderer doesn't
work on every compositor, e.g. Hyprland). `npm run tauri dev` sets it from
`.env`; if you run the binary yourself, set it yourself. The debug binary
is a *dev* build — it loads the frontend from the Vite dev URL (1420), so
**Vite must be running** (`npm run dev`) when you launch it standalone.

### Process-kill gotcha

`pkill -f "target/debug/n3o-slic3r"` matches **its own command line** (which
contains that string) and kills the invoking shell — you'll see a spurious
exit code **144**. Use exact-name matching instead:

```bash
pkill -x n3o-slic3r
```

## Screenshotting headlessly

Use a **nested X server (Xvfb) + xdotool**. This is the lowest-friction
path and the right one on a Wayland host:

- **xdotool is display-scoped** (`DISPLAY=:99 xdotool …`) — input goes only
  to the nested server, so it can't leak into the host session (which may be
  locked / in use).
- WebKitGTK runs fine on X11 via `GDK_BACKEND=x11`.

### One-time install (Arch, official `extra` repo — no AUR/build)

```bash
sudo pacman -S --needed xorg-server-xvfb xdotool
# `import` (imagemagick) for capture is usually already present; `scrot` also works.
```

### Recipe

```bash
# 0. Vite must be serving the frontend (own terminal, or background):
npm run dev &                       # → http://localhost:1420
# (a debug binary must already exist: `cargo build -p n3o-slic3r`)

# 1. Nested X server:
Xvfb :99 -screen 0 1600x1000x24 &

# 2. App on :99 (X11 backend + the dmabuf workaround + resources root):
DISPLAY=:99 GDK_BACKEND=x11 WEBKIT_DISABLE_DMABUF_RENDERER=1 \
  N3O_SLIC3R_RESOURCES_ROOT=./resources \
  ./target/debug/n3o-slic3r &
sleep 8                              # libslic3r init + first render

# 3. Drive it (window class/name is "n3o-slic3r"):
DISPLAY=:99 xdotool search --name n3o-slic3r windowactivate
DISPLAY=:99 xdotool mousemove 1007 530 click 1     # coords = screenshot pixels
DISPLAY=:99 xdotool type "My PLA (hot)"
DISPLAY=:99 xdotool key Return

# 4. Capture (root window of :99 == the app, full-screen window):
DISPLAY=:99 import -window root /tmp/shot.png
#   or: DISPLAY=:99 scrot /tmp/shot.png
```

Read each PNG back and confirm before the next click — a blank/solid frame
means the app didn't render (check Vite is up and the dmabuf var is set).

### App-state notes for navigation

- The app uses the **real user library** at `~/.config/n3o-slic3r/`
  (`printers/`, `filaments/`). With printers configured it opens straight
  into the populated workspace; with none it shows onboarding.
- On launch with prior autosaves it shows a **"Recover unsaved projects"**
  dialog — dismiss it (Keep/Discard) before driving the main UI.
- To reach the **filament editor**: main view → **SLOTS** row (top-right,
  e.g. `1 PLA · 2 PLA · …`) → click a slot → **FilamentPickerModal** →
  *Duplicate* a bundled filament (or *Edit* a user one) → **FilamentSettingsModal**.

### Cleanup

```bash
pkill -x n3o-slic3r
pkill -x Xvfb
# free Vite's port if you backgrounded it:
fuser -k 1420/tcp
```

### Why not the alternatives (Wayland host)

- **ydotool** injects at the kernel/uinput level → **global**; it lands in
  the host compositor (e.g. a locked Hyprland), not a nested server. Not
  display-scoped, so unusable for isolated automation.
- **sway headless** renders fine but, with `WLR_LIBINPUT_NO_DEVICES=1`,
  advertises **no input capability** (the webview never binds a pointer);
  injecting needs the virtual-pointer protocol via `wlrctl` (**AUR-only**)
  or a hand-built client (`wlr-protocols` XML isn't installed by default).
- **Xephyr** nests into a *visible* window — hidden behind a host lock
  screen. Xvfb is fully headless and sidesteps that.
