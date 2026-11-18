"""Visible top-level windows, read through user32.

No function here types, moves the pointer, clicks or changes the foreground.
"""

import ctypes
import time
from ctypes import wintypes

_USER32 = ctypes.windll.user32 if hasattr(ctypes, "windll") else None
_ENUM = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM) if _USER32 else None


def _last_error(user32):
    """The error saved by a failed `EnumWindows`, from the fake or user32."""
    code = getattr(user32, "last_error", None)
    if code is not None:
        return code
    return ctypes.GetLastError()


def visible_windows_of(pids, user32=None):
    """Returns the titles of the visible top-level windows that `pids` own.

    A zero `EnumWindows` result raises `OSError`, so a failed enumeration is
    never reported as an empty desktop.
    """
    user32 = user32 or _USER32
    wanted = set(pids)
    found = []

    def callback(hwnd, _):
        owner = wintypes.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
        if owner.value in wanted and user32.IsWindowVisible(hwnd):
            length = user32.GetWindowTextLengthW(hwnd)
            title = ctypes.create_unicode_buffer(length + 1)
            user32.GetWindowTextW(hwnd, title, length + 1)
            found.append(title.value or "<untitled>")
        return True

    completed = user32.EnumWindows(_ENUM(callback), 0)
    if not completed:
        code = _last_error(user32)
        error = ctypes.WinError(code)
        raise OSError(f"EnumWindows failed with Win32 error {code}: {error}") from error
    return found


def watch_windows(pids, seconds, enum=visible_windows_of, interval=0.05):
    """Samples `pids` for `seconds` and returns every visible window title seen."""
    seen = []
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        for title in enum(pids):
            if title not in seen:
                seen.append(title)
        time.sleep(interval)
    return seen
