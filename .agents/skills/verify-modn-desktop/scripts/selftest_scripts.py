"""Selftest checks for the scripts themselves, their docs and the window seam.

`no_physical_input_remains` and `verifier_never_touches_system_clipboard` scan
the verifier's own code, so this file is the one place allowed to name the
banned APIs. The other checks use fakes and files only; nothing here touches
the desktop.
"""

import ast
import re
from pathlib import Path

import fixtures
import journeys_core
import uia
import windows

SCRIPTS = Path(__file__).resolve().parent
REPO = SCRIPTS.parents[3]
BANNED_APIS = {
    "Click",
    "RightClick",
    "DoubleClick",
    "MoveTo",
    "SendKeys",
    "SendInput",
    "mouse_event",
    "SetCursorPos",
    "SetForegroundWindow",
    "SetFocus",
    "focus",
    "bring_to_front",
}
CLIPBOARD_APIS = (
    "OpenClipboard",
    "GetClipboardData",
    "SetClipboardData",
    "EmptyClipboard",
    "GetClipboardSequenceNumber",
    "CF_UNICODETEXT",
    "win32clipboard",
    "pyperclip",
    "Get-Clipboard",
    "Set-Clipboard",
    "clip.exe",
    "tkinter",
)


def _called_name(node):
    function = node.func
    if isinstance(function, ast.Attribute):
        return function.attr
    if isinstance(function, ast.Name):
        return function.id
    return None


def _physical_calls(path):
    tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
    return sorted(
        {
            f"{path.name}:{node.lineno}:{name}"
            for node in ast.walk(tree)
            if isinstance(node, ast.Call)
            for name in [_called_name(node)]
            if name in BANNED_APIS
        }
    )


class NoPatternControl:
    """A control that publishes no usable UI Automation action."""

    Name = "Fake details"
    IsEnabled = True

    def _missing(self):
        raise RuntimeError("no pattern")

    GetInvokePattern = _missing
    GetLegacyIAccessiblePattern = _missing
    GetSelectionItemPattern = _missing
    GetExpandCollapsePattern = _missing

    def GetClickablePoint(self):
        return type("Point", (), {"x": 1, "y": 1})()


class FakeUser32:
    """A user32 seam whose EnumWindows result and errors are scripted."""

    def __init__(self, result=1, last_error=0, windows=()):
        self.result = result
        self.last_error = last_error
        self.windows = list(windows)

    def EnumWindows(self, callback, _):
        for hwnd, _, _ in self.windows:
            callback(hwnd, 0)
        return self.result

    def GetWindowThreadProcessId(self, hwnd, owner):
        for handle, pid, _ in self.windows:
            if handle == hwnd:
                owner._obj.value = pid
                return

    def IsWindowVisible(self, hwnd):
        return True

    def GetWindowTextLengthW(self, hwnd):
        return len(self._title(hwnd))

    def GetWindowTextW(self, hwnd, buffer, length):
        buffer.value = self._title(hwnd)

    def _title(self, hwnd):
        for handle, _, title in self.windows:
            if handle == hwnd:
                return title
        return ""


def _doc_play_sentences(text):
    flat = re.sub(r"\s+", " ", text)
    return [
        sentence
        for sentence in re.split(r"(?<=[.!?]) ", flat)
        if "play" in sentence.lower() and "fixture" in sentence.lower()
    ]


def _docs_name_the_play_source():
    exe = re.compile(r"[\w.-]+\.exe")
    for name in ("SKILL.md", "features/ownership.md"):
        text = (SCRIPTS.parent / name).read_text(encoding="utf-8")
        sentences = _doc_play_sentences(text)
        checked = 0
        for sentence in sentences:
            names = exe.findall(sentence)
            if not names:
                continue
            checked += 1
            if any(found != fixtures.PLAY_SOURCE for found in names):
                return False
        if checked == 0:
            return False
    return True


def checks():
    """Returns one boolean per selftest decision."""
    results = {}

    offenders = [
        offender
        for path in sorted(SCRIPTS.glob("*.py"))
        if path.name != Path(__file__).name
        for offender in _physical_calls(path)
    ]
    results["no_physical_input_remains"] = not offenders

    clipboard_offenders = [
        f"{path.name}:{token}"
        for path in sorted(SCRIPTS.glob("*.py"))
        if path.name != Path(__file__).name
        for token in CLIPBOARD_APIS
        if token in path.read_text(encoding="utf-8")
    ]
    results["verifier_never_touches_system_clipboard"] = not clipboard_offenders

    patched = {}
    clicks = []
    try:
        for name in ("Click", "RightClick", "DoubleClick", "MoveTo", "SendKeys"):
            patched[name] = getattr(uia.auto, name, None)
            setattr(uia.auto, name, lambda *args, **kwargs: clicks.append(args))
        try:
            uia.Session.__new__(uia.Session).activate(NoPatternControl())
        except uia.Failure as error:
            results["activate_fails_without_a_pattern"] = (
                "Fake details" in str(error)
                and "Invoke" in str(error)
                and clicks == []
            )
        else:
            results["activate_fails_without_a_pattern"] = False
    finally:
        for name, original in patched.items():
            if original is not None:
                setattr(uia.auto, name, original)

    failing = FakeUser32(result=0, last_error=5)
    try:
        windows.visible_windows_of([1], user32=failing)
    except OSError as error:
        results["enum_windows_zero_raises"] = (
            "EnumWindows" in str(error) and "5" in str(error)
        )
    else:
        results["enum_windows_zero_raises"] = False

    def failing_enum(pids):
        return windows.visible_windows_of(pids, user32=failing)

    try:
        windows.watch_windows([1], 0.2, enum=failing_enum, interval=0.01)
    except OSError:
        results["watch_windows_propagates_enum_failure"] = True
    else:
        results["watch_windows_propagates_enum_failure"] = False

    visible = FakeUser32(result=1, windows=[(100, 7, "Fixture window")])
    results["enum_windows_returns_owned_titles"] = (
        windows.visible_windows_of([7], user32=visible) == ["Fixture window"]
    )

    messages = (REPO / "apps/modn-desktop/ui/js/messages.js").read_text(encoding="utf-8")
    match = re.search(r"CLIPBOARD_FAILED:\s*'([^']*)'", messages)
    results["clipboard_failure_constant_matches_messages_js"] = bool(
        match and journeys_core.DIAGNOSTICS_CLIPBOARD_FAILED in match.group(1)
    )

    results["play_docs_name_the_play_source"] = _docs_name_the_play_source()

    claims = " ".join(
        (SCRIPTS.parent / name).read_text(encoding="utf-8")
        for name in ("SKILL.md", "features/ownership.md")
    )
    results["background_claim_is_precise"] = (
        "never move the pointer" in claims
        and "without taking the foreground" not in claims
        and "keep using the desktop" not in claims
    )
    return results
