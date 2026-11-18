"""Reads and restores the Windows clipboard text through PowerShell.

The reads are read-only. The only write is a best-effort restore of text the
journey found before it clicked `Copy diagnostics`. No function here touches a
window, the foreground or the focus.
"""

import os
import subprocess
import tempfile
from pathlib import Path

_CREATE_NO_WINDOW = getattr(subprocess, "CREATE_NO_WINDOW", 0)
_READ = (
    "[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false); "
    "$text = Get-Clipboard -Raw; "
    "if ($null -ne $text) { [Console]::Out.Write($text) }"
)
_RESTORE = (
    "$text = Get-Content -LiteralPath $env:MODN_VERIFY_CLIPBOARD -Raw -Encoding UTF8; "
    "Set-Clipboard -Value $text"
)


def _powershell(command, env=None):
    return subprocess.run(
        ["powershell", "-NoProfile", "-NonInteractive", "-Command", command],
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        env=env,
        timeout=30,
        creationflags=_CREATE_NO_WINDOW,
        check=True,
    )


def read_text():
    """Returns the clipboard text, or an empty string when it holds no text."""
    return _powershell(_READ).stdout


def write_text(text):
    """Puts `text` back on the clipboard; the caller treats a failure as best-effort."""
    handle = tempfile.NamedTemporaryFile(
        "w", encoding="utf-8", suffix=".txt", delete=False
    )
    try:
        handle.write(text)
        handle.close()
        env = dict(os.environ, MODN_VERIFY_CLIPBOARD=str(Path(handle.name)))
        _powershell(_RESTORE, env=env)
    finally:
        Path(handle.name).unlink(missing_ok=True)
