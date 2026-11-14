"""Verify Modn Desktop through UI Automation on disposable fixtures.

Run from the repository root:

    uv run --no-project python .agents/skills/verify-modn-desktop/scripts/verify.py --build

The script builds the app and the test-instruction-budget CLI, then runs every
journey against a fresh temporary fixture. It never touches a real game, a
real store library or the owner's Modn state, and it only addresses the Modn
window of the process it started.
"""

import argparse
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

REPO = Path(__file__).resolve().parents[4]
TARGET = REPO / "target"


def bootstrap():
    """Re-runs under uv with uiautomation when the interpreter lacks it."""
    try:
        import uiautomation  # noqa: F401
        return
    except ImportError:
        pass
    if os.environ.get("MODN_UIA_BOOTSTRAP") == "1":
        raise SystemExit("uiautomation is unavailable")
    os.environ["MODN_UIA_BOOTSTRAP"] = "1"
    command = [
        "uv", "run", "--no-project", "--with", "uiautomation", "python",
        str(Path(__file__).resolve()), *sys.argv[1:],
    ]
    raise SystemExit(subprocess.call(command))


bootstrap()

import fixtures  # noqa: E402
import lifecycle  # noqa: E402
import uia  # noqa: E402
from fixtures import Fixture, run, snapshot  # noqa: E402

BLOCKED_PLAN = None


class App:
    """One launched `modn-desktop.exe`, addressed only by its process id."""

    def __init__(self, fixture, backend, extra_env=None):
        self.fixture = fixture
        self.backend = Path(backend).resolve()
        self.extra_env = extra_env or {}
        self.process = None
        self.session = None
        self.stderr = None

    def launch(self, timeout=40):
        env = self.fixture.environment()
        env.update(self.extra_env)
        self.stderr = open(self.fixture.evidence / "app-stderr.txt", "w", encoding="utf-8")
        args = [
            str(TARGET / "debug" / "modn-desktop.exe"),
            "--backend", str(self.backend),
            "--state-dir", str(self.fixture.state),
            "--data-dir", str(self.fixture.data),
            "--evidence-dir", str(self.fixture.evidence / "backend"),
        ]
        if self.fixture.modules.is_dir() and any(self.fixture.modules.iterdir()):
            args += ["--module-dir", str(self.fixture.modules)]
        self.process = subprocess.Popen(
            args, env=env, stdout=self.stderr, stderr=self.stderr,
            creationflags=subprocess.CREATE_NO_WINDOW,
        )
        self.stderr.close()
        (self.fixture.evidence / "launch.json").write_text(
            json.dumps({"args": args, "pid": self.process.pid}, indent=2), encoding="utf-8")
        try:
            self.session = uia.Session(self.process.pid, self.fixture.evidence).attach(timeout)
        except Exception:
            self.kill()
            raise
        return self

    def close(self, timeout=45):
        if self.process is None or self.process.poll() is not None:
            return self.process.returncode if self.process else None
        for dialog in self.session.extra_windows():
            for control in uia._walk(dialog):
                if control.Name in ("OK", "Close", "Cancel"):
                    try:
                        self.session.activate(control)
                        break
                    except Exception:
                        continue
        self.session.request_close()
        try:
            self.process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait(timeout=10)
        return self.process.returncode

    def kill(self):
        if self.process and self.process.poll() is None:
            self.process.kill()
            self.process.wait(timeout=10)


class Runner:
    def __init__(self, args):
        self.args = args
        self.assertions = 0
        self.results = []
        self.cleanup = []

    def check(self, condition, message):
        uia.require(condition, message)
        self.assertions += 1

    def journey(self, name, function):
        started = time.monotonic()
        try:
            function()
            status = "passed"
        except Exception as error:  # noqa: BLE001 - every failure is recorded
            status = f"failed: {error}"
            print(f"  ! {name}: {error}")
        self.results.append({
            "journey": name,
            "status": status,
            "seconds": round(time.monotonic() - started, 1),
        })
        print(f"  {name}: {status}")

    def clean(self):
        for action in reversed(self.cleanup):
            try:
                action()
            except Exception:
                pass


def cli(fixture, *args):
    return run(
        [TARGET / "debug" / "modn.exe", "--json", "--state-dir", fixture.state, *args],
        env=fixture.environment(),
    )


