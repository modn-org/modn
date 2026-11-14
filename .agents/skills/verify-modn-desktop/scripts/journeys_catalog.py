"""Catalog, update and genuine-7z journeys for the Modn desktop verifier.

The module is imported by `verify.py`. It reuses the app driver, the toast
checks and the archive picker that `lifecycle.py` owns, so a titled window and
any CLI output are rejected the same way in every journey.
"""

import os
import shutil
from pathlib import Path

import uia
from catalog_fixture import add_catalog_release
from fixtures import Fixture, require, run
from lifecycle import (
    GAME_ID,
    _app,
    _assert_clean_ui,
    _back_home,
    _bytes_state,
    _cli,
    _cli_path,
    _content,
    _history,
    _open_game,
    _packages,
    _pick_archive,
    _toast_undo,
    _wait_toast,
)

DEMO_ID = "demo-pack"
DEMO_NAME = "Demo Pack"
SEVEN_ID = "seven-pack"
SEVEN_ARCHIVE = "seven-pack.7z"
SEVEN_SIGNATURE = b"7z\xbc\xaf\x27\x1c"

V1_FILES = {
    "DemoPack/DemoPack.json": '{"id":"DemoPack","version":"1.0.0"}',
    "DemoPack/init.lua": 'return { name = "DemoPack", version = "1.0.0" }',
}
V2_FILES = {
    "DemoPack/DemoPack.json": '{"id":"DemoPack","version":"2.0.0"}',
    "DemoPack/init.lua": 'return { name = "DemoPack", version = "2.0.0" }',
    "DemoPack/release.lua": 'return { version = "2.0.0", note = "adds one file" }',
}
SEVEN_FILES = {
    "SevenPack/SevenPack.json": '{"id":"SevenPack","version":"1.0.0"}',
    "SevenPack/init.lua": 'return { name = "SevenPack" }',
    "SevenPack/data/extra.lua": 'return "packed by the installed 7-Zip"',
}


def seven_zip():
    """The installed 7-Zip command, or the loud failure the task requires."""
    for name in ("7z.exe", "7z"):
        found = shutil.which(name)
        if found:
            return found
    for variable in ("ProgramFiles", "ProgramFiles(x86)"):
        root = os.environ.get(variable)
        if root and (Path(root) / "7-Zip" / "7z.exe").is_file():
            return str(Path(root) / "7-Zip" / "7z.exe")
    raise uia.Failure("7-Zip is required: install it and put 7z on PATH")


def make_7z(command, folder, archive, files):
    """Writes `files` below `folder` and packs them with the real 7z command."""
    for name, content in files.items():
        target = folder / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(content.encode() if isinstance(content, str) else content)
    archive.parent.mkdir(parents=True, exist_ok=True)
    result = run([command, "a", "-bd", "-y", str(archive), *files], cwd=folder)
    require(result.returncode == 0, f"7z failed: {result.stderr.strip()}")
    require(
        archive.read_bytes()[:6] == SEVEN_SIGNATURE,
        f"{archive.name} is not a genuine 7z archive",
    )


def _version(packages, mod_id):
    for package in packages:
        if package.get("mod_id") == mod_id:
            return package.get("version")
    return None


def _expected(files):
    return {
        name: content.encode() if isinstance(content, str) else content
        for name, content in files.items()
    }


def _check_bytes(runner, content, files, message):
    expected = _expected(files)
    missing = [name for name in expected if not (content / name).is_file()]
    runner.check(not missing, f"{message}: missing {missing}")
    observed = {name: (content / name).read_bytes() for name in expected}
    runner.check(observed == expected, f"{message}: installed bytes differ")


def _wait_enabled(session, name, timeout=90):
    session.wait_for(
        lambda s: [c for c in s.find_all(name=name) if c.IsEnabled],
        f"enabled {name}",
        timeout=timeout,
    )


