#!/usr/bin/env python3
"""Exercise Modn's existing CLI against disposable fake installations."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import zipfile

REPO = Path(__file__).resolve().parents[4]
FIXTURE = REPO / "tests" / "fixtures" / "test-game"
PAYLOAD = b"fixture-owned-archive"
UNMANAGED = b"do-not-touch"


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def snapshot(folder):
    return {
        path.relative_to(folder).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in folder.rglob("*") if path.is_file()
    }


class Driver:
    def __init__(self, binary, evidence, root):
        self.binary = binary
        self.evidence = evidence
        self.root = root
        self.index = 0

    def run(self, label, args, *, success=True):
        self.index += 1
        command = [str(self.binary), *map(str, args)]
        completed = subprocess.run(
            command, cwd=REPO,
            env={**os.environ, "MODN_GAME_MODULES_DIR": str(self.root / "modules")},
            capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=90)
        record = {
            "label": label, "command": command, "exit_code": completed.returncode,
            "stdout": completed.stdout, "stderr": completed.stderr,
        }
        (self.evidence / f"{self.index:02d}-{label}.json").write_text(
            json.dumps(record, indent=2, ensure_ascii=False), encoding="utf-8")
        require((completed.returncode == 0) == success,
                f"{label}: unexpected exit {completed.returncode}; {completed.stderr}")
        return completed.stdout

    def json(self, label, args, *, success=True):
        result = self.run(label, ["--json", "--state-dir", self.root / "state", *args],
                          success=success)
        return json.loads(result) if success else None


def write_zip(path, members):
    with zipfile.ZipFile(path, "w", compression=zipfile.ZIP_STORED) as output:
        for name, payload in members.items():
            output.writestr(name, payload)


def verify(driver):
    root = driver.root
    modules = root / "modules"
    shutil.copytree(FIXTURE, modules / "test-game")
    shutil.copytree(REPO / "crates/modn-core/game-modules/cyberpunk-2077",
                    modules / "cyberpunk-2077")
    game = root / "test-game"
    cyberpunk = root / "cyberpunk-fixture"
    (game / "bin").mkdir(parents=True)
    (game / "bin/TestGame.exe").write_bytes(b"fixture-game")
    (cyberpunk / "bin/x64").mkdir(parents=True)
    (cyberpunk / "bin/x64/Cyberpunk2077.exe").write_bytes(b"fixture-cyberpunk")
    unmanaged = game / "addons/manual.plugin"
    unmanaged.parent.mkdir(parents=True)
    unmanaged.write_bytes(UNMANAGED)
    test_zip = root / "test.zip"
    write_zip(test_zip, {"Demo/package/plugins/example.plugin": b"fixture-plugin"})
    cyber_zip = root / "legacy.zip"
    write_zip(cyber_zip, {"archive/pc/mod/managed.archive": PAYLOAD})
    before = snapshot(game)

    driver.run("doctor-help", ["--help"])
    info = driver.json("module-check", ["module", "check", FIXTURE, "--allow-root", game])
    require(info["game_id"] == "test-game" and info["api_version"] == 1,
            "module identity is not test-game")
    driver.json("inspect-test-archive", ["inspect", test_zip])
    driver.json("no-default-game", ["install", test_zip, "--dry-run", "--path", game],
                success=False)
    require(not (root / "state").exists(), "unselected dry run created state")
    driver.run("state-inside-game-rejected",
               ["--state-dir", game / "private-state", "game", "add", "test-game",
                "--path", game], success=False)
    require(not (game / "private-state").exists(), "manager state entered game")
    driver.json("conflicting-game-ids-rejected",
                ["--game", "cyberpunk-2077", "game", "add", "test-game",
                 "--path", game], success=False)
    require(not (root / "state").exists(), "rejected registration created state")
    report = driver.json("test-game-dry-run",
                         ["--game", "test-game", "install", test_zip, "--dry-run",
                          "--path", game])
    require(report["status"] == "reviewable" and report["metadata_only"], str(report))
    require([(o["source"], o["destination"]) for o in report["operations"]] ==
            [("Demo/package/plugins/example.plugin", "addons/example.plugin")],
            "Luau mapping ignored")
    require(snapshot(game) == before, "preview modified game")
    require(not (root / "state").exists(), "preview created state")

    # A Luau-only change affects the next preview; there is no Rust rebuild.
    module_file = modules / "test-game/main.luau"
    original_policy = module_file.read_text(encoding="utf-8")
    changed_policy = original_policy.replace(
        'destination = "addons/" .. suffix,',
        'destination = "other-addons/" .. suffix,')
    require(changed_policy != original_policy, "fixture policy replacement was a no-op")
    module_file.write_text(changed_policy, encoding="utf-8")
    changed = driver.json("runtime-luau-remapping",
                          ["--game", "test-game", "install", test_zip, "--dry-run",
                           "--path", game])
    require(changed["status"] == "reviewable" and
            changed["operations"][0]["destination"] == "other-addons/example.plugin",
            "Luau changes require an unexpected Rust rebuild")
    module_file.write_text(original_policy, encoding="utf-8")
    require(snapshot(game) == before and not (root / "state").exists(),
            "runtime policy preview modified game or state")

    registered = driver.json("register-test-game",
                             ["game", "add", "test-game", "--path", game])
    require(registered["game_id"] == "test-game", "registration identity wrong")
    require(any(x["game_id"] == "test-game"
                for x in driver.json("list-games", ["game", "list"])),
            "registered game missing")
    installed = driver.json("install-test-game",
                            ["install", test_zip, "--yes", "--path", game])
    package_id = installed["id"]
    owned = game / "addons/example.plugin"
    require(owned.read_bytes() == b"fixture-plugin", "approved install failed")
    require(unmanaged.read_bytes() == UNMANAGED, "unmanaged file changed")
    require(any(x["id"] == package_id
                for x in driver.json("installed", ["installed", "--path", game])),
            "owned package missing")
    scan = driver.json("scan-test-game", ["scan", "--path", game])
    require(scan["game_id"] == "test-game", "wrong scan policy")
    require(any(x["relative_path"] == "addons/example.plugin" and x["managed"]
                and x["kind"] == "test-plugin" for x in scan["mods"]),
            "Luau scan ownership missing")
    require(any(x["relative_path"] == "addons/manual.plugin" and not x["managed"]
                for x in scan["mods"]), "unmanaged fixture adopted")
    driver.json("duplicate-install-blocked",
                ["install", test_zip, "--yes", "--path", game], success=False)
    require(owned.read_bytes() == b"fixture-plugin", "duplicate mutated owned file")

    owned.write_bytes(b"operator-modification")
    doctor = driver.json("doctor-drift", ["doctor", "--path", game])
    require(any(x["code"] == "OWNED_FILE_DRIFT" for x in doctor["diagnostics"]),
            "Luau doctor missed modified file")
    driver.json("modified-uninstall-blocked",
                ["uninstall", package_id, "--yes", "--path", game], success=False)
    require(owned.read_bytes() == b"operator-modification", "modified file deleted")
    owned.write_bytes(b"fixture-plugin")
    driver.json("uninstall-test-game", ["uninstall", package_id, "--yes", "--path", game])
    require(not owned.exists(), "uninstall did not remove owned file")
    require(unmanaged.read_bytes() == UNMANAGED, "uninstall modified unmanaged file")
    require(driver.json("installed-empty", ["installed", "--path", game]) == [],
            "ownership ledger not cleared")

    driver.json("register-cyberpunk",
                ["game", "add", "cyberpunk-2077", "--path", cyberpunk])
    second = driver.json("cyberpunk-dry-run",
                         ["--game", "cyberpunk-2077", "install", cyber_zip,
                          "--dry-run", "--path", cyberpunk])
    require(second["status"] == "reviewable" and
            second["operations"][0]["destination"] == "archive/pc/mod/managed.archive",
            "Cyberpunk policy regressed")
    driver.json("ambiguous-game-rejected", ["scan"], success=False)
    require(driver.json("recover-idle", ["recover", "--yes"])["recovered"] == 0,
            "unexpected recovery pending")
    return {
        "result": "PASS", "game_module": "test-game",
        "test_game_install": "full ordinary CLI lifecycle verified",
        "second_game": "Cyberpunk coexists in the same state",
        "command_count": driver.index, "package_id": package_id,
        "game_files_after": snapshot(game),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, help="Path to an existing modn CLI binary")
    parser.add_argument("--build", action="store_true", help="Run cargo build -p modn-cli --locked")
    args = parser.parse_args()
    evidence = Path(tempfile.mkdtemp(prefix="modn-verify-evidence-"))
    instance = Path(tempfile.mkdtemp(prefix="modn-verify-fixture-"))
    summary = {"result": "FAILED", "evidence": str(evidence), "fixture": str(instance)}
    try:
        if args.build:
            build = subprocess.run(["cargo", "build", "-p", "modn-cli", "--locked"],
                                   cwd=REPO, capture_output=True, text=True,
                                   encoding="utf-8", errors="replace", timeout=300)
            (evidence / "build.json").write_text(json.dumps({
                "exit_code": build.returncode, "stdout": build.stdout,
                "stderr": build.stderr}, indent=2), encoding="utf-8")
            require(build.returncode == 0, "cargo build failed; see build.json")
        binary = (args.binary.resolve() if args.binary else
                  REPO / "target" / "debug" / ("modn.exe" if os.name == "nt" else "modn"))
        require(binary.is_file(), f"CLI binary missing: {binary}; run with --build")
        require(FIXTURE.joinpath("module.json").is_file(), "missing test-game module fixture")
        summary.update(verify(Driver(binary, evidence, instance)))
        print("PASS: CLI verification completed; evidence:", evidence)
        return 0
    except (AssertionError, OSError, subprocess.SubprocessError, ValueError, KeyError) as error:
        summary["error"] = str(error)
        print(f"FAIL: {error}; evidence: {evidence}", file=sys.stderr)
        return 1
    finally:
        (evidence / "summary.json").write_text(
            json.dumps(summary, indent=2, ensure_ascii=False), encoding="utf-8")
        shutil.rmtree(instance)


if __name__ == "__main__":
    sys.exit(main())
