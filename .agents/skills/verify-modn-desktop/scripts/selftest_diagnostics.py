"""Fake-desktop checks for the Copy diagnostics helper, run by `--selftest`.

No real window, clipboard or sleep is used: the button, the toasts and the
clock are fakes, and the report goes to a temporary file.
"""

import itertools
import json
import tempfile
from pathlib import Path

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


def checks():
    """Returns one boolean per decision the Copy diagnostics helper makes."""

    def fake_clock():
        return itertools.count(0.0, 0.5).__next__

    def quiet(seconds):
        return None

    def run(session, report_file, clock=None, sleep=quiet):
        return journeys_core.copy_diagnostics(
            session,
            report_file,
            clock=clock or fake_clock(),
            sleep=sleep,
        )

    def installation(report):
        return report["scan"]["installation"]

    results = {}
    with tempfile.TemporaryDirectory() as folder:
        root = Path(folder)

        report_file = root / "report.txt"
        report_file.write_text(json.dumps(REPORT), encoding="utf-8")
        session = FakeSession(["Diagnostics copied."])
        toast, report = run(session, report_file)
        results["diagnostics_clicks_once"] = session.clicks == 1
        results["diagnostics_reports_the_success_toast"] = toast == "Diagnostics copied."
        results["diagnostics_reads_the_report_file"] = (
            installation(report) == "C:\\fixture\\game"
        )

        late_file = root / "late.txt"
        session = FakeSession(["Diagnostics copied."])
        toast, report = run(
            session,
            late_file,
            sleep=lambda _: late_file.write_text(json.dumps(REPORT), encoding="utf-8"),
        )
        results["diagnostics_accepts_a_late_report_file"] = (
            toast == "Diagnostics copied."
            and installation(report) == "C:\\fixture\\game"
        )

        sleeps = []
        session = FakeSession(["Diagnostics copied."])
        try:
            run(session, root / "absent.txt", sleep=lambda _: sleeps.append(1))
        except uia.Failure as error:
            results["diagnostics_fails_without_a_report_file"] = (
                "doctor report" in str(error) and len(sleeps) <= 31
            )
        else:
            results["diagnostics_fails_without_a_report_file"] = False

        other_file = root / "other.txt"
        other_file.write_text("not a doctor report", encoding="utf-8")
        session = FakeSession(["Diagnostics copied."])
        try:
            run(session, other_file)
        except uia.Failure as error:
            results["diagnostics_rejects_other_file_text"] = (
                "doctor report" in str(error) and session.clicks == 1
            )
        else:
            results["diagnostics_rejects_other_file_text"] = False

        session = FakeSession(["Something went wrong. Try again."])
        try:
            run(session, root / "error.txt")
        except uia.Failure as error:
            results["diagnostics_fails_on_an_error_toast"] = (
                "Something went wrong" in str(error) and session.clicks == 1
            )
        else:
            results["diagnostics_fails_on_an_error_toast"] = False

        session = FakeSession([journeys_core.DIAGNOSTICS_CLIPBOARD_FAILED])
        try:
            run(session, root / "clipboard-failure.txt")
        except uia.Failure as error:
            results["diagnostics_fails_on_a_clipboard_failure"] = (
                journeys_core.DIAGNOSTICS_CLIPBOARD_FAILED in str(error)
                and session.clicks == 1
            )
        else:
            results["diagnostics_fails_on_a_clipboard_failure"] = False

    results["diagnostics_report_rejects_other_json"] = (
        journeys_core.diagnostics_report('{"scan": {}}') is None
        and journeys_core.diagnostics_report("[]") is None
        and journeys_core.diagnostics_report("not json") is None
    )
    results["same_path_ignores_the_verbatim_prefix"] = journeys_core.same_path(
        "\\\\?\\C:\\fixture\\game", "C:\\fixture\\game"
    )
    return results
