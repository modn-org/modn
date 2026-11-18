"""The game-agnostic Desktop journeys, in run order.

Each journey builds a disposable fixture and drives one Modn window. The shared
app driver and helpers live in `harness.py`.
"""

import json
import os
import sys
import time

import clipboard
import uia
from fixtures import PLAY_SOURCE, Fixture, image_pids, signed_copy, snapshot, stop_pid
from harness import BINARIES, App, cli, test_backend_state
from windows import watch_windows


# ---------------------------------------------------------------- journeys
def journey_detection(runner):
    """1. Startup detection, one-click registration, untouched game bytes."""
    fx = Fixture()
    runner.cleanup.append(fx.remove)
    fx.main_bucket()
    game = fx.steam_library("2379780", "Balatro", "Balatro.exe")
    before = snapshot(game)
    app = App(fx, BINARIES.cli).launch()
    runner.cleanup.append(app.kill)
    session = app.session
    card = session.wait_control(prefix="Add Balatro at", control_type="ButtonControl")
    runner.check(snapshot(game) == before, "Detection changed the fixture game folder")
    session.activate(card)
    card = session.wait_control(prefix="Open Balatro at", control_type="ButtonControl")
    runner.check(str(game) in card.Name, "Card did not become registered")
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
    app = App(fx, BINARIES.test_backend).launch()
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
    app = App(fx, BINARIES.test_backend).launch()
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
    app = App(fx, BINARIES.test_backend).launch()
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
    session.request_close()
    dialog = uia.dialog_for(session, timeout=15)
    runner.check(dialog.Name == "Modn", f"Close dialog has an unexpected title: {dialog.Name!r}")
    dialogs = uia.titled_windows(session)
    runner.check(
        len(dialogs) == 1,
        f"One close request opened {len(dialogs)} dialogs: {[d.Name for d in dialogs]}",
    )
    text = uia.dialog_text(dialog)
    runner.check(
        "Modn is still working. Wait for the operation to finish." in text.splitlines(),
        f"Close was not refused with the expected text: {text!r}",
    )
    uia.dismiss(dialog, session)
    session.wait_for(
        lambda s: not uia.titled_windows(s),
        "every close dialog dismissed",
        timeout=10,
    )
    runner.check(not uia.titled_windows(session), "A titled window outlived the dialog")
    runner.check(app.process.poll() is None, "The window closed during the write")
    session.wait_for(lambda s: (fx.state / "write.json").exists(), "completed write", timeout=60)
    session.wait_text("Install done", timeout=60)
    runner.check(session.exists("Undo"), "Undo action is missing from the toast")
    app.close()


def journey_blocked(runner):
    """6. A blocked plan shows its reason and opens no confirmation."""
    fx = Fixture()
    runner.cleanup.append(fx.remove)
    test_backend_state(fx, "complete")
    archive = fx.zip("refuse.zip", {"addons/demo.lua": "return {}"})
    app = App(fx, BINARIES.test_backend).launch()
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
    runner.check(
        blocked[0].Name == "Modn blocked this operation. Open details for the reason.",
        f"Unexpected blocked reason: {blocked[0].Name!r}",
    )
    runner.check(not blocked[0].IsEnabled, "Blocked button is still enabled")
    extra = [
        (w.Name, w.ClassName, w.ControlTypeName, w.AutomationId, str(w.BoundingRectangle))
        for w in session.extra_windows()
        if w.Name.strip()
    ]
    runner.check(not extra, f"A dialog opened for a blocked plan: {extra}")
    tree = session.snapshot("blocked")
    for text in [
        "fixture secret refusal text",
        "fixture stderr noise",
        "fixture refusal",
        "invalid fixture response",
    ]:
        runner.check(text not in tree, f"Backend text leaked into the UI: {text}")
    app.close()


SCAN_BUTTON = "Scan this PC for supported games"
BACKEND_NOISE = ("invalid fixture response", "fixture refusal")
DIAGNOSTICS_COPIED = "Diagnostics copied."
DIAGNOSTICS_ERROR = "Something went wrong"
DIAGNOSTICS_CLIPBOARD_FAILED = "Modn could not copy diagnostics to the clipboard."
REPORT_KEYS = ("scan", "diagnostics", "created_directories")
TOAST_SECONDS = 15.0
PLAY_WINDOW_WATCH_SECONDS = 1.5


