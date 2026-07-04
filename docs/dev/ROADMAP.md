# Roadmap — post-MVP / road to 1.0

The MVP candidate ships (Phases 0–9 complete). None of the items below
block the MVP — this is the path to a canonical 1.0. The *planned*
post-MVP deferrals (plugin compose hook, hot reload, Orca preset
importer) live in `Execution_Plan.md` §16; this file tracks what's left
to reach 1.0, mostly surfaced from real use.

## Features

### UI optimizations
TBD — to discuss. Placeholder; capture specifics here as they're named.

### Generic Klipper/Moonraker webcam source
The live webcam ships for both MVP printers (Bambu LAN push; Snapmaker U1
Moonraker poll + paired mTLS wake). A generic Moonraker/Klipper MJPEG
source would cover plain Klipper printers — the U1's poll-and-wake path is
Snapmaker-specific.

### Auto-arrange packing polish
Auto-arrange drives libslic3r's libnest2d nester with real exclusion-zone
and prime/wipe-tower avoidance, spilling onto extra plates. Polish:
- Turn on `allow_rotations` for tighter packs (the FFI already supports it).
- Bound the hull cost on dense scenes — hull the local mesh once, then
  transform (convex hull commutes with affine maps).

## Packaging & distribution

### Windows build — CI + signing
The Windows build cross-compiles end-to-end from Linux — deps → engine →
`slic3r_ffi.dll` → app → NSIS installer, no Windows host and no wine (see
`packaging/windows-cross/README.md`). Remaining: wire it into CI, and add
an Authenticode signer (`bundle.windows.signCommand`). Neither is an
unknown.

### Arch publish — verify end-to-end
`packaging/arch/publish.sh` builds a signed `.pkg.tar.zst` for a bare
`pacman -U`. It's written, `bash -n`-clean, and package-name resolution is
confirmed — but the full `makepkg` build and the upload are untested. Run
it on a real Arch box to confirm.
