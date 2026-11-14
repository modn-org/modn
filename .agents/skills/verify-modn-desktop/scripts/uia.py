"""UI Automation access to one Modn window, identified by process id.

Only top-level windows owned by the launched process are ever touched, and
only descendants of that window are searched. Nothing here moves the mouse:
buttons are activated through their UIA patterns.
"""

import ctypes
import json
import time

import uiautomation as auto

WM_CLOSE = 0x0010

_user32 = ctypes.windll.user32
_WNDENUMPROC = ctypes.WINFUNCTYPE(ctypes.c_bool, ctypes.c_void_p, ctypes.c_void_p)


def _visible_windows(pid):
    """Top-level window handles owned by `pid`, without a UIA root walk."""
    handles = []

    def collect(hwnd, _):
        owner = ctypes.c_ulong()
        _user32.GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
        if owner.value == pid and _user32.IsWindowVisible(hwnd):
            handles.append(hwnd)
        return True

    _user32.EnumWindows(_WNDENUMPROC(collect), 0)
    return handles


def _title_of(hwnd):
    buffer = ctypes.create_unicode_buffer(512)
    _user32.GetWindowTextW(hwnd, buffer, 512)
    return buffer.value


class Failure(AssertionError):
    """An assertion about the user interface failed."""


def require(condition, message):
    if not condition:
        raise Failure(message)


class Session:
    """Drives the top-level window of exactly one launched process."""

    def __init__(self, pid, evidence=None):
        self.pid = pid
        self.evidence = evidence
        self.window = None
        self.snapshots = 0
        auto.SetGlobalSearchTimeout(2)

    # -- window ---------------------------------------------------------
    def attach(self, timeout=30):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            for hwnd in _visible_windows(self.pid):
                try:
                    if _title_of(hwnd) == "Modn":
                        self.window = auto.ControlFromHandle(hwnd)
                        return self
                except Exception:
                    continue
            time.sleep(0.2)
        raise Failure(f"No Modn window for pid {self.pid}")

    def owned_windows(self):
        windows = []
        for hwnd in _visible_windows(self.pid):
            try:
                windows.append(auto.ControlFromHandle(hwnd))
            except Exception:
                continue
        return windows

    def extra_windows(self):
        """Visible top-level windows of this process other than the Modn window."""
        main = self.window.NativeWindowHandle
        windows = []
        for hwnd in _visible_windows(self.pid):
            if hwnd == main:
                continue
            try:
                windows.append(auto.ControlFromHandle(hwnd))
            except Exception:
                continue
        return windows

    def focus(self):
        """Brings the owned window to the foreground, so the clipboard works."""
        ctypes.windll.user32.SetForegroundWindow(self.window.NativeWindowHandle)
        time.sleep(0.3)

    def request_close(self):
        """Posts WM_CLOSE to the owned window only, as a user would."""
        require(self.window is not None, "No window to close")
        ctypes.windll.user32.PostMessageW(self.window.NativeWindowHandle, WM_CLOSE, 0, 0)

    # -- tree -----------------------------------------------------------
    def walk(self, control=None):
        yield from self._walk(control or self.window)

    def _walk(self, control):
        yield control
        try:
            children = control.GetChildren()
        except Exception:
            return
        for child in children:
            yield from self._walk(child)

    def text(self):
        return self.text_of(self.window)

    def tree_text(self):
        """Every visible string in the main window and every owned dialog."""
        parts = [self.text()]
        for window in self.extra_windows():
            try:
                parts.append(self.text_of(window))
            except Exception:
                continue
        return "\n".join(part for part in parts if part)

    def text_of(self, root):
        parts = []
        for control in self._walk(root):
            try:
                for value in (control.Name, self.value_of(control)):
                    if value:
                        parts.append(value)
            except Exception:
                continue
        return "\n".join(parts)

    @staticmethod
    def value_of(control):
        try:
            if control.IsValuePatternAvailable():
                return control.GetValuePattern().Value
        except Exception:
            return ""
        return ""

    def describe(self, control):
        return {
            "type": control.ControlTypeName,
            "name": control.Name,
            "automation_id": control.AutomationId,
            "enabled": control.IsEnabled,
            "offscreen": control.IsOffscreen,
        }

    def snapshot(self, label):
        """Records the full tree as evidence and returns it as text."""
        self.snapshots += 1
        text = self.tree_text()
        if self.evidence is not None:
            path = self.evidence / f"uia-{self.snapshots:03d}-{label}.txt"
            path.write_text(text, encoding="utf-8")
        return text

    # -- queries --------------------------------------------------------
    def find_all(self, name=None, prefix=None, control_type=None, window=None):
        matches = []
        for control in self._walk(window) if window is not None else self.walk():
            try:
                if control_type and control.ControlTypeName != control_type:
                    continue
                if name is not None and control.Name != name:
                    continue
                if prefix is not None and not control.Name.startswith(prefix):
                    continue
            except Exception:
                continue
            matches.append(control)
        return matches

    def find(self, name=None, prefix=None, control_type=None, window=None):
        matches = self.find_all(name=name, prefix=prefix, control_type=control_type, window=window)
        require(len(matches) == 1, f"Expected one control for {name or prefix}, found {len(matches)}")
        return matches[0]

    def exists(self, name=None, prefix=None, control_type=None):
        return bool(self.find_all(name=name, prefix=prefix, control_type=control_type))

    # -- actions --------------------------------------------------------
    def activate(self, control):
        require(control.IsEnabled, f"Control disabled: {control.Name}")
        # Invoke is synchronous and can time out while the handler it started
        # waits on a modal dialog. A timeout therefore does not mean the click
        # failed, and retrying here would open two dialogs.
        try:
            pattern = control.GetInvokePattern()
        except Exception:
            pattern = None
        if pattern is not None:
            try:
                pattern.Invoke()
            except Exception:
                pass
            return
        try:
            control.GetLegacyIAccessiblePattern().DoDefaultAction()
            return
        except Exception:
            pass
        try:
            point = control.GetClickablePoint()
            auto.Click(point.x, point.y)
            return
        except Exception as error:
            raise Failure(f"Cannot activate {control.Name}: {error}") from error

    def click(self, name=None, prefix=None, control_type="ButtonControl", window=None):
        control = self.find(name=name, prefix=prefix, control_type=control_type, window=window)
        self.activate(control)
        return control

    def wait_text(self, text, timeout=30, message=None):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if text in self.text():
                return True
            time.sleep(0.2)
        raise Failure(message or f"Timed out waiting for {text!r}")

    def wait_for(self, predicate, description, timeout=30):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                if predicate(self):
                    return True
            except Exception:
                pass
            time.sleep(0.2)
        raise Failure(f"Timed out waiting for {description}")

    def set_text(self, control, value):
        control.GetValuePattern().SetValue(value)