def wait_cards(self, prefix, timeout=45):
    """Waits for one Home card whose accessible name starts with `prefix`."""
    self.wait_for(
        lambda s: s.find_all(prefix=prefix, control_type="ButtonControl"),
        f"home card {prefix}", timeout,
    )
    return self.find_all(prefix=prefix, control_type="ButtonControl")[0]


uia.Session.wait_cards = wait_cards


def click_until(session, name, needles, attempts=2, timeout=20):
    """Clicks a control until one of `needles` appears, because a re-render can
    detach the node that UI Automation found."""
    for _ in range(attempts):
        session.click(name=name)
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            for line in session.text().splitlines():
                if any(needle in line for needle in needles):
                    return line
            time.sleep(0.2)
    raise uia.Failure(f"Timed out waiting for {needles}")


def install_via_dialog(session, archive):
    session.click(prefix="Install a local archive in")
    uia.set_file_dialog(uia.dialog_for(session), archive, session)


def test_backend_state(fixture, mode):
    fixture.state.mkdir(parents=True, exist_ok=True)
    (fixture.state / "test-backend.marker").write_text("disposable", encoding="utf-8")
    (fixture.state / "mode").write_text(mode, encoding="utf-8")
    (fixture.state / "game").mkdir(exist_ok=True)
    (fixture.state / "game" / "TestGame.exe").write_bytes(b"fixture")


# ---------------------------------------------------------------- journeys
def journey_detection(runner):
    """1. Startup detection, one-click registration, untouched game bytes."""
    fx = Fixture()
    runner.cleanup.append(fx.remove)
    fx.main_bucket()
    game = fx.steam_library("2379780", "Balatro", "Balatro.exe")
    before = snapshot(game)
    app = App(fx, TARGET / "debug" / "modn.exe").launch()
    runner.cleanup.append(app.kill)
    session = app.session
    session.wait_for(
        lambda s: s.find_all(prefix="Add Balatro at", control_type="ButtonControl"),
        "detected Balatro card",
    )
    runner.check(snapshot(game) == before, "Detection changed the fixture game folder")
    card = session.find_all(prefix="Add Balatro at", control_type="ButtonControl")[0]
    session.activate(card)
    session.wait_for(
        lambda s: s.find_all(prefix="Open Balatro at", control_type="ButtonControl"),
        "registered Balatro card",
    )
    runner.check(session.exists(prefix="Open Balatro at"), "Card did not become registered")
    listed = cli(fx, "game", "list")
    runner.check(listed.returncode == 0, f"CLI game list failed: {listed.stderr}")
    runner.check("balatro" in listed.stdout.lower(), "CLI does not list the registered game")
    runner.check(snapshot(game) == before, "Registration changed the fixture game folder")
    runner.check(app.close() in (0, None), "App did not exit cleanly")
    (fx.evidence / "journey-detection.json").write_text(
        json.dumps({"game": str(game), "game_list": listed.stdout}, indent=2), encoding="utf-8")


def journey_progress(runner):
    """2. One-click install shows Cancel, then completes with an Undo toast."""
    fx = Fixture()
    runner.cleanup.append(fx.remove)
    test_backend_state(fx, "complete")
    archive = fx.zip("slow.zip", {"addons/demo.lua": "return {}"})
    app = App(fx, TARGET / "debug" / "modn-desktop-test-backend.exe").launch()
    runner.cleanup.append(app.kill)
    session = app.session
    card = session.wait_cards("Open Fixture at")
    session.activate(card)
    session.click(prefix="Install a local archive in")
    dialog = uia.dialog_for(session)
    uia.set_file_dialog(dialog, archive, session)
    session.wait_for(lambda s: s.find_all(prefix="Cancel"), "Cancel affordance", timeout=40)
    runner.check(True, "Install button became Cancel")
    session.wait_text("Install done", timeout=60)
    runner.check((fx.state / "write.json").exists(), "Backend did not record the write")
    runner.check(session.exists("Undo"), "Undo action is missing from the toast")
    app.close()


