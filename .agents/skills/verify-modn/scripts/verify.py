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
        completed = subprocess.run(command, cwd=REPO, capture_output=True, text=True,
                                   encoding="utf-8", errors="replace", timeout=90)
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
    game = root / "test-game"
    cyberpunk = root / "cyberpunk-fixture"
    game.joinpath("bin").mkdir(parents=True)
    game.joinpath("bin/TestGame.exe").write_bytes(b"fictional-game-marker")
    cyberpunk.joinpath("bin/x64").mkdir(parents=True)
    cyberpunk.joinpath("bin/x64/Cyberpunk2077.exe").write_bytes(b"disposable-marker")
    unmanaged = cyberpunk / "archive/pc/mod/unmanaged.archive"
    unmanaged.parent.mkdir(parents=True)
    unmanaged.write_bytes(UNMANAGED)
    test_zip = root / "fixture-package.zip"
    write_zip(test_zip, {"Demo/package/plugins/example.plugin": b"fixture-plugin"})
    cyber_zip = root / "legacy.zip"
    write_zip(cyber_zip, {"archive/pc/mod/managed.archive": PAYLOAD,
                          "README.md": b"documentation"})
    starting_game = snapshot(cyberpunk)
    starting_test = snapshot(game)

    driver.run("doctor-help", ["--help"])
    info = driver.json("module-check", ["module", "check", FIXTURE, "--allow-root", game])
    require(info["game_id"] == "test-game" and info["api_version"] == 1,
            "module check did not recognize test-game fixture")
    require(snapshot(game) == starting_test, "module check changed fake game")

    inspect_test = driver.json("inspect-test-archive", ["inspect", test_zip])
    require(inspect_test["format"] == "zip" and
            inspect_test["files"][0]["entry"] == "Demo/package/plugins/example.plugin",
            "inspect did not recognize test-game archive")
    driver.run("game-registration-not-yet-supported",
               ["--json", "--state-dir", root / "unsupported-state",
                "game", "add", "test-game", "--path", game], success=False)
    driver.json("test-game-install-not-yet-supported",
                ["install", test_zip, "--dry-run", "--path", game], success=False)
    require(snapshot(game) == starting_test, "unsupported game command modified fixture")

    report = driver.json("cyberpunk-dry-run",
                         ["install", cyber_zip, "--dry-run", "--path", cyberpunk])
    require(report["status"] == "reviewable" and report["metadata_only"],
            "valid legacy dry run is not reviewable")
    require([o["destination"] for o in report["operations"]] ==
            ["archive/pc/mod/managed.archive"], "unexpected dry-run mapping")
    require(snapshot(cyberpunk) == starting_game, "dry run changed a game file")
    require(not (root / "state").exists(), "explicit dry run initialized manager state")

    installed = driver.json("install-approved",
                            ["install", cyber_zip, "--yes", "--path", cyberpunk])
    package_id = installed["id"]
    managed = cyberpunk / "archive/pc/mod/managed.archive"
    require(managed.read_bytes() == PAYLOAD, "approved install did not write payload")
    require(unmanaged.read_bytes() == UNMANAGED, "unmanaged file modified")

    listed = driver.json("installed", ["installed", "--path", cyberpunk])
    require(any(item["id"] == package_id for item in listed),
            "installed inventory lacks newly owned package")
    scan = driver.json("scan-owned", ["scan", "--path", cyberpunk])
    require(any(item["relative_path"] == "archive/pc/mod/managed.archive"
                and item["managed"] for item in scan["mods"]), "scan missed owned file")
    require(any(item["relative_path"] == "archive/pc/mod/unmanaged.archive"
                and not item["managed"] for item in scan["mods"]),
            "scan adopted unmanaged file")
    driver.json("duplicate-install-blocked",
                ["install", cyber_zip, "--yes", "--path", cyberpunk], success=False)
    require(managed.read_bytes() == PAYLOAD and unmanaged.read_bytes() == UNMANAGED,
            "conflicting install changed existing data")

    managed.write_bytes(b"operator-modification")
    doctor = driver.json("doctor-drift", ["doctor", "--path", cyberpunk])
    require(any(item["code"] == "OWNED_FILE_MODIFIED"
                for item in doctor["diagnostics"]), "doctor missed ownership drift")
    driver.json("uninstall-modified-blocked",
                ["uninstall", package_id, "--yes", "--path", cyberpunk], success=False)
    require(managed.read_bytes() == b"operator-modification",
            "blocked uninstall deleted modified payload")
    managed.write_bytes(PAYLOAD)
    driver.json("uninstall-approved",
                ["uninstall", package_id, "--yes", "--path", cyberpunk])
    require(not managed.exists(), "uninstall left owned payload")
    require(unmanaged.read_bytes() == UNMANAGED, "uninstall removed unmanaged data")
    remaining = driver.json("installed-after-uninstall", ["installed", "--path", cyberpunk])
    require(remaining == [], "uninstall left managed package in ledger")
    recovery = driver.json("recover-idle", ["recover", "--yes"])
    require(recovery["recovered"] == 0, "idle recovery was unexpectedly nonempty")
    require(snapshot(game) == starting_test, "test-game fixture changed unexpectedly")
    return {
        "result": "PASS", "game_module": "test-game",
        "module_check": "verified via normal CLI with approved root",
        "test_game_install": "unsupported by current Cyberpunk-only CLI (expected)",
        "transaction_lifecycle": "verified with disposable Cyberpunk-compatible fixture",
        "package_id": package_id,
        "command_count": driver.index,
        "game_files_after": snapshot(cyberpunk),
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
