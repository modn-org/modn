"""The Remove from Modn journey: a native Yes/No confirmation.

One registered fixture game is removed twice. `No` must keep the registration,
`Yes` must unregister it, and the game folder must stay byte-for-byte
unchanged in both answers.
"""

import json
import time

import uia
from fixtures import Fixture, snapshot
from harness import BINARIES, App, cli

GAME_ID = "balatro"
GAME = "Balatro"
APP_ID = "2379780"
MARKER = "Balatro.exe"
WARNING = (
    "Remove balatro from Modn? Modn only unregisters this installation; "
    "every game file stays on disk."
)
REMOVE_TEXT = "Remove from Modn"


def _registered_cards(session):
    return session.find_all(prefix=f"Open {GAME} at", control_type="ButtonControl")


def _add_cards(session):
    return session.find_all(prefix=f"Add {GAME} at", control_type="ButtonControl")


def _remove_controls(session):
    return [
        control
        for control in session.find_all(control_type="ButtonControl")
        if control.Name and control.Name.startswith("Remove ") and control.Name.endswith("from Modn")
    ]


def _remove_control(session, timeout=20):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        controls = _remove_controls(session)
        if controls:
            uia.require(
                len(controls) == 1,
                f"Expected one {REMOVE_TEXT} control, found {len(controls)}",
            )
            return controls[0]
        time.sleep(0.2)
    raise uia.Failure(f"Timed out waiting for the {REMOVE_TEXT} control")


def _start_removal(session):
    """Clicks Remove from Modn and returns the native dialog it opened."""
    session.activate(_remove_control(session))
    return uia.dialog_for(session)


def _check_removal_dialog(runner, session, dialog):
    """Asserts the one native dialog is the removal confirmation."""
    dialogs = uia.titled_windows(session)
    runner.check(
        len(dialogs) == 1,
        f"Removal opened {len(dialogs)} native dialogs: {[d.Name for d in dialogs]}",
    )
    runner.check(dialog.Name == "Modn", f"Removal dialog title is {dialog.Name!r}")
    text = uia.dialog_text(dialog)
    runner.check(WARNING in text.splitlines(), f"Removal dialog text is {text!r}")
    buttons = uia.dialog_buttons(dialog)
    runner.check(
        sorted(buttons) == ["No", "Yes"],
        f"Removal dialog buttons are {buttons!r}",
    )


def _check_dialog_closed(runner, session):
    session.wait_for(
        lambda s: not uia.titled_windows(s),
        "every removal dialog dismissed",
        timeout=15,
    )
    runner.check(not uia.titled_windows(session), "A native dialog outlived its answer")


def _check_registered(runner, session, game, listed):
    card = session.wait_control(prefix=f"Open {GAME} at", control_type="ButtonControl")
    runner.check(str(game) in card.Name, f"Open card does not name {game}: {card.Name!r}")
    runner.check(not _add_cards(session), f"Home also shows an Add {GAME} card")
    runner.check(listed.returncode == 0, f"CLI game list failed: {listed.stderr}")
    runner.check(GAME_ID in listed.stdout.lower(), "CLI no longer lists the registered game")
    _remove_control(session)


def journey_remove(runner):
    """10. No keeps the registration; Yes unregisters and leaves game bytes."""
    fx = Fixture()
    runner.cleanup.append(fx.remove)
    fx.main_bucket()
    game = fx.steam_library(APP_ID, GAME, MARKER)
    before = snapshot(game)
    app = App(fx, BINARIES.cli).launch()
    runner.cleanup.append(app.kill)
    session = app.session

    session.activate(session.wait_control(prefix=f"Add {GAME} at", control_type="ButtonControl"))
    session.wait_for(lambda s: _registered_cards(s), f"registered {GAME} card")
    _check_registered(runner, session, game, cli(fx, "game", "list"))
    runner.check(snapshot(game) == before, "Registration changed the fixture game folder")
    session.snapshot("registered")

    dialog = _start_removal(session)
    time.sleep(0.5)
    _check_removal_dialog(runner, session, dialog)
    session.snapshot("remove-confirmation")
    uia.answer_dialog(dialog, session, "No")
    _check_dialog_closed(runner, session)
    runner.check(app.process.poll() is None, "The app exited after answering No")

    session.wait_for(
        lambda s: [control for control in _remove_controls(s) if control.IsEnabled],
        f"enabled {REMOVE_TEXT} control after No",
    )
    _check_registered(runner, session, game, cli(fx, "game", "list"))
    runner.check(snapshot(game) == before, "No changed the fixture game folder")
    session.snapshot("kept-after-no")

    dialog = _start_removal(session)
    time.sleep(0.5)
    _check_removal_dialog(runner, session, dialog)
    uia.answer_dialog(dialog, session, "Yes")
    _check_dialog_closed(runner, session)
    runner.check(app.process.poll() is None, "The app exited after answering Yes")

    session.wait_for(lambda s: not _registered_cards(s), f"no Open {GAME} card after Yes", timeout=15)
    card = session.wait_control(prefix=f"Add {GAME} at", control_type="ButtonControl", timeout=45)
    runner.check(str(game) in card.Name, f"Add card does not name {game}: {card.Name!r}")
    runner.check(not _remove_controls(session), f"{REMOVE_TEXT} still shows while unregistered")
    listed = cli(fx, "game", "list")
    runner.check(listed.returncode == 0, f"CLI game list failed: {listed.stderr}")
    runner.check(GAME_ID not in listed.stdout.lower(), "CLI still lists the removed game")
    runner.check(snapshot(game) == before, "Yes changed the fixture game folder")
    session.snapshot("unregistered-after-yes")

    result = app.close()
    runner.check(result in (0, None), f"App did not exit cleanly: {result!r}")
    runner.check(app.process.poll() is not None, "The app process outlived the journey")
    (fx.evidence / "journey-remove.json").write_text(
        json.dumps(
            {
                "game": str(game),
                "warning": WARNING,
                "game_list_after_yes": listed.stdout,
                "snapshot_unchanged": snapshot(game) == before,
            },
            indent=2,
        ),
        encoding="utf-8",
    )