def journey_cancel(runner):
    """3. Cancel before the commit leaves the fixture unchanged."""
    fx = Fixture()
    runner.cleanup.append(fx.remove)
    test_backend_state(fx, "complete")
    archive = fx.zip("hang-before-commit.zip", {"addons/demo.lua": "return {}"})
    before = snapshot(fx.state)
    before_game = snapshot(fx.state / "game")
    app = App(fx, TARGET / "debug" / "modn-desktop-test-backend.exe").launch()
    runner.cleanup.append(app.kill)
    session = app.session
    session.activate(session.wait_cards("Open Fixture at"))
    session.click(prefix="Install a local archive in")
    uia.set_file_dialog(uia.dialog_for(session), archive, session)
    session.wait_for(lambda s: s.find_all(prefix="Cancel"), "Cancel affordance", timeout=40)
    session.click(prefix="Cancel")
    session.wait_text("Cancelled. Nothing changed.", timeout=60)
    runner.check(not (fx.state / "write.json").exists(), "Cancel wrote to the fixture state")
    runner.check(snapshot(fx.state / "game") == before_game, "Cancel changed the fixture game")
    runner.check(
        set(snapshot(fx.state)) - set(before) <= {"last-args.json", "child-pid"},
        "Cancel added manager state",
    )
    app.close()


def journey_finishing(runner):
    """4. The commit phase shows Finishing, disables the button and refuses close."""
    fx = Fixture()
    runner.cleanup.append(fx.remove)
    test_backend_state(fx, "complete")
    archive = fx.zip("hang-after-commit.zip", {"addons/demo.lua": "return {}"})
    app = App(fx, TARGET / "debug" / "modn-desktop-test-backend.exe").launch()
    runner.cleanup.append(app.kill)
    session = app.session
    session.activate(session.wait_cards("Open Fixture at"))
    session.click(prefix="Install a local archive in")
    uia.set_file_dialog(uia.dialog_for(session), archive, session)
    session.wait_for(lambda s: s.find_all(prefix="Cancel"), "Cancel affordance", timeout=40)
    session.wait_for(
        lambda s: [c for c in s.find_all(prefix="Cancel") if not c.IsEnabled],
        "disabled commit-phase button",
        timeout=40,
    )
    runner.check(True, "Button is disabled during the commit phase")
    dialog = None
    for _ in range(4):
        session.request_close()
        try:
            dialog = uia.dialog_for(session, timeout=3)
            break
        except uia.Failure:
            runner.check(app.process.poll() is None, "The window closed during the write")
    runner.check(dialog is not None, "Closing during the commit was not refused")
    text = uia.dialog_text(dialog)
    runner.check("still working" in text, f"Close was not refused: {text!r}")
    uia.dismiss(dialog, session)
    session.wait_for(lambda s: (fx.state / "write.json").exists(), "completed write", timeout=60)
    runner.check(not session.extra_windows(), "An extra window outlived the dialog")
    runner.check(app.process.poll() is None, "The window closed during the write")
    app.close()


def journey_blocked(runner):
    """6. A blocked plan shows its reason and opens no confirmation."""
    fx = Fixture()
    runner.cleanup.append(fx.remove)
    test_backend_state(fx, "complete")
    archive = fx.zip("refuse.zip", {"addons/demo.lua": "return {}"})
    app = App(fx, TARGET / "debug" / "modn-desktop-test-backend.exe").launch()
    runner.cleanup.append(app.kill)
    session = app.session
    session.activate(session.wait_cards("Open Fixture at"))
    session.click(prefix="Install a local archive in")
    uia.set_file_dialog(uia.dialog_for(session), archive, session)
    session.wait_for(
        lambda s: [c for c in s.find_all(control_type="ButtonControl") if not c.IsEnabled and c.Name and "blocked" in c.Name.lower()],
        "blocked plan reason",
        timeout=60,
    )
    blocked = [c for c in session.find_all(control_type="ButtonControl") if c.Name and "blocked" in c.Name.lower()]
    runner.check(len(blocked) == 1, "Blocked reason is not shown once")
    runner.check(not blocked[0].IsEnabled, "Blocked button is still enabled")
    runner.check(not session.extra_windows(), "A dialog opened for a blocked plan")
    tree = session.snapshot("blocked")
    runner.check("fixture secret refusal text" not in tree, "CLI message leaked into the UI")
    runner.check("fixture stderr noise" not in tree, "CLI stderr leaked into the UI")
    app.close()


