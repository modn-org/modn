"""Real-CLI lifecycle and native-code approval journeys.

The module is imported by `verify.py`. It imports `verify` back only inside a
journey, after that module has finished loading, to reuse the app driver.
"""

import hashlib
import json
import time
from pathlib import Path

import uia
from fixtures import Fixture, run, snapshot, write

REPO = Path(__file__).resolve().parents[4]


def _driver():
    import verify

    return verify


def _cli(fx, *args):
    return _driver().cli(fx, *args)


def _app(fx, backend):
    return _driver().App(fx, backend)


def _target():
    return _driver().TARGET / "debug"


def _install_archive(session, archive):
    _driver().install_via_dialog(session, archive)


def _open_game(session, name):
    session.activate(session.wait_cards(f"Open {name} at"))


def _back_home(session):
    session.click(name="Back to games", control_type="HyperlinkControl")
    session.wait_for(
        lambda s: s.find_all(prefix="Open ", control_type="ButtonControl"),
        "home cards after Back to games",
    )


def _click_any(session, name):
    matches = session.find_all(name=name)
    uia.require(len(matches) == 1, f"Expected one control named {name}, found {len(matches)}")
    session.activate(matches[0])


def _wait_file(path, present=True, timeout=60):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if path.exists() == present:
            return
        time.sleep(0.2)
    raise uia.Failure(f"Timed out waiting for {path} present={present}")


def _packages(fx, game):
    result = _cli(fx, "list", "--path", game)
    uia.require(result.returncode == 0, f"CLI list failed: {result.stderr}")
    return json.loads(result.stdout)


def publish(fx, bucket, files, *, version="1.0.0", native=False, name="Fixture Pack"):
    """Adds one synthetic catalog release and its verified cache entry."""
    mod_id = name.lower().replace(" ", "-")
    archive = fx.zip(f"{mod_id}-{version}.zip", files)
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    url = f"https://fixture.invalid/{mod_id}-{version}.zip"
    manifest = {
        "schema_version": 1,
        "id": mod_id,
        "game": "balatro",
        "name": name,
        "version": version,
        "archive_format": "zip",
        "sha256": digest,
        "source": {"kind": "https", "url": url},
        "native_injector": native,
    }
    write(bucket / "games" / "balatro" / f"{mod_id}.json", json.dumps(manifest))
    url_hash = hashlib.sha256(url.encode()).hexdigest()[:7]
    cache = fx.root / "cache"
    cache.mkdir(exist_ok=True)
    (cache / f"balatro#{mod_id}#{version}#{url_hash}.zip").write_bytes(archive.read_bytes())
    return mod_id


def journey_lifecycle(runner):
    """5. Install, uninstall, both undo paths, diagnostics and details."""
    fx = Fixture()
    runner.cleanup.append(fx.remove)
    bucket = fx.main_bucket()
    game = fx.root / "game"
    game.mkdir()
    (game / "Balatro.exe").write_bytes(b"fixture")
    registered = _cli(fx, "game", "add", "balatro", "--path", game)
    runner.check(registered.returncode == 0, f"game add failed: {registered.stderr}")
    content = fx.root / "appdata" / "Balatro" / "Mods"
    archive = fx.zip("demo-pack.zip", {"demo.lua": "return {}"})
    app = _app(fx, _target() / "modn.exe").launch()
    runner.cleanup.append(app.kill)
    session = app.session
    _open_game(session, "Balatro")
    _install_archive(session, archive)
    session.wait_text("Install done", timeout=90)
    runner.check((content / "demo.lua").is_file(), "The archive install wrote no file")
    packages = _packages(fx, game)
    runner.check(any(pkg.get("mod_id") == "demo-pack" for pkg in packages), f"CLI does not list demo-pack: {packages}")
    session.wait_for(
        lambda s: s.find_all(prefix="Uninstall demo-pack"),
        "uninstall button after install",
    )
    session.click(prefix="Uninstall demo-pack")
    session.wait_text("Uninstall done", timeout=90)
    _wait_file(content / "demo.lua", present=False)
    runner.check(not _packages(fx, game), "Uninstall left a manager package")

    _install_archive(session, archive)
    session.wait_text("Install done", timeout=90)
    session.click(name="Undo")
    _wait_file(content / "demo.lua", present=False)
    runner.check(not _packages(fx, game), "Toast undo left a manager package")
    runner.check(session.exists(name="Undo") is False, "Dismissed toast kept its Undo action")

    publish(fx, bucket, {"FixturePack/init.lua": "return {}"})
    _back_home(session)
    _open_game(session, "Balatro")
    session.click(prefix="Install Fixture Pack")
    session.wait_text("Install done", timeout=90)
    runner.check(
        (content / "FixturePack" / "init.lua").is_file(),
        "Catalog install wrote no file",
    )
    session.click(prefix="Uninstall Fixture Pack")
    session.wait_text("Uninstall done", timeout=90)
    _wait_file(content / "FixturePack" / "init.lua", present=False)
    session.click(name="Recent")
    session.click(prefix="Undo Uninstalled")
    _wait_file(content / "FixturePack" / "init.lua")
    runner.check(True, "Recent menu undo restored the catalog files")

    session.click(prefix="Open details for Fixture Pack")
    session.wait_text("History", timeout=60)
    runner.check(session.exists(name="Operation history"), "History section is missing")
    _click_any(session, "Details")
    session.wait_text("No approval needed.", timeout=60)
    runner.check(True, "Details plan expanded and reported no approval")
    session.focus()
    session.click(name="Copy diagnostics")
    session.wait_text("Diagnostics copied.", timeout=30)
    copied = uia.clipboard_text()
    report = json.loads(copied)
    runner.check(isinstance(report, dict) and report, "Clipboard does not hold a JSON report")
    tree = session.snapshot("details")
    runner.check('"schema_version"' not in tree and '"content_root"' not in tree,
                 "Diagnostics JSON is visible in the window tree")
    app.close()


