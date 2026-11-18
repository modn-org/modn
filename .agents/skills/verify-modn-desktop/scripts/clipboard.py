"""Reads and restores the Windows clipboard text under one clipboard lock.

The snapshot returns the text together with the Windows clipboard sequence
number, so the number proves which write was read. The restore opens the
clipboard first, compares that number while it holds the lock, and writes only
when nobody copied since. Everything goes through `api`, an injectable object
of Win32 functions that loads lazily, so a selftest can use fakes on any
platform. No function here touches a window, the foreground or the focus.
"""

import ctypes
import time
from types import SimpleNamespace

CF_UNICODETEXT = 13
GMEM_MOVEABLE = 0x0002
RETRY_SECONDS = 0.05

_API = None


def _real_api():
    """Returns the real user32 and kernel32 functions with full signatures."""
    user32 = ctypes.WinDLL("user32", use_last_error=True)
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)

    user32.OpenClipboard.argtypes = [ctypes.c_void_p]
    user32.OpenClipboard.restype = ctypes.c_int
    user32.CloseClipboard.argtypes = []
    user32.CloseClipboard.restype = ctypes.c_int
    user32.GetClipboardSequenceNumber.argtypes = []
    user32.GetClipboardSequenceNumber.restype = ctypes.c_uint
    user32.GetClipboardData.argtypes = [ctypes.c_uint]
    user32.GetClipboardData.restype = ctypes.c_void_p
    user32.EmptyClipboard.argtypes = []
    user32.EmptyClipboard.restype = ctypes.c_int
    user32.SetClipboardData.argtypes = [ctypes.c_uint, ctypes.c_void_p]
    user32.SetClipboardData.restype = ctypes.c_void_p
    kernel32.GlobalAlloc.argtypes = [ctypes.c_uint, ctypes.c_size_t]
    kernel32.GlobalAlloc.restype = ctypes.c_void_p
    kernel32.GlobalLock.argtypes = [ctypes.c_void_p]
    kernel32.GlobalLock.restype = ctypes.c_void_p
    kernel32.GlobalUnlock.argtypes = [ctypes.c_void_p]
    kernel32.GlobalUnlock.restype = ctypes.c_int
    kernel32.GlobalFree.argtypes = [ctypes.c_void_p]
    kernel32.GlobalFree.restype = ctypes.c_void_p

    return SimpleNamespace(
        open_clipboard=user32.OpenClipboard,
        close_clipboard=user32.CloseClipboard,
        sequence_number=user32.GetClipboardSequenceNumber,
        get_clipboard_data=user32.GetClipboardData,
        empty_clipboard=user32.EmptyClipboard,
        set_clipboard_data=user32.SetClipboardData,
        global_alloc=kernel32.GlobalAlloc,
        global_lock=kernel32.GlobalLock,
        global_unlock=kernel32.GlobalUnlock,
        global_free=kernel32.GlobalFree,
        memmove=ctypes.memmove,
        wstring_at=ctypes.wstring_at,
    )


def _api():
    """Returns the cached real API, loading it on the first real clipboard use."""
    global _API
    if _API is None:
        _API = _real_api()
    return _API


def _open(api, clock, sleep, timeout):
    """Opens the clipboard, retrying every 0.05 seconds until `timeout`."""
    deadline = clock() + timeout
    while clock() < deadline:
        if api.open_clipboard(None):
            return
        sleep(RETRY_SECONDS)
    raise OSError("The clipboard stayed open by another process")


def read_snapshot(timeout=2.0, api=None, clock=time.monotonic, sleep=time.sleep):
    """Returns the clipboard text and its sequence number under one lock.

    A clipboard without Unicode text yields an empty string. The number comes
    from the same open clipboard as the text, so it names that exact write.
    """
    api = api or _api()
    _open(api, clock, sleep, timeout)
    try:
        sequence = api.sequence_number()
        handle = api.get_clipboard_data(CF_UNICODETEXT)
        if not handle:
            return "", sequence
        pointer = api.global_lock(handle)
        if not pointer:
            return "", sequence
        try:
            return api.wstring_at(pointer), sequence
        finally:
            api.global_unlock(handle)
    finally:
        api.close_clipboard()


def _text_block(api, text):
    """Returns a moveable global block holding `text` as UTF-16 with a NUL."""
    data = (text + "\0").encode("utf-16-le")
    handle = api.global_alloc(GMEM_MOVEABLE, len(data))
    if not handle:
        raise OSError("GlobalAlloc failed")
    pointer = api.global_lock(handle)
    if not pointer:
        api.global_free(handle)
        raise OSError("GlobalLock failed")
    try:
        api.memmove(pointer, data, len(data))
    finally:
        api.global_unlock(handle)
    return handle


def restore_if_unchanged(
    text, sequence, timeout=2.0, api=None, clock=time.monotonic, sleep=time.sleep
):
    """Puts `text` back only while the clipboard sequence number is unchanged.

    The sequence check and the write happen under one clipboard lock, so text
    copied by anyone since `sequence` was read stays untouched. An empty `text`
    empties the clipboard. Returns True when it wrote and False when it did not.
    """
    api = api or _api()
    _open(api, clock, sleep, timeout)
    try:
        if api.sequence_number() != sequence:
            return False
        api.empty_clipboard()
        if text:
            handle = _text_block(api, text)
            if not api.set_clipboard_data(CF_UNICODETEXT, handle):
                api.global_free(handle)
                raise OSError("SetClipboardData failed")
        return True
    finally:
        api.close_clipboard()