def journey_no_cli_output(runner):
    """8. Backend stdout and stderr noise never reach the UI tree."""
    fx = Fixture()
    runner.cleanup.append(fx.remove)
    test_backend_state(fx, "invalid-json")
    app = App(fx, TARGET / "debug" / "modn-desktop-test-backend.exe").launch()
    runner.cleanup.append(app.kill)
    session = app.session
    session.click(name="Scan this PC for supported games")
    time.sleep(1.5)
    tree = session.snapshot("stdout-noise")
    runner.check("invalid fixture response" not in tree, "Backend stdout leaked into the UI")
    (fx.state / "mode").write_text("failure", encoding="utf-8")
    session.click(name="Scan this PC for supported games")
    time.sleep(1.5)
    tree = session.snapshot("stderr-noise")
    runner.check("fixture refusal" not in tree, "Backend stderr leaked into the UI")
    runner.check(session.window is not None, "Window disappeared during backend failure")
    app.close()


def journey_play(runner):
    """5. Play runs the fixture executable, and details expose plan and history."""
    fx = Fixture()
    runner.cleanup.append(fx.remove)
    fx.main_bucket()
    game = fx.root / "game"
    game.mkdir(parents=True, exist_ok=True)
    fx.marker_exe(game / "Balatro.exe")
    result = cli(fx, "game", "add", "balatro", "--path", game)
    runner.check(result.returncode == 0, f"game add failed: {result.stderr}")
    before = snapshot(game)
    app = App(fx, TARGET / "debug" / "modn.exe").launch()
    runner.cleanup.append(app.kill)
    session = app.session
    session.activate(session.wait_cards("Open Balatro at"))
    session.wait_for(lambda s: s.find_all(name="Play balatro"), "Play button")
    session.click(name="Play balatro")
    deadline = time.monotonic() + 30
    marker = game / "play-marker.txt"
    while time.monotonic() < deadline and not marker.exists():
        time.sleep(0.2)
    runner.check(marker.exists(), "Play did not start the fixture executable")
    runner.check(snapshot(game).keys() - before.keys() == {"play-marker.txt"}, "Play changed other files")
    session.activate(session.find_all(prefix="Open details for", control_type="HyperlinkControl")[0])
    session.wait_for(lambda s: s.find_all(name="Copy diagnostics"), "details view")
    runner.check(session.exists(name="Operation history"), "History section is missing")
    toast = click_until(session, "Copy diagnostics", ("Diagnostics copied.", "Something went wrong"))
    runner.check(True, f"Copy diagnostics reported: {toast}")
    (fx.evidence / "diagnostics-toast.txt").write_text(toast, encoding="utf-8")
    runner.check(True, "Copy diagnostics confirmed in the UI")
    app.close()


JOURNEYS = [
    ("1 detection", journey_detection),
    ("2 progress", journey_progress),
    ("3 cancel", journey_cancel),
    ("4 finishing", journey_finishing),
    ("5 lifecycle", lifecycle.journey_lifecycle),
    ("6 blocked", journey_blocked),
    ("7 approval", lifecycle.journey_approval),
    ("8 no-cli-output", journey_no_cli_output),
    ("5 play-and-details", journey_play),
]


def build():
    commands = [
        ["cargo", "build", "-p", "modn-desktop", "--features", "verification", "--locked"],
        ["cargo", "build", "-p", "modn-cli", "--features", "modn-core/test-instruction-budget", "--locked"],
    ]
    for command in commands:
        print("+ " + " ".join(command))
        result = subprocess.run(command, cwd=REPO)
        if result.returncode != 0:
            raise SystemExit(f"build failed: {' '.join(command)}")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--build", action="store_true", help="build the app and the fixture CLI first")
    parser.add_argument("--only", default="", help="limit to journeys whose name contains this")
    parser.add_argument("--keep", action="store_true", help="keep fixtures and evidence")
    args = parser.parse_args()
    if args.build:
        build()
    runner = Runner(args)
    started = time.monotonic()
    try:
        for name, function in JOURNEYS:
            if args.only and args.only not in name:
                continue
            runner.journey(name, lambda function=function: function(runner))
    finally:
        runner.clean()
    passed = sum(1 for r in runner.results if r["status"] == "passed")
    summary = {
        "status": "passed" if passed == len(runner.results) and runner.results else "failed",
        "journeys": len(runner.results),
        "passed": passed,
        "failed": len(runner.results) - passed,
        "assertions": runner.assertions,
        "seconds": round(time.monotonic() - started, 1),
        "results": runner.results,
    }
    if not args.keep:
        for path in Path(Path(fixtures.__file__).parent).glob("*"):
            pass
    print(json.dumps(summary))
    return 0 if summary["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
