"""Fake-desktop checks for the Copy diagnostics helper, run by `--selftest`.

No real window or clipboard is used: the button, the toasts and the clipboard
are fakes, so the checks also run on a machine without a desktop.
"""

import itertools
import json

import journeys_core
import uia

REPORT = {
    "scan": {"installation": "C:\\fixture\\game"},
    "diagnostics": [],
    "created_directories": [],
}


class FakeSession:
    """Shows one scripted toast per click of the scripted button."""

    def __init__(self, toasts):
        self.toasts = list(toasts)
        self.clicks = 0
        self.line = ""

    def find(self, **kwargs):
        return object()

    def activate(self, control):
        self.line = self.toasts[min(self.clicks, len(self.toasts) - 1)]
        self.clicks += 1

    def text(self):
        return self.line


class FakeClipboard:
    """Returns scripted reads and records every restore write."""

    def __init__(self, reads):
        self.reads = list(reads)
        self.writes = []

    def read(self):
        if len(self.reads) > 1:
            return self.reads.pop(0)
        return self.reads[0]

    def write(self, text):
        self.writes.append(text)


def checks():
    """Returns one boolean per decision the Copy diagnostics helper makes."""

    def fake_clock():
        return itertools.count(0.0, 0.5).__next__

    def quiet(seconds):
        return None

    def run(session, clipboard, write=None):
        return journeys_core.copy_diagnostics(
            session,
            clipboard.read,
            write or clipboard.write,
            clock=fake_clock(),
            sleep=quiet,
        )

    results = {}
    session = FakeSession(["Diagnostics copied."])
    clipboard = FakeClipboard(["old clipboard", json.dumps(REPORT)])
    report = run(session, clipboard)
    results["diagnostics_clicks_once"] = session.clicks == 1
    results["diagnostics_reads_the_report"] = report["scan"]["installation"] == "C:\\fixture\\game"
    results["diagnostics_restores_the_previous_clipboard"] = clipboard.writes == ["old clipboard"]

    session = FakeSession(["Something went wrong. Try again."])
    clipboard = FakeClipboard(["old clipboard", "unchanged"])
    try:
        run(session, clipboard)
    except uia.Failure as error:
        results["diagnostics_fails_on_an_error_toast"] = (
            session.clicks == 1 and "Something went wrong" in str(error)
        )
    else:
        results["diagnostics_fails_on_an_error_toast"] = False
    results["diagnostics_restores_after_an_error"] = clipboard.writes == ["old clipboard"]

    session = FakeSession(["Diagnostics copied."])
    clipboard = FakeClipboard(["old clipboard", "not a report"])
    try:
        run(session, clipboard)
    except uia.Failure as error:
        results["diagnostics_requires_the_report_on_the_clipboard"] = (
            "doctor report" in str(error) and clipboard.writes == ["old clipboard"]
        )
    else:
        results["diagnostics_requires_the_report_on_the_clipboard"] = False

    def refuse(text):
        raise OSError("clipboard is busy")

    session = FakeSession(["Diagnostics copied."])
    clipboard = FakeClipboard(["old clipboard", json.dumps(REPORT)])
    results["diagnostics_ignores_a_failed_restore"] = (
        run(session, clipboard, write=refuse)["scan"]["installation"] == "C:\\fixture\\game"
    )

    results["diagnostics_report_rejects_other_json"] = (
        journeys_core.diagnostics_report('{"scan": {}}') is None
        and journeys_core.diagnostics_report("[]") is None
        and journeys_core.diagnostics_report("not json") is None
    )
    results["same_path_ignores_the_verbatim_prefix"] = journeys_core.same_path(
        "\\\\?\\C:\\fixture\\game", "C:\\fixture\\game"
    )
    return results
