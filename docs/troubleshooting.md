# Troubleshooting

Concrete fixes for the problems a first-time installer actually hits.
If something here doesn't resolve it, the logs (below) usually say why.

## See what's happening (logs)

n3o-slic3r writes structured logs to **stderr**. The quickest way to see
them is to launch from a terminal instead of the app menu:

```sh
flatpak run org.thegraveyard.n3o-slic3r
```

Errors and warnings print there as you use the app. For more detail, raise
the log level with `RUST_LOG`:

```sh
# Everything at debug level:
RUST_LOG=debug flatpak run org.thegraveyard.n3o-slic3r

# Just the Bambu MQTT traffic (raw AMS / status JSON):
RUST_LOG=mqtt=debug flatpak run org.thegraveyard.n3o-slic3r
```

The app also has an in-window **error console** (it pops open on errors)
— check it first for slice/send failures.

---

## The app won't start, or the 3D view is black / very slow

The viewport needs working GPU acceleration. The Flatpak is granted GPU
access (`--device=dri`), so this is almost always a **host driver**
problem, not a permission one.

1. **Launch from a terminal** (above) and look for OpenGL/EGL errors near
   startup.
2. **Confirm your GPU drivers work** on the host — e.g. `glxinfo | grep
   "OpenGL renderer"` (install `mesa-utils` / `glx-utils`). If that fails
   or shows `llvmpipe`, you're on software rendering; fix the host driver
   (Mesa for AMD/Intel, NVIDIA's proprietary driver for NVIDIA).
3. **NVIDIA**: make sure the Flatpak NVIDIA runtime extension matches your
   driver — `flatpak update` usually pulls the right
   `org.freedesktop.Platform.GL.nvidia-*` automatically; if the window is
   black, a mismatched driver version is the usual cause.
4. A slow-but-working viewport (software rendering) still lets you slice
   and send — it's a performance issue, not a blocker.

---

## Can't reach the printer ("Test connection" fails, status stuck on Connecting)

The Flatpak has LAN network access (`--share=network`), so a failure here
is almost always network configuration or the connection details. The app
**keeps retrying** with a backoff that caps at 60 s, so a printer that
comes online later will connect on its own — but to fix it now:

1. **Same network.** Your computer and the printer must be on the same LAN
   / subnet. From a terminal, confirm you can reach it at all:
   `ping <printer-ip>`.
2. **Check the connection details** (printer **Settings → Connection**):
   - **A1 mini**: the **host** is the printer's IP; the **access code** is
     the 8-character code under *Settings → WLAN/Network → LAN Mode* on the
     printer. The printer must be in **LAN mode**. No cloud account is
     involved.
   - **U1**: the **host** is the printer's IP and the **port** is the
     Moonraker port (default **80**). Confirm Moonraker answers:
     `curl http://<printer-ip>:80/printer/info` should return JSON.
3. **Firewall.** A host firewall can block the MQTT (8883, A1 mini) or
   HTTP (80, U1) connection. Allow outbound to the printer's IP.
4. **Re-test.** Use the **Test connection** button after each change — it
   reports the specific failure (refused, timed out, auth).

### WSL2

Under WSL2 the printer is on the **host** Windows LAN, not inside WSL2's
NAT'd network, so the app often can't see it by default. Either enable
**mirrored networking** (`networkingMode=mirrored` in `.wslconfig`, recent
Windows builds) or set up port/route forwarding from Windows to WSL2. This
is a known WSL2 limitation, not an app bug — WSL2 is a best-effort target.

---

## The Slice button is greyed out, or a slice won't start

If **Slice** is disabled, hover it — the tooltip names the blocker:

- **"bind a printer to this plate first"** — assign a printer to the active
  plate (each plate prints on one printer).
- **"add an object before slicing"** — the plate is empty; add a model or a
  primitive.
- **"loading project…"** — the project is still initializing; wait a moment.

If Slice is enabled but the slice **fails to start**, the reason appears in
the error console. The most common one: a model material is bound to a slot
that has **no filament loaded** (or the slot is unavailable). n3o-slic3r
blocks the slice rather than print with an unknown filament — open the
materials panel, and either load/identify the filament in that slot or
rebind the material to a slot that has one. (Remember: the slot's loaded
filament is what prints; the model doesn't carry an intended filament.)

---

## Send fails after a successful slice

The slice succeeded but **Send** errors or does nothing:

1. The printer must be **connected and idle**. Check its status in the
   **Devices** view — a green dot and live temperatures mean it's reachable.
   A red/"failed" dot means the connection dropped; fix it as under *Can't
   reach the printer* above.
2. After a successful send, Send is briefly disabled while the app waits
   for the printer to pick the job up — that's expected, not a failure.
3. If you'd rather move the file by hand, use **Export** to write the exact
   `.gcode.3mf` (A1 mini) or `.gcode` (U1) bundle to disk and transfer it
   yourself.

---

## Still stuck?

Re-run from a terminal with `RUST_LOG=debug` and capture the output around
the failure — it almost always names the cause. The
[getting-started guide](getting-started.md) covers the happy path end to
end if you want to retrace a step.
