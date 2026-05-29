"""Shared atomic file-write helper for the profile importers.

The cargo dev/test loop reads the generated profile files concurrently
with re-imports; a non-atomic truncate-then-write leaves a partial-
content window where `ProfileLibrary::load` parses garbage TOML and
panics. Writing to a temp file in the same directory and `os.replace`-ing
it into place makes each write atomic on POSIX (and a same-volume rename
on Windows), so a concurrent reader sees either the old or the new file,
never a torn one.

All three importers (`import_machine_profile.py`, `import_processes.py`,
`import_filaments.py`) call this one helper rather than each carrying a
private copy. Imported via `from _atomic_io import atomic_write_text` —
which works because Python puts the run script's directory on `sys.path`.
"""

from __future__ import annotations

import os
import tempfile
from pathlib import Path


def atomic_write_text(path: Path, content: str, encoding: str = "utf-8") -> None:
    """Write `content` to `path` atomically (temp file + os.replace)."""
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = tempfile.NamedTemporaryFile(
        mode="w",
        encoding=encoding,
        dir=path.parent,
        prefix=f".{path.name}.",
        suffix=".tmp",
        delete=False,
    )
    try:
        tmp.write(content)
        tmp.flush()
        os.fsync(tmp.fileno())
        tmp.close()
        os.replace(tmp.name, path)
    except BaseException:
        try:
            os.unlink(tmp.name)
        except FileNotFoundError:
            pass
        raise
