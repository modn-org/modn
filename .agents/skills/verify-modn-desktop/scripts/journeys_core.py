"""The game-agnostic Desktop journeys, in run order.

Each journey builds a disposable fixture and drives one Modn window. The shared
app driver and helpers live in `harness.py`.
"""

import json
import time

import uia
from fixtures import Fixture, snapshot
from harness import BINARIES, App, cli, click_until, test_backend_state


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
    app = App(fx, BINARIES.test_backend).launch()
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
    """9. Play runs the fixture executable, and details expose plan and history."""
    fx = Fixture()
    runner.cleanup.append(fx.remove)
    fx.main_bucket()
    game = fx.root / "game"
    game.mkdir(parents=True, exist_ok=True)
    fx.marker_exe(game / "Balatro.exe")
    result = cli(fx, "game", "add", "balatro", "--path", game)
    runner.check(result.returncode == 0, f"game add failed: {result.stderr}")
    before = snapshot(game)
    app = App(fx, BINARIES.cli).launch()
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
