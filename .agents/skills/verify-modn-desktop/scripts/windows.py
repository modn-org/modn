"""Visible top-level windows of given process ids, read through EnumWindows."""

import ctypes
import time
from ctypes import wintypes

_USER32 = ctypes.windll.user32 if hasattr(ctypes, "windll") else None
_ENUM = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM) if _USER32 else None


def visible_windows_of(pids):
    """Returns the titles of the visible top-level windows that `pids` own."""
    wanted = set(pids)
    found = []

    def callback(hwnd, _):
        owner = wintypes.DWORD()
        _USER32.GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
        if owner.value in wanted and _USER32.IsWindowVisible(hwnd):
            length = _USER32.GetWindowTextLengthW(hwnd)
            title = ctypes.create_unicode_buffer(length + 1)
            _USER32.GetWindowTextW(hwnd, title, length + 1)
            found.append(title.value or "<untitled>")
        return True

    _USER32.EnumWindows(_ENUM(callback), 0)
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
