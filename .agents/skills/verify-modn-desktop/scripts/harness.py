"""Shared verifier harness: bootstrap, binaries, app driver and helpers.

Importing this module re-runs the entry script under `uv` when the interpreter
lacks `uiautomation`, then imports the UI Automation driver. The re-run happens
at import time so that the `uia` import below always succeeds.
"""

import json
import os
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

REPO = Path(__file__).resolve().parents[4]
TARGET = Path(os.environ["CARGO_TARGET_DIR"]) if os.environ.get("CARGO_TARGET_DIR") else REPO / "target"


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
        str(Path(sys.argv[0]).resolve()), *sys.argv[1:],
    ]
    raise SystemExit(subprocess.call(command))


bootstrap()

import fixtures  # noqa: E402
import uia  # noqa: E402
from fixtures import Fixture, run, snapshot  # noqa: E402

BLOCKED_PLAN = None


class Binaries:
    """The three executables the journeys drive, with CLI overrides applied."""

    def __init__(self, target=None):
        debug = Path(target or TARGET) / "debug"
        self.app = debug / "modn-desktop.exe"
        self.cli = debug / "modn.exe"
        self.test_backend = debug / "modn-desktop-test-backend.exe"

    def override(self, app=None, cli=None, test_backend=None):
        if app:
            self.app = Path(app)
        if cli:
            self.cli = Path(cli)
        if test_backend:
            self.test_backend = Path(test_backend)

    def paths(self):
        return [self.app, self.cli, self.test_backend]

    def missing(self):
        return [path for path in self.paths() if not path.is_file()]


BINARIES = Binaries()


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
            str(BINARIES.app),
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
        # Closing must never answer a destructive confirmation: only OK and
        # Cancel are safe here, and Yes/No dialogs are left for the journey.
        for dialog in self.session.extra_windows():
            for answer in ("OK", "Cancel"):
                if answer in uia.dialog_buttons(dialog):
                    try:
                        uia.answer_dialog(dialog, self.session, answer, timeout=2)
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
        if self.process is None:
            return
        if self.process.poll() is None:
            self.process.kill()
        try:
            self.process.wait(timeout=10)
        except subprocess.TimeoutExpired as error:
            raise uia.Failure(f"App process {self.process.pid} did not exit") from error


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
        mark = len(self.cleanup)
        error_message = None
        try:
            function()
        except Exception as error:  # noqa: BLE001 - every failure is recorded
            error_message = str(error)
        actions = self.cleanup[mark:]
        del self.cleanup[mark:]
        failures = self.clean(actions)
        if error_message and failures:
            status = f"failed: {error_message}; cleanup: {'; '.join(failures)}"
        elif error_message:
            status = f"failed: {error_message}"
        elif failures:
            status = f"failed: cleanup: {'; '.join(failures)}"
        else:
            status = "passed"
        self.results.append({
            "journey": name,
            "status": status,
            "seconds": round(time.monotonic() - started, 1),
        })
        print(f"  {name}: {status}")

    def clean(self, actions=None):
        failures = []
        remaining = self.cleanup if actions is None else actions
        for action in reversed(remaining):
            try:
                action()
            except Exception as error:  # noqa: BLE001 - each cleanup failure is reported
                failures.append(f"{action}: {error}")
        if actions is None:
            self.cleanup.clear()
        return failures


def cli(fixture, *args):
    return run(
        [BINARIES.cli, "--json", "--state-dir", fixture.state, *args],
        env=fixture.environment(),
    )


def wait_control(self, name=None, prefix=None, control_type=None, timeout=45):
    """Waits for a control and returns the node the wait found.

    Re-renders can replace a node between a wait and a second lookup, so
    callers activate the returned node instead of searching again.
    """
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        matches = self.find_all(name=name, prefix=prefix, control_type=control_type)
        if matches:
            return matches[0]
        time.sleep(0.2)
    raise uia.Failure(f"Timed out waiting for control {name or prefix}")


def wait_cards(self, prefix, timeout=45):
    """Waits for one Home card whose accessible name starts with `prefix`."""
    return self.wait_control(prefix=prefix, control_type="ButtonControl", timeout=timeout)


uia.Session.wait_control = wait_control
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
