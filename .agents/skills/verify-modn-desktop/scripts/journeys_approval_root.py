"""The content-root executable approval journey: ask once per mod.

Journey 7 proves the executable ask on the tools-game fixture, whose installs
write below the game folder. This journey proves the same policy for a game
whose module declares a content root: the Balatro fixture installs an
executable-declaring catalog release into `appdata/Balatro/Mods`.
"""

from approval_support import (
    approval_rows as _approval_rows,
    ask_groups as _ask_groups,
    check_approval as _check_approval,
    check_ask as _check_ask,
    check_bytes as _check_bytes,
    clear_toasts as _clear_toasts,
    packages as _packages,
    version_of as _version,
    wait_ask as _wait_ask,
    wait_enabled as _wait_enabled,
    wait_toast_no_ask as _wait_toast_no_ask,
)
from catalog_fixture import add_catalog_release
from fixtures import Fixture, signed_copy
from lifecycle import (
    GAME_ID,
    _app,
    _assert_clean_ui,
    _bytes_state,
    _cli,
    _cli_path,
    _content,
    _open_game,
    _wait_toast,
)

ROOT_EXE_ID = "root-exe-pack"
ROOT_EXE_NAME = "Root Exe Pack"
ROOT_FOLDER = "RootPack"
ROOT_LUA_PATH = f"{ROOT_FOLDER}/pack.lua"
ROOT_EXE_PATH = f"{ROOT_FOLDER}/tools/helper.exe"


def release_files(helper):
    """The archive files of the content-root executable release.

    The Balatro fixture plans a package only when one top folder holds every
    file, so both entries sit below `RootPack`. The executable is declared at
    its destination path, which the module places under the content root.
    """
    return {
        ROOT_LUA_PATH: f'return {{ name = "{ROOT_EXE_ID}" }}',
        ROOT_EXE_PATH: helper,
    }


def journey_approval_content_root(runner):
    """12. An executable ask on a content-root game, remembered after Allow."""
    fx = Fixture()
    runner.cleanup.append(fx.remove)
    fx.main_bucket()
    game = fx.root / "game"
    game.mkdir()
    (game / "Balatro.exe").write_bytes(b"fixture")
    registered = _cli(fx, "game", "add", GAME_ID, "--path", game)
    runner.check(registered.returncode == 0, f"game add failed: {registered.stderr}")

    helper = signed_copy(fx.archives / "helper-source.exe", "hostname.exe").read_bytes()
    files = release_files(helper)
    add_catalog_release(
        fx,
        GAME_ID,
        ROOT_EXE_ID,
        "1.0.0",
        files,
        executables=[ROOT_EXE_PATH],
        name=ROOT_EXE_NAME,
    )
    before = _bytes_state(fx, game)

    app = _app(fx, _cli_path()).launch()
    runner.cleanup.append(app.kill)
    session = app.session
    _open_game(session, "Balatro")

    # 1. The first executable ask writes nothing, and Decline forgets it.
    _wait_enabled(session, f"Install {ROOT_EXE_NAME}")
    session.click(name=f"Install {ROOT_EXE_NAME}")
    _wait_ask(session, "the content-root executable ask")
    _check_ask(runner, session, False, [ROOT_EXE_PATH], "content-root executable ask")
    _assert_clean_ui(runner, session, "approval-root-1-ask")
    runner.check(not _packages(fx, GAME_ID, game), "The content-root ask installed a package")
    runner.check(
        _bytes_state(fx, game) == before,
        "The content-root ask changed game or content bytes",
    )
    runner.check(
        not _approval_rows(fx, ROOT_EXE_ID),
        "The content-root ask stored an approval before the answer",
    )
    session.click(name="Decline")
    session.wait_for(lambda s: not _ask_groups(s), "the content-root ask to close after Decline")
    runner.check(not session.exists(name="Install done"), "Decline showed an install toast")
    runner.check(not _packages(fx, GAME_ID, game), "Decline installed root-exe-pack")
    runner.check(_bytes_state(fx, game) == before, "Decline changed game or content bytes")
    runner.check(not _approval_rows(fx, ROOT_EXE_ID), "Decline stored an approval")
    _assert_clean_ui(runner, session, "approval-root-1-decline")

    # 2. Allow installs into the content root and stores the approval.
    session.click(name=f"Install {ROOT_EXE_NAME}")
    _wait_ask(session, "the content-root executable ask after Decline")
    session.click(name="Allow")
    _wait_toast(session, "Install done", "content-root executable install after Allow")
    runner.check(
        session.exists(name="Undo"), "The content-root install toast has no Undo action"
    )
    packages = _packages(fx, GAME_ID, game)
    runner.check(
        _version(packages, ROOT_EXE_ID) == "1.0.0",
        f"CLI lists {_version(packages, ROOT_EXE_ID)!r} for root-exe-pack: {packages}",
    )
    _check_bytes(runner, _content(fx), files, "The content-root executable install")
    _check_approval(
        runner, fx, ROOT_EXE_ID, False, [ROOT_EXE_PATH], "After the content-root Allow"
    )
    runner.check(
        _bytes_state(fx, game) != before, "The content-root install left no file on disk"
    )
    _assert_clean_ui(runner, session, "approval-root-2-allow")

    # 3. Uninstall restores the earlier bytes and keeps the approval.
    _clear_toasts(session)
    _wait_enabled(session, f"Uninstall {ROOT_EXE_NAME}")
    session.click(name=f"Uninstall {ROOT_EXE_NAME}")
    _wait_toast(session, "Uninstall done", "content-root executable uninstall")
    runner.check(
        not _packages(fx, GAME_ID, game), "The uninstall left root-exe-pack installed"
    )
    runner.check(
        _bytes_state(fx, game) == before,
        "The content-root executable uninstall did not restore the original bytes",
    )
    _check_approval(
        runner, fx, ROOT_EXE_ID, False, [ROOT_EXE_PATH], "The content-root executable uninstall"
    )
    _assert_clean_ui(runner, session, "approval-root-3-uninstall")

    # 4. The stored approval suppresses the next ask for the same mod.
    _clear_toasts(session)
    session.click(name=f"Install {ROOT_EXE_NAME}")
    seen = _wait_toast_no_ask(
        session, "Install done", "content-root reinstall with the stored approval"
    )
    runner.check(
        not seen, "A stored content-root executable approval still asked during the reinstall"
    )
    runner.check(
        not _ask_groups(session),
        "The stored content-root executable approval still shows an ask",
    )
    packages = _packages(fx, GAME_ID, game)
    runner.check(
        _version(packages, ROOT_EXE_ID) == "1.0.0",
        f"CLI lists {_version(packages, ROOT_EXE_ID)!r} after the reinstall: {packages}",
    )
    _check_bytes(runner, _content(fx), files, "The content-root executable reinstall")
    _check_approval(
        runner,
        fx,
        ROOT_EXE_ID,
        False,
        [ROOT_EXE_PATH],
        "The content-root executable reinstall",
    )
    _assert_clean_ui(runner, session, "approval-root-4-remembered")

    # 5. No ask is left over, and the window stayed free of CLI output.
    runner.check(not _ask_groups(session), "An approval ask stayed open at the end")
    _assert_clean_ui(runner, session, "approval-root-5-final")
    runner.check(app.close() in (0, None), "App did not exit cleanly")
    runner.check(
        app.process.poll() is not None,
        "The app process outlived the content-root approval journey",
    )