def titled_windows(session):
    """Visible, titled top-level windows this process owns, except the main one.

    Untitled helper windows, such as the Tao event-target window, are ignored.
    """
    windows = []
    for window in session.extra_windows():
        try:
            if (window.Name or "").strip():
                windows.append(window)
        except Exception:
            continue
    return windows


def dialog_for(session, timeout=20):
    """Waits for the first titled native dialog and returns it.

    A second dialog does not make the wait fail: callers that must reject
    stacked dialogs count `titled_windows` themselves.
    """
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        windows = titled_windows(session)
        if windows:
            return windows[0]
        time.sleep(0.2)
    raise Failure("No native dialog opened")


def dialog_text(dialog):
    parts = []
    for control in _walk(dialog):
        try:
            if control.Name:
                parts.append(control.Name)
        except Exception:
            continue
    return "\n".join(parts)


def _walk(control):
    yield control
    try:
        children = control.GetChildren()
    except Exception:
        return
    for child in children:
        yield from _walk(child)


WM_SETTEXT = 0x000C
BM_CLICK = 0x00F5
IDCANCEL = 2


def _win32():
    return ctypes.windll.user32


def set_file_dialog(dialog, path, session):
    """Fills the common item dialog's file name field and confirms it.

    The common item dialog does not publish its control ids through UI
    Automation, so this uses the documented Win32 dialog control ids: 1148 is
    the file name combo box and 1 is the Open button. Both are resolved on the
    dialog window that this process owns.
    """
    window = dialog.NativeWindowHandle
    combo = _win32().GetDlgItem(window, 1148)
    require(combo, "File dialog has no file name field")
    _win32().SendMessageW(combo, WM_SETTEXT, 0, ctypes.c_wchar_p(str(path)))
    confirm = _win32().GetDlgItem(window, 1)
    require(confirm, "File dialog has no Open button")
    _win32().PostMessageW(confirm, BM_CLICK, 0, 0)


def _find_by_id(dialog, automation_id):
    matches = [c for c in _walk(dialog) if c.AutomationId == automation_id]
    if len(matches) != 1:
        return None
    return matches[0]


def dismiss(dialog, session, name="OK"):
    session.click(name=name, window=dialog)


def clipboard_text(timeout=10):
    """Reads the Unicode text clipboard without changing it."""
    user32 = ctypes.windll.user32
    kernel32 = ctypes.windll.kernel32
    user32.OpenClipboard.argtypes = [ctypes.c_void_p]
    user32.GetClipboardData.restype = ctypes.c_void_p
    kernel32.GlobalLock.argtypes = [ctypes.c_void_p]
    kernel32.GlobalLock.restype = ctypes.c_void_p
    kernel32.GlobalUnlock.argtypes = [ctypes.c_void_p]
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if user32.OpenClipboard(None):
            break
        time.sleep(0.1)
    else:
        raise Failure("Cannot open the clipboard")
    try:
        handle = user32.GetClipboardData(13)
        require(handle, "Clipboard holds no Unicode text")
        pointer = kernel32.GlobalLock(handle)
        require(pointer, "Cannot lock the clipboard text")
        try:
            return ctypes.wstring_at(pointer)
        finally:
            kernel32.GlobalUnlock(handle)
    finally:
        user32.CloseClipboard()


def to_json(control):
    return json.dumps({"name": control.Name, "type": control.ControlTypeName})
