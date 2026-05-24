#!/usr/bin/env python3
"""Send a .gcode.3mf to a Bambu printer using bambu-connect's exact
command shape. Faster iteration loop than rebuilding our Rust app:
pick a known-working open-source impl, run it bytes-identical
against the live printer, see whether it works.

If THIS accepts → bambu-connect's shape is correct, our Rust diff
is the bug. Copy the shape verbatim.

If THIS rejects with the same "MQTT command invalid" → the shape
is firmware-stale and we need the bpftrace capture from a real
Bambu Studio send.

Requires:
  - `pip install paho-mqtt==1.6.1` (bambu-connect's pinned version)
  - `curl` on PATH (used for FTPS upload, mirroring FileClient's
    own curl invocations).
  - `bambu-connect` cloned at `~/src/third/bambu-connect` (or
    pass --bambu-connect-path).

Usage:
  bambu_send_test.py <host> <access_code> <serial> <local.gcode.3mf>

  Optional:
    --remote-name <name>        — name the file gets on the printer
                                  (default: same basename as local).
    --bambu-connect-path <dir>  — path to a checkout of
                                  mattcar15/bambu-connect.
    --skip-upload               — skip the FTPS upload step (use
                                  when the file is already on the
                                  printer from a prior run).
"""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
import time
from pathlib import Path

DEFAULT_BAMBU_CONNECT = Path.home() / "src/third/bambu-connect"


def upload_via_ftps(
    host: str,
    access_code: str,
    local: Path,
    remote_name: str,
) -> None:
    """FTPS upload via curl to the printer's root. Mirrors the style
    of bambu-connect's FileClient (which also shells out to curl)
    and matches our Rust driver's send path."""
    target = f"ftps://{host}/{remote_name}"
    cmd = [
        "curl",
        "--ftp-pasv",
        "--insecure",
        "-T",
        str(local),
        target,
        "--user",
        f"bblp:{access_code}",
    ]
    print(f"[ftps] uploading {local.name} → {target}")
    result = subprocess.run(cmd, capture_output=True, text=True)
    if result.returncode != 0:
        print(f"[ftps] curl failed: {result.stderr.strip()}", file=sys.stderr)
        sys.exit(2)
    print("[ftps] upload OK")


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__,
                                formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("host", help="Printer IP (or .local name)")
    p.add_argument("access_code", help="8-digit LAN access code")
    p.add_argument("serial", help="Printer serial (CN of its peer cert)")
    p.add_argument("local_3mf", type=Path,
                   help="Path to a .gcode.3mf bundle on disk")
    p.add_argument("--remote-name", default=None,
                   help="Filename on the printer side (default: basename of local_3mf)")
    p.add_argument("--bambu-connect-path", type=Path, default=DEFAULT_BAMBU_CONNECT,
                   help="Where bambu-connect is cloned")
    p.add_argument("--skip-upload", action="store_true",
                   help="Use an already-uploaded file on the printer")
    args = p.parse_args()

    if not args.skip_upload and not args.local_3mf.is_file():
        sys.exit(f"local file not found: {args.local_3mf}")
    if not args.bambu_connect_path.is_dir():
        sys.exit(
            f"bambu-connect path not found: {args.bambu_connect_path}\n"
            "Pass --bambu-connect-path or clone mattcar15/bambu-connect "
            "to that location."
        )

    remote_name = args.remote_name or args.local_3mf.name
    if not args.skip_upload:
        upload_via_ftps(args.host, args.access_code, args.local_3mf,
                        remote_name)
    else:
        print(f"[ftps] skipped — assuming {remote_name} already on printer")

    # Import bambu-connect from the user's checkout. Its ExecuteClient
    # carries the canonical start_print shape we want to test.
    sys.path.insert(0, str(args.bambu_connect_path))
    try:
        from bambu_connect import BambuClient  # type: ignore
    except ImportError as e:
        sys.exit(
            f"failed to import bambu-connect from {args.bambu_connect_path}: {e}\n"
            "Make sure `pip install paho-mqtt==1.6.1` ran first."
        )

    print(f"[mqtt] connecting to {args.host} as bblp+<access_code>…")
    client = BambuClient(args.host, args.access_code, args.serial)

    # Light status hook so we see what the printer reports back.
    # bambu-connect's WatchClient fires `message_callback` on every
    # status delta; we just stringify and print.
    def on_status(s):
        # PrinterStatus is a dataclass with mc_print_stage etc.
        # Print just the fields likely to mention command acceptance.
        bits = []
        for attr in (
            "subtask_name", "gcode_file", "mc_print_stage",
            "mc_remaining_time", "print_error", "command",
        ):
            v = getattr(s, attr, None)
            if v not in (None, ""):
                bits.append(f"{attr}={v!r}")
        if bits:
            print(f"[status] {' · '.join(bits)}")

    client.start_watch_client(message_callback=on_status)

    # Give the watch client a moment to subscribe before we issue
    # the print command — otherwise we'd miss the immediate ack /
    # error in the printer's reply.
    time.sleep(2.0)

    print(f"[mqtt] sending project_file command for {remote_name}…")
    client.executeClient.start_print(remote_name)

    # Watch for ~30 s so we see whether the printer accepts (status
    # transitions to PREPARE/PRINTING) or rejects (LCD shows error,
    # status stays IDLE).
    print("[mqtt] waiting 30s for printer response (Ctrl-C to stop)…")
    try:
        time.sleep(30.0)
    except KeyboardInterrupt:
        pass

    client.stop_watch_client()
    client.executeClient.disconnect()
    print("[done] disconnected")


if __name__ == "__main__":
    main()
