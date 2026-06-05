# n3o-slic3r — release notes

## 0.1.0 — MVP candidate

The first public build of n3o-slic3r: a multi-printer-first desktop
slicer for Linux. It runs the complete slice workflow on its own — load,
slice, preview, send, monitor — with **no other slicer required at any
point.**

### Supported printers

- **Bambu Lab A1 mini** — AMS lite, single hotend. LAN over MQTT
  (access code + serial; no cloud account). Sent as `.gcode.3mf`.
- **Snapmaker U1** — 4-toolhead CoreXY toolchanger. LAN over Moonraker
  HTTP. Sent as plain `.gcode`. Per-toolhead nozzle/hotend config.

Both can be configured and used in the same project.

### What's in it

- **Multi-printer, multi-plate projects.** Up to 4 plates, each bound to
  its own printer; per-(plate, printer) material→slot bindings.
- **Transparent settings cascade.** Every setting shows where its value
  came from — hover any row for the layer ladder (printer → build plate →
  nozzle → filament → user → project → object) with the winning layer
  highlighted, plus per-object overrides.
- **Filament as slot-truth.** Model materials are abstract indices you
  bind to physical slots; the slot's loaded filament is what prints. Live
  filament read from each printer; manual override for third-party spools.
- **In-app G-code preview.** Layer slider (single / up-to-N / range),
  color modes (feature, speed, flow, layer time, tool), hover inspection,
  travel/retraction toggles, per-layer and full-job stats, colour-blind-safe
  palette. Opens any `.gcode` or `.gcode.3mf` from disk too.
- **Slice and send.** Off-thread slicing with live progress; send and
  monitor (state, layer, temps, pause/resume/stop) per printer.
- **Lua plugin system.** Sandboxed G-code plugins (pre-slice / post-slice
  / pre-send) over a typed G-code model. Ships with **platecycler**, which
  auto-ejects a finished plate on an A1 mini + PlateCycler. See the
  [plugin authoring guide](plugin-authoring.md).
- **3D viewport.** Load STL / OBJ / 3MF; move / rotate / scale / mirror /
  lay-flat / auto-arrange; primitives; per-printer bed + exclusion zones.
- **Open foreign projects.** Import a Bambu Studio / OrcaSlicer /
  Snapmaker Orca `.3mf` **project** (geometry + the project's settings) as
  a one-time migration. The other slicer does not need to be installed.
- **Linux Flatpak** with self-hosted distribution and first-run onboarding.

### Install

See the [getting-started guide](getting-started.md). In short:

```sh
flatpak install --from \
  https://thegraveyard.org/n3o-slic3r/org.thegraveyard.n3o-slic3r.flatpakref
flatpak run org.thegraveyard.n3o-slic3r
```

---

## Known issues and limitations

- **Linux only.** Native Windows and macOS builds are post-MVP.
- **WSL2 is best-effort.** The app runs under WSLg, but reaching a printer
  on the host LAN needs mirrored networking or port forwarding — see
  [Troubleshooting](troubleshooting.md).
- **Self-hosted distribution.** Installed from a `.flatpakref` + repo;
  Flathub submission is post-MVP.
- **Plugins load on launch.** Use the manual reload after editing a
  plugin; automatic file-watch hot reload is post-MVP.
- **No calibration-object library.** The Objects panel ships Primitives
  and your imported models; bundled calibration fixtures (temperature /
  flow / retraction towers) are not included in this release.
- **Preset/profile import is not in the UI.** You can open a foreign
  *project* (above), but importing OrcaSlicer machine/filament/process
  *presets* through the UI is post-MVP. The two supported printers ship
  with first-class profiles, so you don't need it to get started.
- **Material auto-binding is positional.** On first assignment, materials
  bind to slots by index; review and adjust in the materials panel.
- **Viewport performance on low-end GPUs is unverified.** It targets 30 fps
  on integrated graphics but has only been validated on a discrete GPU;
  software rendering works but is slow.
- **Mixed-nozzle-size U1 prints** are supported in the data model but not
  validated with real prints in this release.
- **Logs go to stderr.** Run from a terminal to see them (there's no log
  file yet); the in-app error console shows slice/send failures.

## Explicitly out of scope for the MVP (post-MVP)

Native Windows/macOS, Flathub submission, plugin hot reload, the plugin
**compose** hook and multi-plate batch PlateCycler, OrcaSlicer
preset/profile import, calibration wizards, paint-on supports, camera
streams, filament inventory, cloud/accounts, and any AI features.
