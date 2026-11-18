"""Fake-Win32 checks for the clipboard snapshot and conditional restore.

No real clipboard is opened: every ctypes call is a fake that records its name,
so the checks prove the call order around one clipboard lock on any platform.
"""

import clipboard

LOCK_CALLS = (
    "OpenClipboard",
    "GetClipboardSequenceNumber",
    "GetClipboardData",
    "EmptyClipboard",
    "SetClipboardData",
    "CloseClipboard",
)


class FakeApi:
    """Records each Win32 call in order and answers from scripted values."""

    def __init__(self, opens=(1,), sequence=7, data=0, text="", set_ok=True):
        self.calls = []
        self.opens = list(opens)
        self.sequence = sequence
        self.data = data
        self.text = text
        self.set_ok = set_ok
        self.allocated = []
        self.moved = []
        self.freed = []

    def open_clipboard(self, owner):
        self.calls.append("OpenClipboard")
        if len(self.opens) > 1:
            return self.opens.pop(0)
        return self.opens[0]

    def close_clipboard(self):
        self.calls.append("CloseClipboard")
        return 1

    def sequence_number(self):
        self.calls.append("GetClipboardSequenceNumber")
        return self.sequence

    def get_clipboard_data(self, fmt):
        self.calls.append("GetClipboardData")
        return self.data

    def empty_clipboard(self):
        self.calls.append("EmptyClipboard")
        return 1

    def set_clipboard_data(self, fmt, handle):
        self.calls.append("SetClipboardData")
        return handle if self.set_ok else 0

    def global_alloc(self, flags, size):
        self.calls.append("GlobalAlloc")
        handle = 0x1000 + len(self.allocated)
        self.allocated.append((flags, size, handle))
        return handle

    def global_lock(self, handle):
        self.calls.append("GlobalLock")
        return 0x2000

    def global_unlock(self, handle):
        self.calls.append("GlobalUnlock")
        return 1

    def global_free(self, handle):
        self.calls.append("GlobalFree")
        self.freed.append(handle)
        return 0

    def memmove(self, pointer, data, size):
        self.calls.append("memmove")
        self.moved.append((pointer, data, size))

    def wstring_at(self, pointer):
        self.calls.append("wstring_at")
        return self.text


class FakeClock:
    """Advances only when the code sleeps, so a retry loop ends at its timeout."""

    def __init__(self):
        self.now = 0.0

    def __call__(self):
        return self.now

    def sleep(self, seconds):
        self.now += seconds


def locked(api):
    """Returns the clipboard-lock calls, in order."""
    return [call for call in api.calls if call in LOCK_CALLS]


def call(function, api, *args, timeout=2.0):
    """Calls `function` with fake time, so no retry ever really waits."""
    clock = FakeClock()
    return function(*args, api=api, clock=clock, sleep=clock.sleep, timeout=timeout)


def checks():
    """Returns one boolean per clipboard-lock promise."""
    results = {}

    # The text and the sequence come from the same open clipboard.
    api = FakeApi(data=0x1234, text="hello")
    text, sequence = call(clipboard.read_snapshot, api)
    results["clipboard_snapshot_reads_one_lock"] = (
        (text, sequence) == ("hello", 7)
        and api.calls.count("CloseClipboard") == 1
        and locked(api)
        == [
            "OpenClipboard",
            "GetClipboardSequenceNumber",
            "GetClipboardData",
            "CloseClipboard",
        ]
    )

    api = FakeApi(data=0)
    text, _ = call(clipboard.read_snapshot, api)
    results["clipboard_snapshot_without_unicode_text_is_empty"] = (
        text == "" and "GlobalLock" not in api.calls
    )

    # Unchanged sequence: empty and write while the clipboard is still open.
    api = FakeApi(sequence=7)
    data = "back\0".encode("utf-16-le")
    restored = call(clipboard.restore_if_unchanged, api, "back", 7)
    results["clipboard_restore_writes_under_one_lock"] = (
        restored
        and api.calls.count("CloseClipboard") == 1
        and locked(api)
        == [
            "OpenClipboard",
            "GetClipboardSequenceNumber",
            "EmptyClipboard",
            "SetClipboardData",
            "CloseClipboard",
        ]
        and api.allocated == [(clipboard.GMEM_MOVEABLE, len(data), 0x1000)]
        and api.moved == [(0x2000, data, len(data))]
    )

    # A changed sequence proves someone copied since: no empty and no write.
    api = FakeApi(sequence=8)
    restored = call(clipboard.restore_if_unchanged, api, "back", 7)
    results["clipboard_restore_skips_a_changed_sequence"] = (
        restored is False
        and api.calls
        == ["OpenClipboard", "GetClipboardSequenceNumber", "CloseClipboard"]
    )

    # A blocked clipboard gives no lock, so it must never be closed.
    api = FakeApi(opens=(0,))
    try:
        call(clipboard.read_snapshot, api, timeout=0.2)
    except OSError:
        snapshot_blocked = "CloseClipboard" not in api.calls
    else:
        snapshot_blocked = False
    api = FakeApi(opens=(0,))
    try:
        call(clipboard.restore_if_unchanged, api, "back", 7, timeout=0.2)
    except OSError:
        restore_blocked = "CloseClipboard" not in api.calls
    else:
        restore_blocked = False
    results["clipboard_open_timeout_raises_without_a_close"] = (
        snapshot_blocked and restore_blocked
    )

    # A failed set frees its block and still closes the one open clipboard.
    api = FakeApi(set_ok=False)
    try:
        call(clipboard.restore_if_unchanged, api, "back", 7)
    except OSError:
        freed = api.freed == [api.allocated[0][2]]
        closed = api.calls.count("CloseClipboard") == 1
    else:
        freed = closed = False
    results["clipboard_restore_frees_and_closes_after_a_failed_set"] = freed and closed

    # An empty restore only empties: no block and no set.
    api = FakeApi()
    restored = call(clipboard.restore_if_unchanged, api, "", 7)
    results["clipboard_restore_of_empty_text_only_empties"] = (
        restored
        and locked(api)
        == [
            "OpenClipboard",
            "GetClipboardSequenceNumber",
            "EmptyClipboard",
            "CloseClipboard",
        ]
        and "SetClipboardData" not in api.calls
        and "GlobalAlloc" not in api.calls
    )

    return results
