# PR-9-2 — Linux flatpak build

Status: 🟡 builds + runs on Arch (2026-05-31); multi-distro + GPU-visual
confirmation pending. The phase's **long pole** — core deliverable done.

> **Outcome.** `packaging/flatpak/` builds a flatpak that compiles
> everything in-sandbox against the runtime, bundles `libslic3r_ffi.so` +
> the resources tree, and launches with the embedded production frontend.
> Validated on Arch: builds, installs (`flatpak install`), and runs — the
> backend fully initializes (libslic3r, the env-pointed resource root,
> profile + instance libraries, plugin host) and the **project lead
> visually confirmed the n3o UI loads** (the embedded production
> frontend, not a stray dev server).
>
> Divergences from this ticket's original wording, recorded per PRD §11.3:
> - **Runtime is GNOME 50 (freedesktop 25.08), not the bare Freedesktop
>   SDK.** Only the GNOME runtime ships the webkitgtk Tauri needs on
>   Linux. Toolchain via rust-stable + node22 + **llvm21** SDK
>   extensions (llvm21 supplies libclang for slic3r-ffi's bindgen).
> - **Two extra modules**: `glu` (the SDK/runtime omit libGLU, which
>   GLEW needs) and `orca-deps` (the dep tree, cached independently).
> - **`WEBKIT_DISABLE_DMABUF_RENDERER=1`** in finish-args — the webkit
>   DMABUF renderer trips over NVIDIA/Wayland (protocol error 71).
> - Must build via **`tauri build`**, not bare `cargo build`, or the
>   webview loads `devUrl` (localhost:1420) instead of the embedded UI.
>
> **Still open:** clean install+run on **Ubuntu and Fedora** (needs
> those environments — folds into the PR-9-8 audit); a visual check that
> the 3D viewport renders with the DMABUF renderer off; the WSL2
> best-effort note; and the Flathub-shape offline-vendored manifest
> (post-MVP).

**Scope.** Produce a flatpak that bundles the Tauri app, its
`libslic3r_ffi.so`, the webview/runtime deps, and the `resources/`
tree, and runs the full workflow on a clean Linux box. Starts from the
known-good Tauri bundle (`packaging/arch/` proves the build produces a
desktop binary + bundled FFI + resources).

**Acceptance criteria.**

- **Flatpak manifest** (`packaging/flatpak/<app-id>.yml` or `.json`):
  - Runtime: **Freedesktop SDK** (pick and pin a current version);
    justify the choice in a comment (vs. GNOME/KDE runtimes).
  - Builds or vendors `libslic3r_ffi.so` and its transitive deps so the
    app is self-contained — no system OrcaSlicer, no host libslic3r
    (PRD §5, standalone at runtime).
  - Bundles the `resources/` tree into the app's resource dir so the
    profile + plugin libraries load with `N3O_SLIC3R_RESOURCES_ROOT`
    **unset** (production path = Tauri `resource_dir()`; see
    `lib.rs::resources_root`).
  - Webview: the WebKitGTK / wry deps the Tauri 2 Linux build needs,
    resolved inside the sandbox.
- **GPU acceleration** works for the 3D viewport via flatpak hardware
  permissions (`--device=dri`); document the finished permission set
  (`finish-args`) and why each is needed (DRI, network for printer LAN
  comms only, filesystem for project open/save).
- **Network policy:** the sandbox allows only what the product
  principle permits — user-configured printers on the LAN. No
  telemetry/analytics host reachable (PRD §11.5). Document the
  `--share=network` scope and that it carries no outbound analytics.
- **Builds reproducibly** from a documented command; the artifact
  installs with `flatpak install` and launches.
- **Runs cleanly on Ubuntu, Fedora, and Arch** with current flatpak
  runtimes (the §11 exit criterion). Record the tested runtime
  versions.
- **WSL2 best-effort** (scope decision 3): note whether the flatpak
  runs under WSLg and document known limitations (printer LAN comms via
  WSL2 NAT may need user-side network setup). Not a blocker.

**Effort.** ~4 days (flatpak runtime/permission fiddliness + the
libslic3r bundling are the time sinks).

**Dependencies.** None hard; the Tauri build already works. Best paired
with PR-9-3 (distribution) once an artifact exists.

**Out of scope.**

- Windows / macOS native builds — post-MVP (§11 explicitly).
- Flathub submission — post-MVP (PR-9-3 ships self-hosted only).
- Code signing / notarization beyond what flatpak requires.
