# Getting started with n3o-slic3r

n3o-slic3r is a multi-printer-first desktop slicer. You load models,
assign each plate to a printer, slice, preview the G-code, and send it
to the printer over your LAN — all in one app. **You don't need any
other slicer installed**: every step of the workflow happens here.

The MVP supports two printers, configured at the same time:

- **Bambu Lab A1 mini** (AMS lite, single hotend, LAN over MQTT)
- **Snapmaker U1** (4-toolhead toolchanger, LAN over Moonraker HTTP)

This guide takes you from a clean Linux machine to your first print.

---

## 1. What you need

- A 64-bit Linux desktop with [Flatpak](https://flatpak.org/setup/)
  installed and the Flathub remote configured (the app's runtime comes
  from Flathub):

  ```sh
  # Most distros: install flatpak via your package manager, then:
  flatpak remote-add --if-not-exists flathub \
    https://flathub.org/repo/flathub.flatpakrepo
  ```

- A GPU with working OpenGL/Vulkan drivers (the 3D viewport needs
  hardware acceleration; software rendering works but is slow).
- At least one supported printer (A1 mini or U1) reachable on the same
  LAN as your computer, powered on, and in LAN mode.

Tested on recent Ubuntu, Fedora, and Arch with current Flatpak
runtimes. It also runs under WSL2 with WSLg as a best-effort target —
see [Troubleshooting](troubleshooting.md) for the WSL2 networking
caveat.

---

## 2. Install

Install from the published `.flatpakref` (this pulls the app's repo and
GPG key automatically):

```sh
flatpak install --from \
  https://n3o.thegraveyard.org/repo/org.thegraveyard.n3o-slic3r.flatpakref
```

Then launch it:

```sh
flatpak run org.thegraveyard.n3o-slic3r
```

(It also appears in your desktop's application menu as **n3o-slic3r**.)

To update later, `flatpak update org.thegraveyard.n3o-slic3r`.

> **Arch users** can alternatively build a native package from
> `packaging/arch/` (see `packaging/arch/README.md`). The Flatpak is the
> recommended path; the rest of this guide assumes it.

---

## 3. First run — add your printer

The first time you launch, n3o-slic3r shows a welcome screen because no
printers are configured yet. Click **Add printer**.

1. **Pick your printer.** Search or scroll the catalog and select
   **Bambu Lab A1 mini** or **Snapmaker U1**. The panel previews the
   bed size and capabilities.
2. **Name it** (e.g. "Garage A1") — useful once you have more than one.
   If the printer supports AMS units, pick how many you have.
3. Click **Add**. The printer now exists in your library, but it isn't
   connected yet.

### Enter the connection details

Open the new printer's **Settings** and go to the **Connection** tab.
What you enter depends on the printer:

**Bambu Lab A1 mini**
- **Host**: the printer's IP address on your LAN.
- **Access code**: the 8-character LAN access code. Find it on the
  printer: *Settings → WLAN/Network → LAN Mode* shows the access code.
- The serial number is detected automatically — you don't type it.
- This is a pure LAN connection: **no Bambu cloud account is used or
  required.**

**Snapmaker U1**
- **Host**: the printer's IP address on your LAN.
- **Port**: the Moonraker port (default **80** — leave it unless you've
  changed it).

Click **Test connection**. A green result means you're good. If it
fails, see [Troubleshooting](troubleshooting.md) → *Printer not
reachable*. Save the settings.

The printer's status dot (and live temperatures, once connected) appear
in the **Devices** view.

---

## 4. Load a model

In the **prepare** workspace, using the **Objects** panel on the left:

- **Add a model file.** Click **Add model…** (or the viewport's load
  button) and pick an STL, OBJ, or 3MF file. Geometry, positions, and
  per-part material hints are imported.
- **Or drop in a shape.** The **Primitives** section (cube, cylinder,
  sphere, cone, torus) adds a ready-made object to the active plate —
  handy for a quick test print.

Move, rotate, scale, lay-flat, or auto-arrange objects with the toolbar
gizmo and the per-object controls.

> Opening a **project** from Bambu Studio / OrcaSlicer / Snapmaker Orca
> (a `.3mf` project file) is also supported via **Open project** — it
> reconstructs geometry, plate layout, and the project's settings. This
> is an optional one-time migration convenience; the other slicer does
> not need to be installed.

---

## 5. Assign materials to slots

n3o-slic3r treats a model's materials as **abstract indices** (material
1, 2, 3, …). What each one actually prints as is whatever filament is
loaded in the slot you bind it to — **the printer's loadout is the
source of truth, not the model.** There is no "the model wanted PETG"
to fight with.

In the materials/slot panel, each model material shows a picker; bind it
to a physical slot, and the slot's currently-loaded filament is shown
inline so you can see exactly what it will print as. A first binding is
suggested automatically; adjust it freely. If a bound slot has no
filament loaded (or the slot is unavailable), n3o-slic3r blocks the
slice and tells you which one to load or rebind.

For a single-material print on the A1 mini, the default binding is
usually all you need.

---

## 6. Slice

Click **Slice** in the top bar. It slices the **active plate** against
its bound printer's full settings cascade. Progress appears in a
floating window over the canvas; a slice that can't start (e.g. no
printer bound, or an unloaded slot) tells you why in the error console.

When it finishes, the view switches to **preview** automatically.

---

## 7. Preview the G-code

The preview is a full in-app G-code viewer — no external tool needed:

- **Layer slider** (single layer, up-to-N, or a layer range) with arrow
  keys for next/previous layer.
- **Color modes**: feature type, speed, flow, layer time, or tool index.
- **Hover** any segment to see its command, position, speed, feature,
  and layer.
- **Toggle** travels and retractions.
- **Stats**: per-layer (time, filament, max speed, height) and full-job
  (time per feature, filament per extruder, layer count, bounding box).

You can also preview any G-code from disk: drag a `.gcode` or
`.gcode.3mf` file onto the preview.

---

## 8. Send to the printer

Back on the active plate, click **Send**. The driver wraps the slice in
whatever format the bound printer needs (`.gcode.3mf` with the right
metadata for the A1 mini; plain `.gcode` for the U1) and uploads it over
the LAN. Watch the job — state, current layer, temperatures — in the
**Devices** view; **pause / resume / stop** are there too.

Prefer to handle the file yourself? **Export** writes the exact bundle
n3o-slic3r would send to a location you choose.

> The Flatpak ships with LAN network access so it can reach your
> printer. If a send fails to connect, see
> [Troubleshooting](troubleshooting.md).

---

## 9. Multiple printers and plates (optional)

A project can hold up to 4 **plates**, and **each plate has its own
printer**. Add plates from the plate tab strip and assign a printer per
plate — slice Plate 1 for the A1 mini and Plates 2–3 for the U1 in the
same project. Material→slot bindings are stored **per (plate, printer)**,
so reassigning a plate to a different printer surfaces that printer's own
loadout. Changing a plate's printer re-resolves the cascade and flags any
settings that don't carry over — as warnings, never silent changes.

---

## Where to go next

- **Understand a setting's value.** Every setting shows where its value
  came from in the cascade — hover the row to open the ladder (printer →
  build plate → nozzle → filament → user → project → object) with the
  winning layer highlighted. That's the settings explainer; there's no
  separate manual to memorize.
- **Write a plugin.** n3o-slic3r runs sandboxed Lua plugins that
  transform G-code (the bundled **platecycler** plugin auto-ejects a
  finished plate on an A1 mini + PlateCycler). See the
  [plugin authoring guide](plugin-authoring.md).
- **Hit a snag?** See [Troubleshooting](troubleshooting.md).
