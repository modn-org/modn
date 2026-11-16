"""Visible top-level windows and the foreground, read through user32.

No function here types, moves the pointer or clicks. `take_foreground` only
asks Windows to activate a window for the focus-sensitive clipboard write.
"""

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


def foreground_info():
    """Describes the window that has the foreground: its handle, owner and title."""
    hwnd = _USER32.GetForegroundWindow()
    owner = wintypes.DWORD()
    _USER32.GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
    length = _USER32.GetWindowTextLengthW(hwnd)
    title = ctypes.create_unicode_buffer(length + 1)
    _USER32.GetWindowTextW(hwnd, title, length + 1)
    return {"hwnd": hwnd, "pid": owner.value, "title": title.value}


def take_foreground(hwnd):
    """Makes one attempt to give `hwnd` the foreground and reports whether it has it.

    The thread of the current foreground window lends its input state for the
    two calls, so `SetForegroundWindow` is allowed while another app holds the
    foreground lock. Nothing is typed, moved or clicked.
    """
    if _USER32.GetForegroundWindow() == hwnd:
        return True
    kernel32 = ctypes.windll.kernel32
    current = _USER32.GetForegroundWindow()
    lender = _USER32.GetWindowThreadProcessId(current, None) if current else 0
    mine = kernel32.GetCurrentThreadId()
    attached = bool(lender and lender != mine and _USER32.AttachThreadInput(mine, lender, True))
    try:
        if _USER32.IsIconic(hwnd):
            _USER32.ShowWindow(hwnd, 9)  # SW_RESTORE
        _USER32.SetForegroundWindow(hwnd)
    finally:
        if attached:
            _USER32.AttachThreadInput(mine, lender, False)
    return _USER32.GetForegroundWindow() == hwnd


def bring_to_front(hwnd, timeout=10.0, interval=0.2, front=None, clock=time.monotonic, sleep=time.sleep):
    """Repeats `take_foreground` until `hwnd` has the foreground or `timeout` passes."""
    front = take_foreground if front is None else front
    deadline = clock() + timeout
    while True:
        if front(hwnd):
            return True
        if clock() >= deadline:
            return False
        sleep(interval)


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