def diagnostics_copied(toast):
    """True when the Copy diagnostics toast reports success."""
    return DIAGNOSTICS_COPIED in (toast or "")


def diagnostics_report(text):
    """Returns the doctor report in `text`, or None when `text` is not one.

    The clipboard must hold the pretty-printed doctor JSON. A JSON object with
    the report's keys proves the Rust write; any other text does not.
    """
    try:
        value = json.loads(text or "")
    except (TypeError, ValueError):
        return None
    if not isinstance(value, dict):
        return None
    if any(key not in value for key in REPORT_KEYS):
        return None
    return value


def same_path(left, right):
    """True when two Windows paths name one location.

    The CLI reports a verbatim path such as `\\\\?\\C:\\game` while the fixture
    holds `C:\\game`; the prefix and the letter case do not change the target.
    """
    def normalized(value):
        text = os.path.normcase(os.path.abspath(str(value)))
        return text[4:] if text.startswith("\\\\?\\") else text

    return normalized(left) == normalized(right)


def wait_toast(session, clock=time.monotonic, sleep=time.sleep, timeout=TOAST_SECONDS):
    """Returns the first Copy diagnostics toast line, or raises `uia.Failure`."""
    deadline = clock() + timeout
    while clock() < deadline:
        for line in session.text().splitlines():
            if (
                DIAGNOSTICS_COPIED in line
                or DIAGNOSTICS_ERROR in line
                or DIAGNOSTICS_CLIPBOARD_FAILED in line
            ):
                return line
        sleep(0.05)
    raise uia.Failure(f"Copy diagnostics showed no toast within {timeout} seconds")


def _restore_clipboard(previous, read_clipboard, write_clipboard):
    """Puts `previous` back only while the clipboard still holds our report.

    The human may copy other text while the journey runs, so a restore could
    overwrite it. A read failure means no restore and no exception.
    """
    try:
        current = read_clipboard()
    except Exception:
        return
    if current == previous or diagnostics_report(current) is None:
        return
    try:
        write_clipboard(previous)
    except Exception:
        pass


def copy_diagnostics(
    session, read_clipboard, write_clipboard, clock=time.monotonic, sleep=time.sleep
):
    """Activates Copy diagnostics once and returns the toast and clipboard report.

    The activation uses UI Automation patterns, so it needs no foreground and
    changes no focus. The system clipboard is saved before the activation. It is
    restored only when a fresh read still shows this journey's doctor report,
    and the text differs from the saved text; text the human copied meanwhile
    stays. A restore failure never fails the journey. Raises `uia.Failure` when
    the toast reports an error or clipboard failure, or when the clipboard does
    not hold the doctor report.
    """
    previous = read_clipboard()
    try:
        session.activate(session.find(name="Copy diagnostics"))
        toast = wait_toast(session, clock=clock, sleep=sleep)
        if not diagnostics_copied(toast):
            raise uia.Failure(f"Copy diagnostics reported: {toast}")
        report = diagnostics_report(read_clipboard())
        if report is None:
            raise uia.Failure(
                "Copy diagnostics did not put the doctor report on the clipboard"
            )
        return toast, report
    finally:
        _restore_clipboard(previous, read_clipboard, write_clipboard)


def _scan(session):
    """Waits for the rendered Scan button, then clicks it once."""
    session.wait_for(
        lambda s: s.find_all(name=SCAN_BUTTON, control_type="ButtonControl"),
        "Scan button",
    )
    session.wait_for(
        lambda s: [c for c in s.find_all(name=SCAN_BUTTON) if c.IsEnabled],
        "enabled Scan button",
    )
    session.click(name=SCAN_BUTTON)


def _assert_no_noise(runner, session, needle, seconds=3):
    """Polls the whole owned window tree, including toasts and dialogs."""
    leaked = []
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        tree = session.tree_text()
        if needle in tree:
            leaked.append(tree)
        time.sleep(0.2)
    runner.check(not leaked, f"Backend text leaked into the UI: {needle}")


