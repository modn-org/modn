"""Fake-desktop checks for the Copy diagnostics helper, run by `--selftest`.

No real window is used: the foreground, the focused button and the toasts are
fakes, so the checks also run on a machine without a desktop.
"""

import itertools
import tempfile
from pathlib import Path

import journeys_core
import uia
import windows


class FakeSession:
    """Shows the next scripted toast after each click of the focused button."""

    def __init__(self, toasts):
        self.toasts = list(toasts)
        self.clicks = 0
        self.line = ""
        self.window = type("Window", (), {"NativeWindowHandle": 7})()
        self.button = type("Control", (), {"SetFocus": lambda control: None})()

    def find(self, **kwargs):
        return self.button

    def activate(self, control):
        self.line = self.toasts[min(self.clicks, len(self.toasts) - 1)]
        self.clicks += 1

    def text(self):
        return self.line

    def wait_for(self, predicate, description, timeout=30):
        self.line = ""
        return True


class Foreground:
    """A desktop whose foreground is either the Modn window (7) or the editor."""

    def __init__(self, hold, sticky=False):
        self.current = 7 if hold else 99
        self.sticky = sticky
        self.takes = 0

    def front(self, hwnd):
        self.takes += 1
        if not self.sticky:
            self.current = hwnd
        return not self.sticky

    def holder(self):
        if self.current == 7:
            return {"hwnd": 7, "pid": 20912, "title": "Modn"}
        return {"hwnd": 99, "pid": 16544, "title": "T3 Code (Beta)"}


def checks():
    """Returns one boolean per decision the Copy diagnostics helper makes."""

    def fake_clock():
        return itertools.count(0.0, 0.5).__next__

    def quiet(seconds):
        return None

    results = {}
    with tempfile.TemporaryDirectory() as folder:
        fx = type("Fx", (), {"evidence": Path(folder)})()

        def run_copy(session, foreground, clock):
            return journeys_core.copy_diagnostics(
                session, fx, front=foreground.front, holder=foreground.holder,
                clock=clock, sleep=quiet,
            )

        steady = Foreground(hold=True)
        held, info = journeys_core.hold_foreground(
            7, steady.front, steady.holder, fake_clock(), quiet, 5.0
        )
        results["hold_accepts_a_steady_window"] = held and steady.takes == 0 and info["pid"] == 20912

        flapping = Foreground(hold=True)
        flapping.script = [7, 99] * 10
        flapping.holder = lambda: {
            "hwnd": flapping.script.pop(0),
            "pid": 16544,
            "title": "T3 Code (Beta)",
        }
        held, info = journeys_core.hold_foreground(
            7, flapping.front, flapping.holder, fake_clock(), quiet, 5.0
        )
        results["hold_needs_consecutive_samples"] = not held and flapping.takes >= 1

        retried = FakeSession(["Something went wrong. Try again.", "Diagnostics copied."])
        results["diagnostics_retries_after_error_toast"] = (
            run_copy(retried, Foreground(hold=True), fake_clock()) == "Diagnostics copied."
            and retried.clicks == 2
        )

        failing = FakeSession(["Something went wrong. Try again."] * 3)
        try:
            run_copy(failing, Foreground(hold=True), fake_clock())
        except uia.Failure as error:
            results["diagnostics_fails_after_three_error_toasts"] = (
                failing.clicks == journeys_core.DIAGNOSTICS_ATTEMPTS
                and "Keep the desktop idle" in str(error)
            )
        else:
            results["diagnostics_fails_after_three_error_toasts"] = False

        try:
            run_copy(FakeSession(["Diagnostics copied."]), Foreground(hold=False, sticky=True), fake_clock())
        except uia.Failure as error:
            results["diagnostics_names_the_holder"] = (
                "T3 Code (Beta)" in str(error) and "Keep the desktop idle" in str(error)
            )
        else:
            results["diagnostics_names_the_holder"] = False

        keeper = Foreground(hold=False)
        toast_session = FakeSession(["Diagnostics copied."])
        toast_session.line = "Diagnostics copied."
        kept = journeys_core.watch_toast(
            toast_session, 7, keeper.front, keeper.holder, fake_clock(), quiet
        )
        results["toast_wait_restores_the_foreground"] = kept == "Diagnostics copied." and keeper.takes >= 1

        run_copy(FakeSession(["Something went wrong.", "Diagnostics copied."]), Foreground(hold=True), fake_clock())
        toasts = (Path(folder) / "diagnostics-toast.txt").read_text(encoding="utf-8")
        samples = (Path(folder) / "diagnostics-foreground.txt").read_text(encoding="utf-8")
        results["diagnostics_records_every_toast"] = "attempt 1" in toasts and "attempt 2" in toasts
        results["diagnostics_records_foreground_samples"] = "Modn" in samples

    tried = []

    def flaky_front(hwnd):
        tried.append(hwnd)
        return len(tried) >= 3

    results["bring_to_front_retries"] = (
        windows.bring_to_front(7, timeout=5, front=flaky_front, clock=fake_clock(), sleep=quiet)
        and len(tried) == 3
    )
    results["bring_to_front_reports_a_timeout"] = not windows.bring_to_front(
        7, timeout=2, front=lambda hwnd: False, clock=fake_clock(), sleep=quiet
    )
    return results
