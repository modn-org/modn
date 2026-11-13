"""Exercise `modn history` and `modn undo` on a disposable standalone install.

The parent driver owns the real binary, the fixture game root and the evidence
directory. This module adds only the U1..U5 scenario so verify.py stays small.
"""
import hashlib
import json


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def snapshot(folder):
    return {
        path.relative_to(folder).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in folder.rglob("*") if path.is_file()
    }


def _history(driver, label):
    return driver.json(label, ["history", "--game", "test-game"])


def _refusal(driver, label, args):
    driver.run(label, ["--json", "--state-dir", driver.root / "state", *args],
               success=False)
    record = json.loads(
        (driver.evidence / f"{driver.index:02d}-{label}.json").read_text(encoding="utf-8"))
    try:
        return json.loads(record["stderr"])["error"]
    except (ValueError, KeyError) as error:
        raise AssertionError(
            f"{label}: expected a coded JSON error, got {record['stderr']!r}") from error


def verify_undo_history(driver, game, test_zip):
    """Run U1..U5 and return the independent check results for summary.json."""
    owned = game / "addons/example.plugin"
    unmanaged = game / "addons/manual.plugin"
    results = {}

    # U1: one standalone install writes one install record with its owned file.
    history_before = _history(driver, "undo-u1-history-before")
    game_before_install = snapshot(game)
    unmanaged_bytes = unmanaged.read_bytes()
    installed = driver.json("undo-u1-install",
                            ["install", test_zip, "--yes", "--path", game])
    require(owned.read_bytes() == b"fixture-plugin",
            "U1: standalone install did not write the game file")
    require(unmanaged.read_bytes() == unmanaged_bytes,
            "U1: install changed the unmanaged file")
    history = _history(driver, "undo-u1-history-after-install")
    require(len(history) == len(history_before) + 1,
            "U1: install did not add exactly one history record")
    record = history[0]
    require(record["kind"] == "install" and record["game_id"] == "test-game",
            "U1: newest record is not a test-game install")
    require("undoes" not in record,
            "U1: an ordinary install record must omit undoes")
    require(len(record["packages"]) == 1,
            "U1: install record must hold one package")
    package = record["packages"][0]
    require(package.get("package_id_after") == installed["id"],
            "U1: record does not name the installed package")
    require([file["destination"] for file in package["files"]] ==
            ["addons/example.plugin"],
            "U1: record does not hold the owned game file")
    require("digest_after" in package["files"][0] and
            "digest_before" not in package["files"][0],
            "U1: created file digests are wrong")
    listed = driver.json("undo-u1-list", ["list", "--path", game])
    require(any(item["id"] == installed["id"] for item in listed),
            "U1: inventory lost the installed package")
    install_id = record["id"]
    results["U1"] = "PASS"

    # U4 runs before U2. After U2 writes an undo record, the CLI refuses the
    # install ID with UNDO_CONFLICT, so the no-write guards need a clean target.
    game_guard = snapshot(game)
    history_guard = _history(driver, "undo-u4-history-before")
    code = _refusal(driver, "undo-u4-without-yes",
                    ["undo", install_id, "--path", game])
    require(code == "INVALID_ARGUMENTS",
            f"U4: missing --yes returned {code}")
    require(snapshot(game) == game_guard,
            "U4: missing --yes changed the game directory")
    require(_history(driver, "undo-u4-history-after-no-yes") == history_guard,
            "U4: missing --yes changed history")
    plan = driver.json("undo-u4-dry-run",
                       ["undo", install_id, "--dry-run", "--path", game])
    require(plan.get("undo") == install_id and plan.get("dry_run") is True,
            "U4: dry run did not describe the target")
    require(snapshot(game) == game_guard,
            "U4: dry run changed the game directory")
    require(_history(driver, "undo-u4-history-after-dry-run") == history_guard,
            "U4: dry run changed history")
    code = _refusal(driver, "undo-u4-unknown-id",
                    ["undo", "txn-unknown", "--yes", "--path", game])
    require(code == "UNDO_NOT_FOUND",
            f"U4: unknown ID returned {code}")
    require(snapshot(game) == game_guard,
            "U4: unknown ID changed the game directory")
    require(_history(driver, "undo-u4-history-after-unknown") == history_guard,
            "U4: unknown ID changed history")
    results["U4"] = "PASS"

    # U2: undo without an ID removes the newest install and records that undo.
    undone = driver.json("undo-u2-newest", ["undo", "--yes", "--path", game])
    require(undone.get("undo") == install_id and undone.get("kind") == "uninstall",
            "U2: undo did not reverse the install")
    require(installed["id"] in undone.get("removed", []),
            "U2: undo did not report removing the package")
    require(not owned.exists(), "U2: undo left the installed game file")
    require(snapshot(game) == game_before_install,
            "U2: game directory does not match the pre-install snapshot")
    require(driver.json("undo-u2-list", ["list", "--path", game]) == [],
            "U2: inventory still owns files")
    scan = driver.json("undo-u2-scan", ["scan", "--path", game])
    require(not any(mod["managed"] and
                    mod["relative_path"] == "addons/example.plugin"
                    for mod in scan["mods"]),
            "U2: scan still reports the installed file as managed")
    history = _history(driver, "undo-u2-history")
    require(len(history) == len(history_guard) + 1,
            "U2: undo did not add exactly one history record")
    require(history[0]["kind"] == "uninstall" and
            history[0].get("undoes") == install_id,
            "U2: newest record is not an uninstall of the install")
    require(history[1]["id"] == install_id,
            "U2: undo altered the target record")
    undo_id = history[0]["id"]
    results["U2"] = "PASS"

    # U3: a second undo of the same install is refused without changes.
    game_conflict = snapshot(game)
    history_conflict = _history(driver, "undo-u3-history-before")
    code = _refusal(driver, "undo-u3-second-undo",
                    ["undo", install_id, "--yes", "--path", game])
    require(code == "UNDO_CONFLICT",
            f"U3: second undo of the install returned {code}")
    require(snapshot(game) == game_conflict,
            "U3: refused second undo changed the game directory")
    require(_history(driver, "undo-u3-history-after") == history_conflict,
            "U3: refused second undo changed history")
    results["U3"] = "PASS"

    # U5: a manual-source uninstall cannot be restored, so undo refuses.
    reinstalled = driver.json("undo-u5-reinstall",
                              ["install", test_zip, "--yes", "--path", game])
    require(owned.read_bytes() == b"fixture-plugin",
            "U5: reinstall did not write the game file")
    driver.json("undo-u5-uninstall",
                ["uninstall", reinstalled["id"], "--yes", "--path", game])
    require(not owned.exists(), "U5: uninstall did not remove the game file")
    history_manual = _history(driver, "undo-u5-history-before")
    require(history_manual[0]["kind"] == "uninstall" and
            history_manual[0]["packages"][0].get("package_id_before") ==
            reinstalled["id"],
            "U5: newest record is not the uninstall of the reinstalled package")
    game_manual = snapshot(game)
    code = _refusal(driver, "undo-u5-manual-source",
                    ["undo", "--yes", "--path", game])
    require(code == "UNDO_SOURCE_MANUAL",
            f"U5: manual-source uninstall returned {code}")
    require(snapshot(game) == game_manual,
            "U5: refused manual-source undo changed the game directory")
    require(_history(driver, "undo-u5-history-after") == history_manual,
            "U5: refused manual-source undo changed history")
    results["U5"] = "PASS"

    require(len(results) == 5, "U1..U5 did not all run")
    return {
        "undo_history_checks": {key: results[key]
                                for key in ("U1", "U2", "U3", "U4", "U5")},
        "undo_history_passed": len(results),
        "undo_history_total": 5,
        "undo_install_id": install_id,
        "undo_undo_id": undo_id,
        "undo_history_records_final": len(history_manual),
    }