def journey_approval(runner):
    """7. The native-code ask, stored approval and a release that adds native code."""
    fx = Fixture()
    runner.cleanup.append(fx.remove)
    bucket = fx.main_bucket()
    game = fx.root / "game"
    game.mkdir()
    (game / "Balatro.exe").write_bytes(b"fixture")
    registered = _cli(fx, "game", "add", "balatro", "--path", game)
    runner.check(registered.returncode == 0, f"game add failed: {registered.stderr}")
    publish(fx, bucket, {"winmm.dll": "MZ"}, name="Native Pack", native=True)
    publish(fx, bucket, {"plain.lua": "return {}"}, name="Plain Pack")
    app = _app(fx, _target() / "modn.exe").launch()
    runner.cleanup.append(app.kill)
    session = app.session
    _open_game(session, "Balatro")

    session.click(prefix="Install Native Pack")
    session.wait_for(lambda s: s.find_all(name="Native code approval"), "native-code ask")
    runner.check(session.exists(name="Decline"), "Ask has no Decline action")
    session.click(name="Decline")
    runner.check(not session.exists(name="Native code approval"), "Decline kept the ask")
    runner.check(not (game / "winmm.dll").exists(), "Decline wrote the native library")

    session.click(prefix="Install Native Pack")
    session.wait_for(lambda s: s.find_all(name="Allow"), "native-code ask after decline")
    session.click(name="Allow")
    session.wait_text("Install done", timeout=90)
    runner.check((game / "winmm.dll").is_file(), "Allow did not install the native library")
    approval = _cli(fx, "list", "--path", game)
    runner.check(approval.returncode == 0, "CLI list failed after Allow")

    session.click(prefix="Uninstall Native Pack")
    session.wait_text("Uninstall done", timeout=90)
    _wait_file(game / "winmm.dll", present=False)
    session.click(prefix="Install Native Pack")
    session.wait_text("Install done", timeout=90)
    runner.check(not session.exists(name="Native code approval"), "Stored approval did not suppress the ask")
    _wait_file(game / "winmm.dll")

    session.click(prefix="Install Plain Pack")
    session.wait_text("Install done", timeout=90)
    publish(fx, bucket, {"winmm.dll": "MZ"}, version="2.0.0", name="Plain Pack", native=True)
    _back_home(session)
    _open_game(session, "Balatro")
    session.click(prefix="Update Plain Pack")
    session.wait_for(lambda s: s.find_all(name="Native code approval"), "ask for a release that adds native code")
    session.click(name="Allow")
    session.wait_text("Update done", timeout=90)
    runner.check((game / "winmm.dll").is_file(), "Update did not install the native release")
    app.close()
