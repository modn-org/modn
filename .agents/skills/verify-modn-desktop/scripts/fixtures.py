"""Disposable fixtures for the Modn Desktop verifier.

Every path lives under a fresh temporary directory. No real game, Steam
library or `%LOCALAPPDATA%\\modn` state is read or written.
"""

import hashlib
import json
import os
import shutil
import subprocess
import tempfile
import zipfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
HIDDEN = subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def run(command, env=None, cwd=None, timeout=120):
    return subprocess.run(
        list(map(str, command)),
        env=env,
        cwd=cwd,
        capture_output=True,
        encoding="utf-8",
        errors="replace",
        timeout=timeout,
        creationflags=HIDDEN,
    )


def snapshot(folder):
    folder = Path(folder)
    if not folder.exists():
        return {}
    return {
        p.relative_to(folder).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest()
        for p in sorted(folder.rglob("*"))
        if p.is_file()
    }


def write(path, text):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


class Fixture:
    """One run's game, manager state, data directory and evidence."""

    def __init__(self, label=""):
        self.root = Path(tempfile.mkdtemp(prefix="modn-desktop-fixture-"))
        self.evidence = Path(tempfile.mkdtemp(prefix="modn-desktop-evidence-"))
        self.label = label
        self.state = self.root / "state"
        self.data = self.root / "data"
        self.game = self.root / "game"
        self.modules = self.root / "modules"
        self.steam = self.root / "steam"
        self.archives = self.root / "archives"
        for folder in (self.state, self.data, self.modules, self.archives, self.root / "appdata"):
            folder.mkdir(parents=True, exist_ok=True)
        self.evidence.mkdir(exist_ok=True)

    # -- environment ----------------------------------------------------
    def environment(self):
        env = dict(os.environ)
        env["LOCALAPPDATA"] = str(self.data)
        env["MODN_CACHE"] = str(self.root / "cache")
        stores = self.root / "store-libraries.json"
        stores.write_text(json.dumps({"steam_roots": [str(self.steam)]}), encoding="utf-8")
        env["MODN_STORE_LIBRARIES"] = str(stores)
        env["MODN_APPDATA"] = str(self.root / "appdata")
        env.pop("MODN_GAME_MODULES_DIR", None)
        return env

    def main_bucket(self):
        """Copies the repository's official bucket into the disposable data directory."""
        target = self.data / "modn" / "buckets" / "main"
        source = REPO / "buckets" / "main"
        require(source.is_dir(), f"Missing bucket source {source}")
        shutil.copytree(source, target)
        return target

    def steam_library(self, app_id, folder, marker):
        """Writes a fake Steam root that lists one installed game."""
        apps = self.steam / "steamapps"
        apps.mkdir(parents=True, exist_ok=True)
        quoted = str(self.steam).replace("\\", "\\\\")
        write(
            apps / "libraryfolders.vdf",
            f'"libraryfolders"\n{{\n\t"0"\n\t{{\n\t\t"path"\t\t"{quoted}"\n\t}}\n}}\n',
        )
        write(
            apps / f"appmanifest_{app_id}.acf",
            '"AppState"\n{\n'
            f'\t"appid"\t\t"{app_id}"\n\t"name"\t\t"Fixture"\n\t"installdir"\t\t"{folder}"\n'
            "}\n",
        )
        game = apps / "common" / folder
        game.mkdir(parents=True, exist_ok=True)
        (game / marker).write_bytes(b"fixture-game")
        return game

    def register(self, cli, game_id, game, env=None):
        result = run(
            [cli, "--json", "--state-dir", self.state, "game", "add", game_id, "--path", game],
            env=env or self.environment(),
        )
        require(result.returncode == 0, f"game add failed: {result.stderr}")
        return result

    def zip(self, name, files):
        path = self.archives / name
        with zipfile.ZipFile(path, "w") as archive:
            for inner, content in files.items():
                archive.writestr(inner, content)
        return path

    def marker_exe(self, target):
        """Compiles a tiny PE that writes `play-marker.txt` beside itself."""
        source = self.root / "play-marker.rs"
        write(
            source,
            "fn main() {\n"
            "    let exe = std::env::current_exe().expect(\"path\");\n"
            "    let marker = exe.parent().expect(\"dir\").join(\"play-marker.txt\");\n"
            "    std::fs::write(marker, b\"played\").expect(\"marker\");\n"
            "}\n",
        )
        target.parent.mkdir(parents=True, exist_ok=True)
        result = run(["rustc", "-o", target, source])
        require(result.returncode == 0, f"rustc failed: {result.stderr}")
        return target

    def remove(self):
        require(
            self.root.name.startswith("modn-desktop-fixture-")
            and self.root.parent == Path(tempfile.gettempdir()),
            "Unexpected cleanup root",
        )
        shutil.rmtree(self.root, ignore_errors=True)
        require(not self.root.exists(), f"Fixture directory was not removed: {self.root}")
        return True
