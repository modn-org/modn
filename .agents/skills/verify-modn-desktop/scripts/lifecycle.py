"""Real-CLI lifecycle and native-code approval journeys.

The module is imported by `verify.py`. It imports `harness` back only inside a
journey, after that module has finished loading, to reuse the app driver.
"""

import json
import re
import time

import uia
from catalog_fixture import add_catalog_release
from fixtures import Fixture, snapshot

GAME_ID = "balatro"
PICKER_TITLE = "Choose a local ZIP or 7z archive"
ARCHIVE_NAME = "demo-pack.zip"
ARCHIVE_FILES = {
    "DemoPack/DemoPack.json": '{"id":"DemoPack","version":"1.0.0"}',
    "DemoPack/init.lua": 'return { name = "DemoPack" }',
}
LEAK_NEEDLES = ('{"', '"error"', "Traceback")
RAW_CODE = re.compile(r"\b[A-Z][A-Z0-9]+_[A-Z0-9_]+\b")


def _driver():
    import harness

    return harness


def _cli(fx, *args):
    return _driver().cli(fx, *args)


def _app(fx, backend):
    return _driver().App(fx, backend)


def _cli_path():
    return _driver().BINARIES.cli


def _open_game(session, name):
    session.activate(session.wait_cards(f"Open {name} at"))


def _back_home(session):
    session.click(name="Back to games", control_type="HyperlinkControl")
    session.wait_for(
        lambda s: s.find_all(prefix="Open ", control_type="ButtonControl"),
        "home cards after Back to games",
    )


def _content(fx):
    return fx.root / "appdata" / "Balatro" / "Mods"


def _bytes_state(fx, game):
    return {"game": snapshot(game), "content": snapshot(_content(fx))}


def _packages(fx, game):
    result = _cli(fx, "--game", GAME_ID, "list", "--path", game)
    uia.require(result.returncode == 0, f"CLI list failed: {result.stderr}")
    return json.loads(result.stdout)


def _history(fx, game):
    result = _cli(fx, "--game", GAME_ID, "history")
    uia.require(result.returncode == 0, f"CLI history failed: {result.stderr}")
    return json.loads(result.stdout)


def _leaks(text):
    found = [needle for needle in LEAK_NEEDLES if needle in text]
    found.extend(RAW_CODE.findall(text))
    return found


def _assert_clean_ui(runner, session, step):
    runner.check(not _leaks(session.snapshot(step)), "CLI output leaked into the window tree")


def _wait_no_titled(session, description, timeout=20):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if not uia.titled_windows(session):
            return
        time.sleep(0.05)
    titles = [window.Name for window in uia.titled_windows(session)]
    raise uia.Failure(f"Timed out waiting for {description}: {titles}")


def _wait_toast(session, message, context, timeout=90):
    """Waits for a toast while rejecting any titled window and CLI leak."""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        windows = [window.Name for window in uia.titled_windows(session)]
        if windows:
            raise uia.Failure(f"A titled window opened during {context}: {windows}")
        tree = session.text()
        leaked = _leaks(tree)
        if leaked:
            raise uia.Failure(f"CLI output leaked into the UI during {context}: {leaked}")
        if message in tree:
            return
        time.sleep(0.1)
    raise uia.Failure(
        f"Timed out waiting for {message!r} after {context}; last window text: {tree[-1500:]}"
    )


def _toast_undo(session):
    session.wait_for(
        lambda s: [c for c in s.find_all(name="Undo", control_type="ButtonControl") if c.IsEnabled],
        "enabled Undo toast action",
        timeout=8,
    )
    matches = [
        c for c in session.find_all(name="Undo", control_type="ButtonControl") if c.IsEnabled
    ]
    uia.require(len(matches) == 1, f"Expected one Undo toast action, found {len(matches)}")
    session.activate(matches[0])


def _pick_archive(session, archive):
    dialog = uia.dialog_for(session)
    uia.set_file_dialog(dialog, archive, session)
    _wait_no_titled(session, "the archive picker to close")


def _history_buttons(session):
    lists = session.find_all(name="Operation history", control_type="ListControl")
    if len(lists) != 1:
        return []
    return session.find_all(prefix="Undo ", control_type="ButtonControl", window=lists[0])