def journey_no_cli_output(runner):
    """8. Backend stdout and stderr noise never reach the UI tree."""
    fx = Fixture()
    runner.cleanup.append(fx.remove)
    test_backend_state(fx, "invalid-json")
    app = App(fx, BINARIES.test_backend).launch()
    runner.cleanup.append(app.kill)
    session = app.session
    _scan(session)
    _assert_no_noise(runner, session, "invalid fixture response")
    session.snapshot("stdout-noise")
    (fx.state / "mode").write_text("failure", encoding="utf-8")
    _scan(session)
    _assert_no_noise(runner, session, "fixture refusal")
    session.snapshot("stderr-noise")
    runner.check(session.window is not None, "Window disappeared during backend failure")
    tree = session.tree_text()
    for needle in BACKEND_NOISE:
        runner.check(needle not in tree, f"Backend text leaked into the UI: {needle}")
    app.close()


def journey_play(runner):
    """9. Play starts the fixture executable, and details expose plan and history."""
    fx = Fixture()
    runner.cleanup.append(fx.remove)
    fx.main_bucket()
    game = fx.root / "game"
    game.mkdir(parents=True, exist_ok=True)
    exe = signed_copy(game / "Balatro.exe", PLAY_SOURCE)
    result = cli(fx, "game", "add", "balatro", "--path", game)
    runner.check(result.returncode == 0, f"game add failed: {result.stderr}")
    before = snapshot(game)
    app = App(fx, BINARIES.cli).launch()
    runner.cleanup.append(app.kill)
    started = []
    try:
        session = app.session
        session.activate(session.wait_cards("Open Balatro at"))
        session.wait_for(lambda s: s.find_all(name="Play balatro"), "Play button")
        clicked = time.monotonic()
        session.click(name="Play balatro")
        deadline = clicked + 60
        while time.monotonic() < deadline and not started:
            started = image_pids(exe)
            if not started:
                time.sleep(0.05)
        seen_after = time.monotonic() - clicked
        (fx.evidence / "journey-play.json").write_text(
            json.dumps(
                {
                    "image": str(exe),
                    "started_pids": started,
                    "first_seen_seconds": round(seen_after, 2),
                    "ui_text_tail": session.text()[-2000:],
                },
                indent=2,
            ),
            encoding="utf-8",
        )
        runner.check(bool(started), f"Play did not start {exe}")
        shown = watch_windows(started, PLAY_WINDOW_WATCH_SECONDS)
        runner.check(not shown, f"The play fixture showed a window or dialog: {shown}")
        for pid in started:
            stop_pid(pid)
        deadline = time.monotonic() + 15
        remaining = image_pids(exe)
        while time.monotonic() < deadline and remaining:
            time.sleep(0.2)
            remaining = image_pids(exe)
        runner.check(not remaining, f"Play process outlived its stop: {remaining}")
        runner.check(snapshot(game) == before, "Play changed the fixture game folder")
        (fx.evidence / "journey-play.json").write_text(
            json.dumps(
                {
                    "image": str(exe),
                    "started_pids": started,
                    "first_seen_seconds": round(seen_after, 2),
                    "fixture_windows": shown,
                    "stopped_pids": started,
                    "remaining_pids": remaining,
                    "snapshot_unchanged": snapshot(game) == before,
                },
                indent=2,
            ),
            encoding="utf-8",
        )
        session.activate(session.wait_control(prefix="Open details for", control_type="HyperlinkControl"))
        session.wait_for(lambda s: s.find_all(name="Copy diagnostics"), "details view")
        runner.check(session.exists(name="Operation history"), "History section is missing")
        toast, report = copy_diagnostics(session, clipboard.read_text, clipboard.write_text)
        installation = report.get("scan", {}).get("installation")
        runner.check(
            isinstance(installation, str) and same_path(installation, game),
            f"Clipboard doctor report names {installation!r}, not {game}",
        )
        (fx.evidence / "diagnostics.json").write_text(
            json.dumps(
                {
                    "toast": toast,
                    "report_keys": sorted(report),
                    "installation": installation,
                },
                indent=2,
            ),
            encoding="utf-8",
        )
        app.close()
    finally:
        for pid in image_pids(exe):
            stop_pid(pid)
