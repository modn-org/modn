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
OTHER_REPORT = {
    "scan": {"installation": "C:\\fixture\\other"},
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
    """Serves scripted snapshots and restores under one sequence number.

    Each read returns the next scripted `(text, bump)` and raises the sequence
    number by `bump`, as an app write does. `restore` writes only while the
    number still equals its argument, like the real lock check; a `human_text`
    set before the call simulates a copy that lands first.
    """

    def __init__(self, reads, fail_on_read=None):
        self.reads = list(reads)
        self.writes = []
        self.restores = []
        self.reads_made = 0
        self.sequence = 0
        self.text = ""
        self.fail_on_read = fail_on_read
        self.human_text = None

    def read(self):
        self.reads_made += 1
        if self.reads_made == self.fail_on_read:
            raise OSError("clipboard is busy")
        text, bump = self.reads[min(self.reads_made - 1, len(self.reads) - 1)]
        self.sequence += bump
        self.text = text
        return text, self.sequence

    def restore(self, text, sequence):
        self.restores.append((text, sequence))
        if self.human_text is not None:
            self.sequence += 1
            self.text = self.human_text
            self.human_text = None
        if self.sequence != sequence:
            return False
        self.text = text
        self.writes.append(text)
        self.sequence += 1
        return True


def checks():
    """Returns one boolean per decision the Copy diagnostics helper makes."""

    def fake_clock():
        return itertools.count(0.0, 0.5).__next__

    def quiet(seconds):
        return None

    def run(session, clipboard, restore=None, clock=None):
        return journeys_core.copy_diagnostics(
            session,
            clipboard.read,
            restore or clipboard.restore,
            clock=clock or fake_clock(),
            sleep=quiet,
        )

    def installation(report):
        return report["scan"]["installation"]

    # (a) Nobody copied: one restore with the earlier text and the sequence of
    # the read that proved the app write.
    session = FakeSession(["Diagnostics copied."])
    clipboard = FakeClipboard([("old clipboard", 0), (json.dumps(REPORT), 1)])
    toast, report = run(session, clipboard)
    results = {}
    results["diagnostics_clicks_once"] = session.clicks == 1
    results["diagnostics_reports_the_success_toast"] = toast == "Diagnostics copied."
    results["diagnostics_reads_the_report"] = installation(report) == "C:\\fixture\\game"
    results["diagnostics_restores_the_previous_clipboard"] = (
        clipboard.restores == [("old clipboard", 1)]
        and clipboard.writes == ["old clipboard"]
        and clipboard.text == "old clipboard"
    )

    # (b) The human copied text after the app write, before the restore.
    session = FakeSession(["Diagnostics copied."])
    clipboard = FakeClipboard([("old clipboard", 0), (json.dumps(REPORT), 1)])
    clipboard.human_text = "human text"
    run(session, clipboard)
    results["diagnostics_keeps_text_the_human_copied_after_the_write"] = (
        clipboard.writes == [] and clipboard.text == "human text"
    )

    # (b) The human copied another valid doctor report: it still stays.
    human_report = json.dumps(OTHER_REPORT)
    session = FakeSession(["Diagnostics copied."])
    clipboard = FakeClipboard([("old clipboard", 0), (json.dumps(REPORT), 1)])
    clipboard.human_text = human_report
    _, report = run(session, clipboard)
    results["diagnostics_keeps_another_report_the_human_copied"] = (
        installation(report) == "C:\\fixture\\game"
        and clipboard.writes == []
        and clipboard.text == human_report
    )

    # (c) An error toast with an unchanged clipboard: no restore at all.
    session = FakeSession(["Something went wrong. Try again."])
    clipboard = FakeClipboard([("old clipboard", 0)])
    try:
        run(session, clipboard)
    except uia.Failure as error:
        results["diagnostics_fails_on_an_error_toast"] = (
            session.clicks == 1 and "Something went wrong" in str(error)
        )
    else:
        results["diagnostics_fails_on_an_error_toast"] = False
    results["diagnostics_does_not_restore_after_an_error"] = clipboard.restores == []

    # (c) The clipboard-failure toast fails at once and restores nothing.
    ticks = []

    def ticking_clock():
        ticks.append(1)
        return len(ticks) * 0.05

    session = FakeSession([journeys_core.DIAGNOSTICS_CLIPBOARD_FAILED])
    clipboard = FakeClipboard([("old clipboard", 0)])
    try:
        run(session, clipboard, clock=ticking_clock)
    except uia.Failure as error:
        results["diagnostics_fails_on_a_clipboard_failure"] = (
            journeys_core.DIAGNOSTICS_CLIPBOARD_FAILED in str(error) and len(ticks) <= 2
        )
    else:
        results["diagnostics_fails_on_a_clipboard_failure"] = False
    results["diagnostics_clipboard_failure_restores_nothing"] = clipboard.restores == []

    # (c) A read without a report proves no app write: no restore.
    session = FakeSession(["Diagnostics copied."])
    clipboard = FakeClipboard([("old clipboard", 0), ("not a report", 1)])
    try:
        run(session, clipboard)
    except uia.Failure as error:
        results["diagnostics_requires_the_report_on_the_clipboard"] = (
            "doctor report" in str(error) and clipboard.restores == []
        )
    else:
        results["diagnostics_requires_the_report_on_the_clipboard"] = False

    # (c) The report read itself fails: the error stays and no restore runs.
    session = FakeSession(["Diagnostics copied."])
    clipboard = FakeClipboard(
        [("old clipboard", 0), (json.dumps(REPORT), 1)], fail_on_read=2
    )
    try:
        run(session, clipboard)
    except OSError:
        results["diagnostics_does_not_restore_after_a_failed_report_read"] = (
            clipboard.restores == []
        )
    else:
        results["diagnostics_does_not_restore_after_a_failed_report_read"] = False

    # (d) A restore that raises is best effort: toast and report still return.
    def refuse(text, sequence):
        raise OSError("clipboard is busy")

    session = FakeSession(["Diagnostics copied."])
    clipboard = FakeClipboard([("old clipboard", 0), (json.dumps(REPORT), 1)])
    toast, report = run(session, clipboard, restore=refuse)
    results["diagnostics_ignores_a_failed_restore"] = (
        toast == "Diagnostics copied."
        and installation(report) == "C:\\fixture\\game"
        and clipboard.writes == []
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