def _center(control):
    point = control.GetClickablePoint()
    return point if isinstance(point, tuple) else (point.x, point.y)


def _expand_details(session):
    """Opens the Details summary, which ignores the Invoke pattern."""
    session.wait_for(lambda s: s.find_all(name="Details"), "Details summary")
    candidates = session.find_all(name="Details")
    control = candidates[0]
    for candidate in candidates:
        try:
            candidate.GetExpandCollapsePattern()
            control = candidate
            break
        except Exception:
            continue
    try:
        control.GetExpandCollapsePattern().Expand()
    except Exception:
        pass
    deadline = time.monotonic() + 3
    while time.monotonic() < deadline:
        if session.find_all(name="File plan"):
            return
        time.sleep(0.1)
    session.focus()
    uia.auto.Click(*_center(control))


def journey_lifecycle(runner):
    """5. Local archive one-click lifecycle, both undo paths, history and diagnostics."""
    fx = Fixture()
    runner.cleanup.append(fx.remove)
    fx.main_bucket()
    game = fx.root / "game"
    game.mkdir()
    (game / "Balatro.exe").write_bytes(b"fixture")
    registered = _cli(fx, "game", "add", GAME_ID, "--path", game)
    runner.check(registered.returncode == 0, f"game add failed: {registered.stderr}")
    content = _content(fx)
    archive = fx.zip(ARCHIVE_NAME, ARCHIVE_FILES)
    before = _bytes_state(fx, game)
    app = _app(fx, _cli_path()).launch()
    runner.cleanup.append(app.kill)
    session = app.session
    _open_game(session, "Balatro")

    # 1. Cancel the native picker: one titled window, no toast, no change.
    session.click(name=f"Install a local archive in {GAME_ID}")
    dialog = uia.dialog_for(session)
    windows = uia.titled_windows(session)
    runner.check(len(windows) == 1, f"Picker cancel opened {len(windows)} titled windows")
    runner.check(dialog.Name == PICKER_TITLE, f"Picker title is {dialog.Name!r}")
    uia.cancel_file_dialog(dialog)
    _wait_no_titled(session, "the picker to close after Cancel")
    session.wait_for(
        lambda s: [c for c in s.find_all(name=f"Install a local archive in {GAME_ID}") if c.IsEnabled],
        "enabled install-archive button after Cancel",
    )
    runner.check(not session.exists(name="Install done"), "Picker Cancel showed an install toast")
    runner.check(
        not session.exists(name="Cancelled. Nothing changed."),
        "Picker Cancel started an operation and showed its toast",
    )
    runner.check(not _packages(fx, game), "Picker Cancel created a CLI package")
    runner.check(_bytes_state(fx, game) == before, "Picker Cancel changed the game or content bytes")
    _assert_clean_ui(runner, session, "lifecycle-1-cancel")

    # 2. Install the archive in one click, with no confirmation window.
    session.click(name=f"Install a local archive in {GAME_ID}")
    _pick_archive(session, archive)
    _wait_toast(session, "Install done", "archive install")
    runner.check(session.exists(name="Undo"), "Install toast has no Undo action")
    installed_files = {
        name: (content / name).read_bytes() for name in ARCHIVE_FILES
    }
    runner.check(
        installed_files == {name: value.encode() for name, value in ARCHIVE_FILES.items()},
        "The archive install wrote different bytes",
    )
    packages = _packages(fx, game)
    runner.check(
        any(pkg.get("mod_id") == "demo-pack" for pkg in packages),
        f"CLI does not list demo-pack: {packages}",
    )
    installed = _bytes_state(fx, game)
    runner.check(installed != before, "The archive install left no file on disk")
    _assert_clean_ui(runner, session, "lifecycle-2-install")

    # 3. Undo from the toast restores the pre-install bytes.
    _toast_undo(session)
    _wait_toast(session, "Undo done", "toast undo")
    runner.check(not _packages(fx, game), "Toast undo left a CLI package")
    runner.check(_bytes_state(fx, game) == before, "Toast undo did not restore the original bytes")
    _assert_clean_ui(runner, session, "lifecycle-3-undo")

    # 4. Install the same archive from the catalog, then uninstall it in one click.
    release = add_catalog_release(
        fx, GAME_ID, "demo-pack", "1.0.0", ARCHIVE_FILES, name="Demo Pack"
    )
    runner.check(release.mod_id == "demo-pack", f"Catalog release id is {release.mod_id!r}")
    _back_home(session)
    _open_game(session, "Balatro")
    session.wait_for(
        lambda s: [c for c in s.find_all(name="Install Demo Pack") if c.IsEnabled],
        "enabled Install Demo Pack",
    )
    session.click(name="Install Demo Pack")
    _wait_toast(session, "Install done", "catalog install")
    runner.check(_bytes_state(fx, game) == installed, "The catalog install wrote other bytes")
    session.wait_for(
        lambda s: [c for c in s.find_all(name="Uninstall Demo Pack") if c.IsEnabled],
        "enabled Uninstall Demo Pack",
    )
    session.click(name="Uninstall Demo Pack")
    _wait_toast(session, "Uninstall done", "uninstall")
    runner.check(not _packages(fx, game), "Uninstall left a CLI package")
    runner.check(_bytes_state(fx, game) == before, "Uninstall did not restore the original bytes")
    _assert_clean_ui(runner, session, "lifecycle-4-uninstall")

    # 5. Undo the uninstall from the history row: the same files come back.
    session.activate(
        session.wait_control(prefix="Open details for Demo Pack", control_type="HyperlinkControl")
    )
    session.wait_text("History", timeout=60)
    session.wait_for(
        lambda s: s.find_all(name="Undo Uninstalled", control_type="ButtonControl"),
        "history undo for the uninstall",
    )
    matches = session.find_all(name="Undo Uninstalled", control_type="ButtonControl")
    runner.check(bool(matches), "History has no undo for the uninstall")
    session.activate(matches[0])
    _wait_toast(session, "Undo done", "history undo")
    runner.check(
        any(pkg.get("mod_id") == "demo-pack" for pkg in _packages(fx, game)),
        "Undo of the uninstall did not restore the CLI package",
    )
    runner.check(
        _bytes_state(fx, game) == installed,
        "Undo of the uninstall did not restore the installed bytes",
    )
    _assert_clean_ui(runner, session, "lifecycle-5-history-undo")

    # 6. The CLI history and the Mod details rows agree on every operation.
    operations = _history(fx, game)
    runner.check(len(operations) == 5, f"CLI history has {len(operations)} operations, not 5")
    runner.check(
        [op.get("kind") for op in reversed(operations)]
        == ["install", "uninstall", "install", "uninstall", "install"],
        f"CLI history order is {[op.get('kind') for op in reversed(operations)]}",
    )
    runner.check(
        operations[3].get("undoes") == operations[4].get("id"),
        "The first undo does not name the first install",
    )
    runner.check(
        operations[0].get("undoes") == operations[1].get("id"),
        "The recent undo does not name the uninstall",
    )
    runner.check(
        all(
            operation.get("packages")
            and operation["packages"][0].get("mod_id") == "demo-pack"
            for operation in operations
        ),
        "A history operation does not name demo-pack",
    )
    session.wait_for(
        lambda s: s.exists(name="Operation history"), "history section after the undo"
    )
    runner.check(session.exists(name="Operation history"), "History section is missing")
    session.wait_for(
        lambda s: len(_history_buttons(s)) == len(operations),
        "history rows to match the CLI",
    )
    runner.check(
        len(_history_buttons(session)) == len(operations),
        "Mod details history does not agree with the CLI count",
    )
    _assert_clean_ui(runner, session, "lifecycle-6-history")

    # 7. Details still reports a reviewable plan without CLI output.
    _expand_details(session)
    session.wait_text("No approval needed.", timeout=60)
    runner.check(
        session.exists(name="No approval needed."),
        "Details plan did not report that no approval is needed",
    )
    runner.check(bool(session.find_all(name="File plan")), "Details plan lists no file operations")
    runner.check(
        not _leaks(session.snapshot("lifecycle-7-details")),
        "Details view shows CLI output",
    )
    app.close()
