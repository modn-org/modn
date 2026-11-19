"""The approval-policy journey: one ask per capability, then remember it.

It proves issue #152: a native-injector or executable release asks once per
mod, Decline writes nothing, the approval is stored in manager state, and a
later release that adds native code asks again.
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
from fixtures import Fixture, signed_copy, snapshot
from lifecycle import (
    GAME_ID,
    _app,
    _assert_clean_ui,
    _back_home,
    _bytes_state,
    _cli,
    _cli_path,
    _content,
    _open_game,
    _wait_toast,
)

TOOLS_GAME_ID = "tools-game"
TOOLS_GAME_NAME = "Tools Game"
NATIVE_ID = "native-pack"
NATIVE_NAME = "Native Pack"
EXE_ID = "exe-pack"
EXE_NAME = "Exe Pack"
PLAIN_ID = "plain-pack"
PLAIN_NAME = "Plain Pack"
NATIVE_FILES = {"winmm.dll": b"MZ native code 1.0.0"}
PLAIN_V1 = {"plain.lua": 'return { version = "1.0.0" }'}
PLAIN_V2 = {"winmm.dll": b"MZ native code 2.0.0"}
PLAIN_V3 = {"winmm.dll": b"MZ native code 3.0.0"}


def journey_approval(runner):
    """7. Native and executable asks, stored approval, and a release that adds native code."""
    fx = Fixture()
    runner.cleanup.append(fx.remove)
    fx.main_bucket()
    game = fx.root / "game"
    game.mkdir()
    (game / "Balatro.exe").write_bytes(b"fixture")
    tools_game = fx.root / "tools-game"
    tools_game.mkdir()
    (tools_game / "Game.exe").write_bytes(b"fixture")
    for game_id, path in ((GAME_ID, game), (TOOLS_GAME_ID, tools_game)):
        registered = _cli(fx, "game", "add", game_id, "--path", path)
        runner.check(registered.returncode == 0, f"game add {game_id} failed: {registered.stderr}")

    helper = signed_copy(fx.archives / "helper-source.exe", "hostname.exe").read_bytes()
    exe_files = {"pack.lua": 'return { name = "exe-pack" }', "tools/helper.exe": helper}
    add_catalog_release(
        fx, GAME_ID, NATIVE_ID, "1.0.0", NATIVE_FILES, native_injector=True
    )
    add_catalog_release(fx, GAME_ID, PLAIN_ID, "1.0.0", PLAIN_V1)
    add_catalog_release(
        fx,
        TOOLS_GAME_ID,
        EXE_ID,
        "1.0.0",
        exe_files,
        executables=["tools/helper.exe"],
    )
    before = _bytes_state(fx, game)
    before_tools = snapshot(tools_game)

    app = _app(fx, _cli_path()).launch()
    runner.cleanup.append(app.kill)
    session = app.session
    _open_game(session, "Balatro")

    # 1. The first native-code ask writes nothing, and Decline forgets it.
    _wait_enabled(session, f"Install {NATIVE_NAME}")
    session.click(name=f"Install {NATIVE_NAME}")
    _wait_ask(session, "the first native-code ask")
    _check_ask(runner, session, True, [], "native-code ask")
    _assert_clean_ui(runner, session, "approval-1-ask")
    runner.check(not _packages(fx, GAME_ID, game), "The native ask installed a package")
    runner.check(
        _bytes_state(fx, game) == before, "The native ask changed game or content bytes"
    )
    runner.check(
        not _approval_rows(fx, NATIVE_ID), "The native ask stored an approval before the answer"
    )
    session.click(name="Decline")
    session.wait_for(lambda s: not _ask_groups(s), "the native ask to close after Decline")
    runner.check(not session.exists(name="Install done"), "Decline showed an install toast")
    runner.check(not _packages(fx, GAME_ID, game), "Decline installed native-pack")
    runner.check(_bytes_state(fx, game) == before, "Decline changed game or content bytes")
    runner.check(not _approval_rows(fx, NATIVE_ID), "Decline stored an approval")
    _assert_clean_ui(runner, session, "approval-1-decline")

    # 2. Allow installs the archive and stores its native-code approval.
    session.click(name=f"Install {NATIVE_NAME}")
    _wait_ask(session, "the native-code ask after Decline")
    session.click(name="Allow")
    _wait_toast(session, "Install done", "native install after Allow")
    runner.check(session.exists(name="Undo"), "The native install toast has no Undo action")
    packages = _packages(fx, GAME_ID, game)
    runner.check(
        _version(packages, NATIVE_ID) == "1.0.0",
        f"CLI lists {_version(packages, NATIVE_ID)!r} for native-pack: {packages}",
    )
    _check_bytes(runner, game, NATIVE_FILES, "The native install")
    _check_approval(runner, fx, NATIVE_ID, True, [], "After Allow")
    runner.check(
        _bytes_state(fx, game) != before, "The native install left no file on disk"
    )
    _assert_clean_ui(runner, session, "approval-2-allow")

    # 3. A stored approval survives uninstall and suppresses the next ask.
    _clear_toasts(session)
    _wait_enabled(session, f"Uninstall {NATIVE_NAME}")
    session.click(name=f"Uninstall {NATIVE_NAME}")
    _wait_toast(session, "Uninstall done", "native uninstall")
    runner.check(not _packages(fx, GAME_ID, game), "The uninstall left native-pack installed")
    runner.check(
        _bytes_state(fx, game) == before,
        "The native uninstall did not restore the original bytes",
    )
    _check_approval(runner, fx, NATIVE_ID, True, [], "The native uninstall")
    _assert_clean_ui(runner, session, "approval-3-uninstall")
    _clear_toasts(session)
    session.click(name=f"Install {NATIVE_NAME}")
    seen = _wait_toast_no_ask(
        session, "Install done", "native reinstall with the stored approval"
    )
    runner.check(not seen, "A stored native approval still asked during the reinstall")
    runner.check(not _ask_groups(session), "The stored native approval still shows an ask")
    packages = _packages(fx, GAME_ID, game)
    runner.check(
        _version(packages, NATIVE_ID) == "1.0.0",
        f"CLI lists {_version(packages, NATIVE_ID)!r} after the reinstall: {packages}",
    )
    _check_bytes(runner, game, NATIVE_FILES, "The native reinstall")
    _check_approval(runner, fx, NATIVE_ID, True, [], "The native reinstall")
    _assert_clean_ui(runner, session, "approval-3-remembered")

    # native-pack owns `winmm.dll` at the game root, the same destination the
    # plain-pack native release needs. Remove it so the later update plan is
    # not blocked by a destination conflict; the stored approval stays.
    _clear_toasts(session)
    _wait_enabled(session, f"Uninstall {NATIVE_NAME}")
    session.click(name=f"Uninstall {NATIVE_NAME}")
    _wait_toast(session, "Uninstall done", "native cleanup before the plain-pack update")
    runner.check(not _packages(fx, GAME_ID, game), "The native cleanup left native-pack")
    runner.check(
        _bytes_state(fx, game) == before,
        "The native cleanup did not restore the original bytes",
    )
    _check_approval(runner, fx, NATIVE_ID, True, [], "The native cleanup")
    _assert_clean_ui(runner, session, "approval-3-cleanup")

    # 4. An executable asks with its path, then its stored approval is remembered.
    _back_home(session)
    _open_game(session, TOOLS_GAME_NAME)
    _clear_toasts(session)
    _wait_enabled(session, f"Install {EXE_NAME}")
    session.click(name=f"Install {EXE_NAME}")
    _wait_ask(session, "the executable ask")
    _check_ask(runner, session, False, ["tools/helper.exe"], "executable ask")
    _assert_clean_ui(runner, session, "approval-4-ask")
    runner.check(not _packages(fx, TOOLS_GAME_ID, tools_game), "The executable ask installed a package")
    runner.check(
        snapshot(tools_game) == before_tools,
        "The executable ask changed game bytes",
    )
    runner.check(
        not _approval_rows(fx, EXE_ID), "The executable ask stored an approval before the answer"
    )
    session.click(name="Allow")
    _wait_toast(session, "Install done", "executable install after Allow")
    packages = _packages(fx, TOOLS_GAME_ID, tools_game)
    runner.check(
        _version(packages, EXE_ID) == "1.0.0",
        f"CLI lists {_version(packages, EXE_ID)!r} for exe-pack: {packages}",
    )
    _check_bytes(runner, tools_game, exe_files, "The executable install")
    _check_approval(
        runner, fx, EXE_ID, False, ["tools/helper.exe"], "After the executable Allow"
    )
    _assert_clean_ui(runner, session, "approval-4-allow")

    _clear_toasts(session)
    _wait_enabled(session, f"Uninstall {EXE_NAME}")
    session.click(name=f"Uninstall {EXE_NAME}")
    _wait_toast(session, "Uninstall done", "executable uninstall")
    runner.check(not _packages(fx, TOOLS_GAME_ID, tools_game), "The uninstall left exe-pack installed")
    runner.check(
        snapshot(tools_game) == before_tools,
        "The executable uninstall did not restore the original bytes",
    )
    _check_approval(
        runner, fx, EXE_ID, False, ["tools/helper.exe"], "The executable uninstall"
    )
    _assert_clean_ui(runner, session, "approval-4-uninstall")
    _clear_toasts(session)
    session.click(name=f"Install {EXE_NAME}")
    seen = _wait_toast_no_ask(
        session, "Install done", "executable reinstall with the stored approval"
    )
    runner.check(not seen, "A stored executable approval still asked during the reinstall")
    runner.check(not _ask_groups(session), "The stored executable approval still shows an ask")
    packages = _packages(fx, TOOLS_GAME_ID, tools_game)
    runner.check(
        _version(packages, EXE_ID) == "1.0.0",
        f"CLI lists {_version(packages, EXE_ID)!r} after the reinstall: {packages}",
    )
    _check_bytes(runner, tools_game, exe_files, "The executable reinstall")
    _check_approval(
        runner, fx, EXE_ID, False, ["tools/helper.exe"], "The executable reinstall"
    )
    _assert_clean_ui(runner, session, "approval-4-remembered")

    # 5. A release that adds native code asks again; the next release does not.
    _back_home(session)
    _open_game(session, "Balatro")
    _clear_toasts(session)
    _wait_enabled(session, f"Install {PLAIN_NAME}")
    session.click(name=f"Install {PLAIN_NAME}")
    seen = _wait_toast_no_ask(session, "Install done", "plain 1.0.0 install")
    runner.check(not seen, "The plain 1.0.0 install asked for an approval")
    packages = _packages(fx, GAME_ID, game)
    runner.check(
        _version(packages, PLAIN_ID) == "1.0.0",
        f"CLI lists {_version(packages, PLAIN_ID)!r} for plain-pack: {packages}",
    )
    _check_bytes(runner, _content(fx), PLAIN_V1, "The plain 1.0.0 install")
    rows = _approval_rows(fx, PLAIN_ID)
    runner.check(
        all(not native and not paths for native, paths in rows),
        f"plain-pack 1.0.0 stored an approved capability: {rows}",
    )
    _assert_clean_ui(runner, session, "approval-5-plain")

    add_catalog_release(
        fx, GAME_ID, PLAIN_ID, "2.0.0", PLAIN_V2, native_injector=True
    )
    _back_home(session)
    _open_game(session, "Balatro")
    _clear_toasts(session)
    _wait_enabled(session, f"Update {PLAIN_NAME}")
    runner.check(
        session.exists(name="Update available"),
        "The plain-pack 2.0.0 tile shows no Update available",
    )
    session.click(name=f"Update {PLAIN_NAME}")
    _wait_ask(session, "the ask for a release that adds native code")
    _check_ask(runner, session, True, [], "native-code ask for plain-pack 2.0.0")
    _assert_clean_ui(runner, session, "approval-5-update-ask")
    session.click(name="Allow")
    _wait_toast(session, "Update done", "native update after Allow")
    packages = _packages(fx, GAME_ID, game)
    runner.check(
        _version(packages, PLAIN_ID) == "2.0.0",
        f"CLI lists {_version(packages, PLAIN_ID)!r} after the update: {packages}",
    )
    _check_bytes(runner, game, PLAIN_V2, "The native update")
    runner.check(
        not (_content(fx) / "plain.lua").exists(),
        "The native update kept the removed plain.lua",
    )
    _check_approval(runner, fx, PLAIN_ID, True, [], "After the native update")
    _assert_clean_ui(runner, session, "approval-5-update")

    add_catalog_release(
        fx, GAME_ID, PLAIN_ID, "3.0.0", PLAIN_V3, native_injector=True
    )
    _back_home(session)
    _open_game(session, "Balatro")
    _clear_toasts(session)
    _wait_enabled(session, f"Update {PLAIN_NAME}")
    runner.check(
        session.exists(name="Update available"),
        "The plain-pack 3.0.0 tile shows no Update available",
    )
    session.click(name=f"Update {PLAIN_NAME}")
    seen = _wait_toast_no_ask(
        session, "Update done", "covered native update to plain-pack 3.0.0"
    )
    runner.check(not seen, "The stored native approval asked again for plain-pack 3.0.0")
    runner.check(
        not _ask_groups(session),
        "The covered plain-pack 3.0.0 update still shows an ask",
    )
    packages = _packages(fx, GAME_ID, game)
    runner.check(
        _version(packages, PLAIN_ID) == "3.0.0",
        f"CLI lists {_version(packages, PLAIN_ID)!r} after the covered update: {packages}",
    )
    _check_bytes(runner, game, PLAIN_V3, "The covered native update")
    _check_approval(
        runner, fx, PLAIN_ID, True, [], "The covered native update to 3.0.0"
    )
    _assert_clean_ui(runner, session, "approval-5-covered-update")

    # 6. Steps 3 and 5 proved that a blocked or covered plan shows no ask. The
    # blocked journey owns the blocked-plan evidence; this journey only keeps
    # the invariant that no ask is left over at the end.
    runner.check(not _ask_groups(session), "An approval ask stayed open at the end")

    # 7. Every snapshot above and every toast poll rejected CLI output.
    _assert_clean_ui(runner, session, "approval-7-final")
    runner.check(app.close() in (0, None), "App did not exit cleanly")
    runner.check(
        app.process.poll() is not None, "The app process outlived the approval journey"
    )
