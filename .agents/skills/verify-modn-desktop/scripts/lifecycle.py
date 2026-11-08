"""Check responsiveness and close refusal with an explicitly selected slow test backend."""
import argparse
import json
from pathlib import Path
import subprocess
import time
from verify import Driver, HIDDEN, nodes, require


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--app", type=Path, required=True)
    parser.add_argument("--backend", type=Path, required=True)
    args = parser.parse_args()
    driver = Driver(args.app, args.backend)
    state = driver.root / "state"
    state.mkdir()
    (driver.root / "modules").mkdir()
    (state / "game").mkdir()
    (state / "test-backend.marker").write_text("disposable")
    (state / "mode").write_text("slow")
    passed = False
    try:
        driver.launch()
        item = next(n for n in nodes(driver.tree()) if n.get("role") == "button" and n.get("name", "").startswith("fixture:"))
        driver.desktop("click", item["ref_id"])
        before = time.monotonic()
        tree = driver.tree()
        driver.check(time.monotonic() - before < 5, "Window did not answer while the backend was busy")
        cancel = next(n for n in nodes(tree) if n.get("name") == "Cancel review")
        driver.check("disabled" in cancel.get("states", []), "Active operation allowed cancellation")
        close = next(n for n in nodes(tree) if n.get("native_id", {}).get("value") == "Close")
        driver.desktop("click", close["ref_id"])
        driver.wait("Wait for the current operation before closing.")
        driver.check(driver.process.poll() is None, "Busy close abandoned the backend operation")
        driver.desktop("screenshot", str(driver.evidence / "busy-close.png"), "--window-id", driver.window)
        child = int((state / "child-pid").read_text())
        driver.wait("Owned packages refreshed.")
        require(not (state / "write.json").exists(), "Read fixture invoked a write")
        driver.check(True, "No write occurred")
        # tasklist writes the console code page, which is not UTF-8 on every locale.
        process_check = subprocess.run(["tasklist", "/fi", f"PID eq {child}", "/fo", "csv", "/nh"], capture_output=True, encoding="oem", errors="replace", creationflags=HIDDEN)
        driver.record("backend-reaped", dict(pid=child, exit=process_check.returncode, stdout=process_check.stdout))
        driver.check(process_check.returncode == 0 and f'"{child}"' not in process_check.stdout, "Backend child remained alive")
        (state / "mode").write_text("")
        source = driver.root / "source.zip"
        source.write_bytes(b"test backend accepts fixture metadata")
        driver.archive(source)
        driver.click("Inspect and preview")
        driver.wait("Review ready.")
        driver.click("Install reviewed archive...")
        (state / "mode").write_text("inventory-failure")
        driver.click("Confirm install")
        driver.wait("Write completed. Owned package refresh failed.")
        driver.check((state / "write.json").is_file(), "Completed write evidence was lost")
        driver.check("disabled" in driver.find("button", "Uninstall selected package...").get("states", []),
                     "Failed inventory retained an owned selection")
        driver.desktop("screenshot", str(driver.evidence / "completed-write-refresh-failed.png"), "--window-id", driver.window)
        passed = True
    finally:
        driver.cleanup()
        summary = dict(status="passed" if passed else "failed", assertions=driver.assertions,
                       native_commands=len(driver.exits), command_nonzero=sum(c != 0 for c in driver.exits),
                       app_exit=driver.process.returncode if driver.process else None,
                       fixture_removed=not driver.root.exists())
        driver.record("summary", summary)
        print(json.dumps(dict(evidence=str(driver.evidence), **summary)))
    require(passed, "Lifecycle proof failed")


if __name__ == "__main__":
    main()
