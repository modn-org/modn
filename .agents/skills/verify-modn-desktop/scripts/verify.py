"""Drive only this run's native Modn Desktop window through agent-desktop."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import zipfile

REPO = Path(__file__).resolve().parents[4]
HIDDEN = subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def snapshot_bytes(folder):
    return {p.relative_to(folder).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in folder.rglob("*") if p.is_file()}


def nodes(tree):
    yield tree
    for child in tree.get("children", []):
        yield from nodes(child)


class Driver:
    def __init__(self, app, backend):
        self.app = app.resolve()
        self.backend = backend.resolve()
        self.root = Path(tempfile.mkdtemp(prefix="modn-desktop-fixture-"))
        self.evidence = Path(tempfile.mkdtemp(prefix="modn-desktop-evidence-"))
        self.process = None
        self.window = None
        self.index = 0
        self.assertions = 0
        self.exits = []
        self.cleanup_complete = False
        self.environment = dict(os.environ, LOCALAPPDATA=str(self.root / "data"),
                                MODN_GAME_MODULES_DIR=str(self.root / "modules"))

    def record(self, label, value):
        self.index += 1
        (self.evidence / f"{self.index:03d}-{label}.json").write_text(
            json.dumps(value, indent=2, ensure_ascii=False), encoding="utf-8")

    def check(self, condition, message):
        require(condition, message)
        self.assertions += 1

    def run(self, command, label, env=None, json_output=True, cwd=None):
        result = subprocess.run(list(map(str, command)), env=env, capture_output=True,
                                encoding="utf-8", errors="replace", timeout=90,
                                creationflags=HIDDEN, cwd=cwd)
        self.exits.append(result.returncode)
        self.record(label, dict(command=list(map(str, command)), exit=result.returncode,
                                stdout=result.stdout, stderr=result.stderr))
        require(result.returncode == 0, f"{label}: {result.stderr} {result.stdout}")
        return json.loads(result.stdout) if json_output else result.stdout

    def cli(self, *args):
        return self.run([self.backend, "--json", "--state-dir", self.root / "state", *args],
                        "independent-cli", self.environment)

    def desktop(self, *args):
        # Resolve the installed Node launcher directly. No shell parses UI values.
        launcher = shutil.which("agent-desktop")
        if launcher and Path(launcher).suffix.lower() == ".cmd":
            launcher = str(Path(launcher).parent / "node_modules/agent-desktop/bin/agent-desktop")
            command = [shutil.which("node"), launcher, *args]
        else:
            command = [launcher, *args]
        result = self.run(command, "desktop")
        require(result.get("ok"), f"Desktop tool refused: {result}")
        return result["data"]

    def tree(self):
        return self.desktop("snapshot", "--window-id", self.window, "--timeout-ms", "20000")["tree"]

    def find(self, role, name):
        found = [n for n in nodes(self.tree()) if n.get("role") == role and n.get("name") == name]
        require(len(found) == 1, f"Expected one {role}:{name}, got {len(found)}")
        return found[0]

    def click(self, name):
        node = self.find("button", name)
        require("disabled" not in node.get("states", []), f"Button disabled: {name}")
        self.desktop("click", node["ref_id"])

    def choose_game(self):
        item = next(n for n in nodes(self.tree()) if n.get("role") == "button" and n.get("name", "").startswith("test-game:"))
        self.desktop("click", item["ref_id"])
        self.wait("Owned packages refreshed")

    def native_control(self, window, role, native_id):
        result = self.desktop("find", "--window-id", window, "--role", role,
                              "--native-id", native_id, "--exact", "--timeout-ms", "20000")
        matches = result["matches"]
        require(len(matches) == 1, f"Expected one native {role}:{native_id}")
        return matches[0]

    def open_file_dialog(self):
        self.click("Browse...")
        deadline = time.monotonic() + 15
        dialog = None
        while time.monotonic() < deadline:
            windows = self.desktop("list-windows", "--app", "modn-desktop.exe")
            own = [w for w in windows if w["pid"] == self.process.pid and w["id"] != self.window]
            if own:
                require(len(own) == 1, "Expected one owned file dialog")
                dialog = own[0]["id"]
                break
        require(dialog, "Native file dialog did not open")
        return dialog

    def cancel_file_dialog(self):
        dialog = self.open_file_dialog()
        self.desktop("click", self.native_control(dialog, "button", "2")["ref_id"])
        self.wait("File selection cancelled.")

    def select_file_dialog(self, path):
        dialog = self.open_file_dialog()
        field = self.native_control(dialog, "textfield", "1148")
        self.desktop("set-value", field["ref_id"], str(path))
        confirm = self.native_control(dialog, "button", "1")
        self.desktop("click", confirm["ref_id"])
        self.wait("Archive selected")
        selected = self.find("textfield", "Absolute path to a local ZIP or 7z")["value"]
        self.check(Path(selected) == path, "Native picker selected a different archive")

    def archive(self, path):
        self.desktop("set-value", self.find("textfield", "Absolute path to a local ZIP or 7z")["ref_id"], str(path))

    def wait(self, text):
        deadline = time.monotonic() + 75
        while time.monotonic() < deadline:
            tree = self.tree()
            if any(text in (n.get("value", "") + n.get("name", "")) for n in nodes(tree)):
                return tree
            require(self.process.poll() is None, f"App exited {self.process.returncode}")
        raise AssertionError(f"Timed out waiting for {text}")

    def unchanged(self, game, state):
        self.check(snapshot_bytes(self.root / "game") == game, "Game bytes changed during a read/cancellation/refusal")
        self.check(snapshot_bytes(self.root / "state") == state, "Manager state changed during a read/cancellation/refusal")

    def prepare(self):
        require(os.name == "nt", "Native proof requires Windows")
        require(self.app.is_file() and self.backend.is_file(), "Build both binaries first")
        shutil.copytree(REPO / "tests/fixtures/test-game", self.root / "modules/test-game")
        (self.root / "game/bin").mkdir(parents=True)
        (self.root / "game/bin/TestGame.exe").write_bytes(b"fixture-only")
        (self.root / "game/addons").mkdir()
        (self.root / "game/addons/manual.plugin").write_bytes(b"preserve-manual")
        for name, entry in [("demo.zip", "package/plugins/demo.plugin"), ("blocked.zip", "../escape.exe"), ("unmapped.zip", "unknown.txt")]:
            with zipfile.ZipFile(self.root / name, "w") as archive:
                archive.writestr(entry, b"owned-fixture")
        payload = self.root / "seven-source/package/plugins/seven.plugin"
        payload.parent.mkdir(parents=True)
        payload.write_bytes(b"seven-owned-fixture")
        seven = shutil.which("7z")
        require(seven, "Install 7-Zip to create the genuine native 7z fixture")
        self.run([seven, "a", "-t7z", str(self.root / "demo.7z"), "package"],
                 "create-7z", json_output=False, cwd=self.root / "seven-source")
        self.check((self.root / "demo.7z").read_bytes().startswith(b"7z\xbc\xaf\x27\x1c"),
                   "Fixture is not a genuine 7z archive")
        self.cli("game", "add", "test-game", "--path", self.root / "game")
        (self.root / "game2/bin").mkdir(parents=True)
        (self.root / "game2/bin/TestGame.exe").write_bytes(b"fixture-only")
        self.cli("game", "add", "test-game", "--path", self.root / "game2")

    def launch(self):
        stderr = open(self.evidence / "app-stderr.txt", "w", encoding="utf-8")
        args = [self.app, "--backend", self.backend, "--state-dir", self.root / "state",
                "--data-dir", self.root / "data", "--module-dir", self.root / "modules",
                "--evidence-dir", self.evidence / "backend"]
        self.process = subprocess.Popen(list(map(str, args)), stdout=stderr, stderr=stderr,
                                        creationflags=HIDDEN)
        stderr.close()
        self.record("launch", dict(args=list(map(str, args)), pid=self.process.pid,
                                   app_sha256=hashlib.sha256(self.app.read_bytes()).hexdigest(),
                                   backend_sha256=hashlib.sha256(self.backend.read_bytes()).hexdigest()))
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            windows = self.desktop("list-windows", "--app", "modn-desktop.exe")
            own = [w for w in windows if w["pid"] == self.process.pid]
            if own:
                self.window = own[0]["id"]
                break
            require(self.process.poll() is None, "App exited before its window appeared")
        require(self.window, "No owned window")
        self.wait("Select a registered installation.")
        self.check(f"PID {self.process.pid}" in self.tree()["name"], "Window identity differs")

    def drive(self):
        self.choose_game()
        self.check(self.cli("list", "--path", self.root / "game") == [], "Fixture starts with owned packages")
        game = snapshot_bytes(self.root / "game")
        state = snapshot_bytes(self.root / "state")
        self.cancel_file_dialog()
        self.unchanged(game, state)
        self.select_file_dialog(self.root / "demo.zip")
        self.click("Inspect and preview")
        self.wait("Review ready.")
        self.check("addons/demo.plugin" in json.dumps(self.find("document", "Proposed files, blockers and results")), "Mapped destination missing")
        self.unchanged(game, state)
        self.cancel_file_dialog()
        self.check(Path(self.find("textfield", "Absolute path to a local ZIP or 7z")["value"]) == self.root / "demo.zip",
                   "Cancelling the picker changed the archive path")
        self.check("disabled" in self.find("button", "Install reviewed archive...").get("states", []),
                   "Cancelling the picker retained review authority")
        self.unchanged(game, state)
        self.click("Inspect and preview")
        self.wait("Review ready.")
        second = next(n for n in nodes(self.tree()) if n.get("role") == "button" and n.get("name", "").startswith("test-game:") and n["name"].endswith("game2"))
        self.desktop("click", second["ref_id"])
        self.wait("Owned packages refreshed")
        self.check("disabled" in self.find("button", "Install reviewed archive...").get("states", []), "Changed game retained review authority")
        self.choose_game()
        self.click("Inspect and preview")
        self.wait("Review ready.")
        self.unchanged(game, state)
        self.desktop("screenshot", str(self.evidence / "review.png"), "--window-id", self.window)
        self.click("Install reviewed archive...")
        self.click("Cancel review")
        self.wait("Cancelled.")
        self.unchanged(game, state)
        self.check("disabled" in self.find("button", "Install reviewed archive...").get("states", []), "Cancel retained write authority")
        self.click("Inspect and preview")
        self.wait("Review ready.")
        self.archive(self.root / "blocked.zip")
        self.check("disabled" in self.find("button", "Install reviewed archive...").get("states", []), "Changed selection retained write authority")
        self.click("Inspect and preview")
        self.wait("Action refused or failed.")
        self.unchanged(game, state)
        self.archive(self.root / "unmapped.zip")
        self.click("Inspect and preview")
        self.wait("Plan blocked.")
        self.check("disabled" in self.find("button", "Install reviewed archive...").get("states", []), "Blocked plan enabled installation")
        self.unchanged(game, state)
        self.archive(self.root / "missing.zip")
        self.click("Inspect and preview")
        self.wait("Action refused or failed.")
        self.unchanged(game, state)
        self.archive(self.root / "demo.zip")
        self.click("Inspect and preview")
        self.wait("Review ready.")
        self.click("Install reviewed archive...")
        with zipfile.ZipFile(self.root / "demo.zip", "w") as archive:
            archive.writestr("package/plugins/demo.plugin", b"changed-source")
        self.click("Confirm install")
        self.wait("Archive changed since review")
        self.unchanged(game, state)
        with zipfile.ZipFile(self.root / "demo.zip", "w") as archive:
            archive.writestr("package/plugins/demo.plugin", b"owned-fixture")
        self.click("Inspect and preview")
        self.wait("Review ready.")
        self.click("Install reviewed archive...")
        self.cli("game", "remove", "test-game", "--path", self.root / "game")
        removed_state = snapshot_bytes(self.root / "state")
        self.click("Confirm install")
        self.wait("Game registration changed")
        self.unchanged(game, removed_state)
        self.cli("game", "add", "test-game", "--path", self.root / "game")
        self.click("Inspect and preview")
        self.wait("Review ready.")
        self.click("Install reviewed archive...")
        self.click("Confirm install")
        self.wait("Change complete.")
        owned = self.root / "game/addons/demo.plugin"
        self.check(owned.read_bytes() == b"owned-fixture", "Installed bytes differ")
        packages = self.cli("list", "--path", self.root / "game")
        self.check(len(packages) == 1 and packages[0]["files"], "Independent package inventory is missing")
        package_id = packages[0]["id"]
        item = self.find("button", package_id)
        self.desktop("click", item["ref_id"])
        self.click("Uninstall selected package...")
        self.click("Cancel review")
        self.check(owned.read_bytes() == b"owned-fixture", "Cancelled uninstall removed bytes")
        self.desktop("click", self.find("button", package_id)["ref_id"])
        self.click("Uninstall selected package...")
        owned.write_bytes(b"external-change")
        drift_game = snapshot_bytes(self.root / "game")
        drift_state = snapshot_bytes(self.root / "state")
        self.click("Confirm uninstall")
        self.wait("Action refused or failed.")
        self.unchanged(drift_game, drift_state)
        self.check(owned.read_bytes() == b"external-change", "Strict uninstall removed modified bytes")
        owned.write_bytes(b"owned-fixture")
        self.click("Uninstall selected package...")
        self.click("Confirm uninstall")
        self.wait("Change complete.")
        self.check(not owned.exists(), "Strict uninstall retained owned file")
        self.check(self.cli("list", "--path", self.root / "game") == [], "Uninstall retained package")
        self.check(snapshot_bytes(self.root / "game") == game, "Uninstall altered manual content")
        self.seven_lifecycle(game)
        self.click("Doctor / diagnostics")
        self.wait("Diagnostics complete")
        self.desktop("screenshot", str(self.evidence / "final.png"), "--window-id", self.window)

    def seven_lifecycle(self, original_game):
        self.select_file_dialog(self.root / "demo.7z")
        game = snapshot_bytes(self.root / "game")
        state = snapshot_bytes(self.root / "state")
        self.click("Inspect and preview")
        self.wait("Review ready.")
        self.check("addons/seven.plugin" in json.dumps(self.find("document", "Proposed files, blockers and results")),
                   "7z preview omitted the intended destination")
        self.unchanged(game, state)
        self.desktop("screenshot", str(self.evidence / "seven-review.png"), "--window-id", self.window)
        self.click("Install reviewed archive...")
        self.click("Confirm install")
        self.wait("Change complete.")
        owned = self.root / "game/addons/seven.plugin"
        self.check(owned.read_bytes() == b"seven-owned-fixture", "7z installed bytes differ")
        packages = self.cli("list", "--path", self.root / "game")
        self.check(len(packages) == 1 and packages[0]["files"], "7z ownership missing")
        self.desktop("click", self.find("button", packages[0]["id"])["ref_id"])
        self.click("Uninstall selected package...")
        owned.write_bytes(b"seven-external-change")
        drift_game = snapshot_bytes(self.root / "game")
        drift_state = snapshot_bytes(self.root / "state")
        self.click("Confirm uninstall")
        self.wait("Action refused or failed.")
        self.unchanged(drift_game, drift_state)
        owned.write_bytes(b"seven-owned-fixture")
        self.click("Uninstall selected package...")
        self.click("Confirm uninstall")
        self.wait("Change complete.")
        self.check(not owned.exists(), "7z strict uninstall retained owned bytes")
        self.check(self.cli("list", "--path", self.root / "game") == [], "7z uninstall retained ownership")
        self.check(snapshot_bytes(self.root / "game") == original_game, "7z lifecycle changed manual content")

    def cleanup(self):
        if self.process and self.process.poll() is None:
            # The Popen handle identifies only the process this run created.
            windows = self.desktop("list-windows", "--app", "modn-desktop.exe")
            dialogs = [w for w in windows if w["pid"] == self.process.pid and w["id"] != self.window]
            for dialog in dialogs:
                cancel = self.native_control(dialog["id"], "button", "2")
                self.desktop("click", cancel["ref_id"])
            tree = self.tree()
            close = next(n for n in nodes(tree) if n.get("native_id", {}).get("value") == "Close")
            self.desktop("click", close["ref_id"])
            self.process.wait(timeout=75)
        require(self.root.name.startswith("modn-desktop-fixture-") and self.root.parent == Path(tempfile.gettempdir()), "Unexpected cleanup root")
        shutil.rmtree(self.root)
        self.cleanup_complete = True


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--app", type=Path, default=Path(os.environ.get("CARGO_TARGET_DIR", REPO / "target")) / "debug/modn-desktop.exe")
    parser.add_argument("--backend", type=Path, required=True)
    args = parser.parse_args()
    driver = Driver(args.app, args.backend)
    status = "failed"
    try:
        driver.prepare()
        driver.launch()
        driver.drive()
        status = "passed"
    finally:
        driver.cleanup()
        summary = dict(status=status, assertions=driver.assertions, native_commands=len(driver.exits),
                       command_nonzero=sum(code != 0 for code in driver.exits),
                       fixture_removed=not driver.root.exists(), pid=driver.process.pid if driver.process else None,
                       app_exit=driver.process.returncode if driver.process else None,
                       cleanup_complete=driver.cleanup_complete)
        driver.record("summary", summary)
        print(json.dumps(dict(evidence=str(driver.evidence), **summary)))
    require(status == "passed", "Native proof failed")


if __name__ == "__main__":
    main()