def journey_catalog(runner):
    """11. Catalog install, release update, toast undo and a real 7z archive."""
    seven = seven_zip()
    fx = Fixture()
    runner.cleanup.append(fx.remove)
    fx.main_bucket()
    game = fx.root / "game"
    game.mkdir()
    (game / "Balatro.exe").write_bytes(b"fixture")
    registered = _cli(fx, "game", "add", GAME_ID, "--path", game)
    runner.check(registered.returncode == 0, f"game add failed: {registered.stderr}")
    content = _content(fx)
    start = _bytes_state(fx, game)
    release = add_catalog_release(
        fx, GAME_ID, DEMO_ID, "1.0.0", V1_FILES, name=DEMO_NAME
    )
    runner.check(release.mod_id == DEMO_ID, f"Catalog release id is {release.mod_id!r}")
    app = _app(fx, _cli_path()).launch()
    runner.cleanup.append(app.kill)
    session = app.session
    _open_game(session, "Balatro")

    # 1. One click installs the 1.0.0 catalog release with no extra window.
    _wait_enabled(session, f"Install {DEMO_NAME}")
    runner.check("1.0.0" in session.text(), "The catalog tile shows no version 1.0.0")
    runner.check(session.exists(name="Not installed"), "The tile is not marked Not installed")
    session.click(name=f"Install {DEMO_NAME}")
    _wait_toast(session, "Install done", "catalog install")
    runner.check(session.exists(name="Undo"), "The catalog install toast has no Undo action")
    packages = _packages(fx, game)
    runner.check(
        _version(packages, DEMO_ID) == "1.0.0",
        f"CLI lists {_version(packages, DEMO_ID)!r} for demo-pack: {packages}",
    )
    _check_bytes(runner, content, V1_FILES, "The catalog install")
    v1_state = _bytes_state(fx, game)
    runner.check(v1_state != start, "The catalog install left no file on disk")
    _assert_clean_ui(runner, session, "catalog-1-install")
    # A toast action lives for eight seconds. Let the install toast expire so
    # the update toast is the only Undo on screen, and the click is unambiguous.
    session.wait_for(
        lambda s: not s.exists(name="Install done"),
        "the catalog install toast to expire",
        timeout=15,
    )

    # 2. A newer release advertises an update after the Library reloads.
    add_catalog_release(fx, GAME_ID, DEMO_ID, "2.0.0", V2_FILES, name=DEMO_NAME)
    _back_home(session)
    _open_game(session, "Balatro")
    _wait_enabled(session, f"Update {DEMO_NAME}")
    runner.check(session.exists(name="Update available"), "The tile shows no Update available")
    runner.check("2.0.0" in session.text(), "The tile shows no version 2.0.0")
    session.click(name=f"Update {DEMO_NAME}")
    _wait_toast(session, "Update done", "catalog update")
    runner.check(session.exists(name="Undo"), "The update toast has no Undo action")
    packages = _packages(fx, game)
    runner.check(
        _version(packages, DEMO_ID) == "2.0.0",
        f"CLI lists {_version(packages, DEMO_ID)!r} for demo-pack: {packages}",
    )
    _check_bytes(runner, content, V2_FILES, "The catalog update")
    v2_state = _bytes_state(fx, game)
    runner.check(v2_state != v1_state, "The catalog update changed no file")
    operations = _history(fx, game)
    runner.check(operations[0].get("kind") == "update", "CLI history has no newest update")
    runner.check(
        operations[0]["packages"][0]["release_after"]["version"] == "2.0.0",
        f"History update release is {operations[0]['packages'][0].get('release_after')}",
    )
    runner.check(
        operations[0]["packages"][0]["release_before"]["version"] == "1.0.0",
        f"History replaced release is {operations[0]['packages'][0].get('release_before')}",
    )
    _assert_clean_ui(runner, session, "catalog-2-update")

    # 3. The toast Undo replaces 2.0.0 with the exact 1.0.0 bytes again.
    _toast_undo(session)
    _wait_toast(session, "Undo done", "toast undo of the update")
    packages = _packages(fx, game)
    runner.check(
        _version(packages, DEMO_ID) == "1.0.0",
        f"CLI lists {_version(packages, DEMO_ID)!r} after the undo: {packages}",
    )
    _check_bytes(runner, content, V1_FILES, "The toast undo of the update")
    runner.check(
        _bytes_state(fx, game) == v1_state, "The toast undo did not restore the 1.0.0 bytes"
    )
    _assert_clean_ui(runner, session, "catalog-3-undo")

    # 4. A genuine .7z installs and uninstalls through one click each.
    before = _bytes_state(fx, game)
    archive = fx.archives / SEVEN_ARCHIVE
    make_7z(seven, fx.root / "seven-stage", archive, SEVEN_FILES)
    session.click(name=f"Install a local archive in {GAME_ID}")
    _pick_archive(session, archive)
    _wait_toast(session, "Install done", "7z install")
    packages = _packages(fx, game)
    runner.check(
        any(pkg.get("mod_id") == SEVEN_ID for pkg in packages),
        f"CLI does not list seven-pack: {packages}",
    )
    _check_bytes(runner, content, SEVEN_FILES, "The 7z install")
    runner.check(_bytes_state(fx, game) != before, "The 7z install left no file on disk")
    _assert_clean_ui(runner, session, "catalog-4-7z-install")
    _wait_enabled(session, f"Uninstall {SEVEN_ID}")
    session.click(name=f"Uninstall {SEVEN_ID}")
    _wait_toast(session, "Uninstall done", "7z uninstall")
    packages = _packages(fx, game)
    runner.check(
        not any(pkg.get("mod_id") == SEVEN_ID for pkg in packages),
        f"CLI still lists seven-pack: {packages}",
    )
    runner.check(
        _bytes_state(fx, game) == before, "The 7z uninstall did not restore the prior bytes"
    )
    runner.check(
        _version(packages, DEMO_ID) == "1.0.0", "The 7z uninstall disturbed demo-pack"
    )
    _assert_clean_ui(runner, session, "catalog-5-7z-uninstall")
    app.close()
