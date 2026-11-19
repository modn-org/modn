"""Disposable fixtures for the Modn Desktop verifier.

Every path lives under a fresh temporary directory. No real game, Steam
library or `%LOCALAPPDATA%\\modn` state is read or written.
"""

import ctypes
import hashlib
import json
import os
import shutil
import subprocess
import tempfile
import zipfile
from ctypes import wintypes
from pathlib import Path

from bucket_fixture import write_main_bucket

HIDDEN = subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0
SYSTEM32 = Path(os.environ.get("SystemRoot", r"C:\Windows")) / "System32"
# `--keep` sets this so cleanup keeps the fixture and prints where it is.
KEEP = False
# Evidence directories made in this process, in creation order. The runner
# reports the ones a journey created with its result.
CREATED = []

# Signed Microsoft binaries used as fixture game files. Nothing is compiled, so
# no unsigned executable exists for Defender to quarantine. `where.exe` is only
# a marker file that detection and removal journeys never start. `systeminfo.exe`
# is a console program, so the hidden console of `modn play` gives it no window.
# It reads system facts and stays alive for about three seconds with no
# arguments, null stdio and the small environment the app forwards, which the
# play journey proves by process image path. A copied GUI binary such as
# `mstsc.exe` has no language resource file and shows an error dialog.
MARKER_SOURCE = "where.exe"
PLAY_SOURCE = "systeminfo.exe"


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


def signed_copy(target, source):
    """Copies one signed Microsoft binary to `target` without compiling anything."""
    origin = SYSTEM32 / source
    require(origin.is_file(), f"Missing signed Microsoft binary {origin}")
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(origin, target)
    return target


def image_pids(image):
    """Process ids whose full image path equals `image`, read through Win32.

    The lookup takes milliseconds, so a poll every 50 ms sees a process that
    lives for one second. A CIM query takes more than one second and missed it.
    """
    kernel32 = ctypes.windll.kernel32
    ids = (wintypes.DWORD * 8192)()
    used = wintypes.DWORD()
    require(
        ctypes.windll.psapi.EnumProcesses(ids, ctypes.sizeof(ids), ctypes.byref(used)),
        "Process lookup failed: EnumProcesses",
    )
    wanted = os.path.normcase(os.path.realpath(str(image)))
    found = []
    for pid in ids[: used.value // ctypes.sizeof(wintypes.DWORD)]:
        handle = kernel32.OpenProcess(0x1000, False, pid)  # PROCESS_QUERY_LIMITED_INFORMATION
        if not handle:
            continue
        try:
            size = wintypes.DWORD(1024)
            path = ctypes.create_unicode_buffer(1024)
            if kernel32.QueryFullProcessImageNameW(handle, 0, path, ctypes.byref(size)):
                if os.path.normcase(os.path.realpath(path.value)) == wanted:
                    found.append(pid)
        finally:
            kernel32.CloseHandle(handle)
    return found


def stop_pid(pid):
    """Stops exactly one process id, never a name or image-path match.

    A process can exit between the lookup and the stop. Callers prove the
    outcome by looking the image path up again, so a missing pid is not an
    error here.
    """
    run(["taskkill", "/PID", str(pid), "/F"])


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
        self.clipboard_file = self.evidence / "clipboard-report.txt"
        for folder in (self.state, self.data, self.modules, self.archives, self.root / "appdata"):
            folder.mkdir(parents=True, exist_ok=True)
        self.evidence.mkdir(exist_ok=True)
        CREATED.append(self.evidence)

    # -- environment ----------------------------------------------------
    def environment(self):
        env = dict(os.environ)
        env["LOCALAPPDATA"] = str(self.data)
        env["MODN_CACHE"] = str(self.root / "cache")
        stores = self.root / "store-libraries.json"
        stores.write_text(json.dumps({"steam_roots": [str(self.steam)]}), encoding="utf-8")
        env["MODN_STORE_LIBRARIES"] = str(stores)
        env["MODN_APPDATA"] = str(self.root / "appdata")
        env["MODN_VERIFY_CLIPBOARD_FILE"] = str(self.clipboard_file)
        env.pop("MODN_GAME_MODULES_DIR", None)
        return env

    def main_bucket(self):
        """Writes the fixture's own bucket contract v2 `main` catalog."""
        target = self.data / "modn" / "buckets" / "main"
        return write_main_bucket(target)

    def steam_library(self, app_id, folder, marker, source=MARKER_SOURCE):
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
        signed_copy(game / marker, source)
        return game

    def register(self, cli, game_id, game, env=None):
        result = run(
            [cli, "--json", "--state-dir", self.state, "game", "add", game_id, "--path", game],
            env=env or self.environment(),
        )
        require(result.returncode == 0, f"game add failed: {result.stderr}")
        return result

    def zip(self, name, files, comment=None):
        path = self.archives / name
        with zipfile.ZipFile(path, "w") as archive:
            for inner, content in files.items():
                archive.writestr(inner, content)
            if comment is not None:
                archive.comment = comment.encode() if isinstance(comment, str) else comment
        return path

    def remove(self):
        if KEEP:
            print(f"Kept fixture {self.root}")
            print(f"Kept evidence {self.evidence}")
            return False
        require(
            self.root.name.startswith("modn-desktop-fixture-")
            and self.root.parent == Path(tempfile.gettempdir()),
            "Unexpected cleanup root",
        )
        shutil.rmtree(self.root, ignore_errors=True)
        require(not self.root.exists(), f"Fixture directory was not removed: {self.root}")
        return True
