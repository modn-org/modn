"""Verify a portable release CLI with disposable Cyberpunk policy fixtures."""
import argparse
import ctypes
from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import socket
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
import zipfile

import bucket_fixture

REPO = Path(__file__).resolve().parents[4]
GAME = "cyberpunk-2077"


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def snapshot(root):
    return {p.relative_to(root).as_posix(): sha(p)
            for p in root.rglob("*") if p.is_file()}


class Run:
    def __init__(self, binary, bucket_binary):
        self.root = Path(tempfile.mkdtemp(prefix="modn-portable-fixture-"))
        self.evidence = Path(tempfile.mkdtemp(prefix="modn-portable-evidence-"))
        self.binary = self.root / "modn.exe"
        shutil.copy2(binary, self.binary)
        self.bucket_binary = self.root / "modn-bucket.exe"
        shutil.copy2(bucket_binary, self.bucket_binary)
        self.env = dict(os.environ)
        for key in list(self.env):
            if key.startswith("MODN_"):
                del self.env[key]
        self.env.update(LOCALAPPDATA=str(self.root / "data"),
                        MODN_CACHE=str(self.root / "cache"))
        self.env["PATH"] = os.pathsep.join(
            p for p in self.env.get("PATH", "").split(os.pathsep)
            if p and not any((Path(p) / name).exists()
                             for name in ("cargo.exe", "rustc.exe")))
        self.game = self.root / "game"
        self.state = self.root / "state"
        self.checks = []
        self.commands = []
        self.results = {}

    def save(self, name, value):
        (self.evidence / name).write_text(json.dumps(value, indent=2), encoding="utf-8")

    def check(self, condition, label):
        if not condition:
            raise AssertionError(label)
        self.checks.append(label)
        print(f"ok {len(self.checks)}: {label}", flush=True)

    def cli(self, label, args, expected=0, executable=None):
        command = [str(executable or self.binary), "--json", "--state-dir", str(self.state),
                   "--game", GAME, *map(str, args)]
        start = time.time()
        proc = subprocess.run(command, cwd=self.root, env=self.env,
                              capture_output=True, text=True, encoding="utf-8",
                              errors="replace", timeout=180)
        record = dict(label=label, command=command, cwd=str(self.root),
                      exit=proc.returncode, stdout=proc.stdout, stderr=proc.stderr,
                      seconds=time.time() - start)
        self.commands.append(record)
        self.save(f"{len(self.commands):02}-{label}.json", record)
        self.check(proc.returncode == expected, f"{label} exits {expected}")
        if proc.returncode != 0:
            return record
        try:
            return json.loads(proc.stdout)
        except json.JSONDecodeError:
            return proc.stdout

    def db(self):
        with closing(sqlite3.connect(self.state / "state.sqlite")) as db:
            tables = [r[0] for r in db.execute(
                "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")]
            return {name: db.execute(f'SELECT * FROM "{name}" ORDER BY rowid').fetchall()
                    for name in tables if not name.startswith("sqlite_")}

    def idle(self, label):
        state = self.db()
        self.check(not state["pending_transactions"], f"{label}: no pending transactions")
        self.check(not list(self.state.glob("txn-*")), f"{label}: no transaction staging")
        self.save(f"state-{label}.json", state)
        return state

    def archive(self, name, files):
        path = self.root / f"{name}.zip"
        with zipfile.ZipFile(path, "w", zipfile.ZIP_STORED) as z:
            for key, value in files.items():
                z.writestr(key, value)
        return path

    def pin(self, archive):
        pin = bucket_fixture.archive_pin(self.binary, archive, self.env)
        self.save(f"inspect-{archive.stem}.json",
                  dict(archive=str(archive), blake3=pin))
        return pin

    def manifest(self, folder, name, archive, version="1.0", dependencies=(), extra=None):
        value = dict(schema_version=2, id=name, game=GAME, name=name,
                     version=version, archive_format="zip",
                     blake3=self.pin(archive),
                     sources=[dict(kind="manual", filename=archive.name)],
                     dependencies=list(dependencies))
        value.update(extra or {})
        folder.mkdir(parents=True, exist_ok=True)
        (folder / f"{name}.json").write_text(json.dumps(value), encoding="utf-8")
        return value


def signed_main_bucket(r):
    """Build and sign a minimal contract v2 main bucket in the fixture root."""
    tree = r.root / "main-tree"
    bucket_fixture.write_bucket(tree, "main", "Disposable signed main bucket")
    bucket_fixture.copy_modules(
        tree, {GAME: bucket_fixture.FIXTURES / GAME})
    bucket = bucket_fixture.zip_tree(tree, r.root / "main.zip")
    signature = Path(f"{bucket}.minisig")
    public_key = r.root / "main.pub"
    key_id = bucket_fixture.sign_tree(tree, signature, public_key)
    r.check(re.fullmatch(r"[0-9A-F]{16}", key_id) is not None,
            f"ephemeral test key {key_id} signed the fixture main bucket")
    return bucket, public_key, key_id


def lifecycle(r):
    outside = subprocess.run(["git", "rev-parse", "--show-toplevel"], cwd=r.root,
                             capture_output=True, text=True)
    r.save("outside-repository.json", dict(command=["git", "rev-parse", "--show-toplevel"],
           cwd=str(r.root), exit=outside.returncode, stdout=outside.stdout, stderr=outside.stderr))
    r.check(outside.returncode == 128, "release binary runs outside a Git repository")
    r.check(not shutil.which("cargo", path=r.env["PATH"]) and
            not shutil.which("rustc", path=r.env["PATH"]), "Rust tools absent from child PATH")
    r.check(not re.search(rb"(?i)(vcruntime\d+(_\d+)?|msvcp\d+|api-ms-win-crt-[a-z-]+-l1-1-0)\.dll",
                          r.binary.read_bytes()), "no dynamic Visual C++ runtime imports")
    bucket, public_key, key_id = signed_main_bucket(r)
    r.env["MODN_MAIN_BUCKET_SOURCE"] = str(bucket)
    r.save("environment.json", dict(head=subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=REPO, text=True).strip(),
        binary_sha256=sha(r.binary), bucket_sha256=sha(bucket), bucket_key=key_id,
        path=r.env["PATH"], fixture=str(r.root), evidence=str(r.evidence), static_crt=True))
    r.env["MODN_TEST_MAIN_BUCKET_PUBLIC_KEY"] = public_key.read_text(encoding="utf-8").strip()
    try:
        result = r.cli("bootstrap", ["bucket", "update"], executable=r.bucket_binary)
    finally:
        del r.env["MODN_TEST_MAIN_BUCKET_PUBLIC_KEY"]
    r.check(result["bucket"] == "main" and result["changed"], "explicit deterministic main ZIP bootstrap")
    r.check(result["signed_by"] == key_id, "test-key build verified the ephemeral bucket signature")
    r.cli("search", ["search"])
    (r.game / "bin/x64").mkdir(parents=True)
    (r.game / "bin/x64/Cyberpunk2077.exe").write_bytes(b"fake executable marker")
    (r.game / "unrelated.bin").write_bytes(b"preserve unrelated bytes")
    baseline = snapshot(r.game)
    r.cli("register", ["game", "add", GAME, "--path", r.game])
    r.cli("doctor-clean", ["doctor", "--path", r.game])
    r.check(snapshot(r.game) == baseline, "registration and doctor preserve fake game")
    catalog = bucket_fixture.write_bucket(
        r.root / "catalog", "fixture", "Synthetic policy stack")
    folder = catalog / "mods" / GAME
    payloads = {
        "red4ext": {p: (p.encode() + b" synthetic\n") * 65536 for p in
                    ["bin/x64/winmm.dll", "red4ext/RED4ext.dll", "red4ext/LICENSE.txt", "red4ext/THIRD_PARTY_LICENSES.txt"]},
        "redscript": {p: b"synthetic compiler payload" for p in
                      ["engine/tools/scc.exe", "engine/tools/scc_lib.dll", "engine/config/base/scripts.ini", "r6/config/cybercmd/scc.toml"]},
        "cet": {p: b"synthetic CET payload" for p in
                ["bin/x64/version.dll", "bin/x64/plugins/cyber_engine_tweaks.asi", "bin/x64/global.ini", "bin/x64/plugins/cyber_engine_tweaks/scripts/json/json.lua", "bin/x64/plugins/cyber_engine_tweaks/tweakdb/tweakdbstr.kark"]},
        "archivexl": {"red4ext/plugins/ArchiveXL/ArchiveXL.dll": b"synthetic ArchiveXL"},
        "codeware": {"red4ext/plugins/Codeware/Codeware.dll": b"synthetic Codeware"},
        "tweakxl": {"red4ext/plugins/TweakXL/TweakXL.dll": b"synthetic TweakXL"},
        "equipment-ex": {"r6/scripts/EquipmentEx/main.reds": b"synthetic redscript",
                         "archive/pc/mod/EquipmentEx.archive": b"synthetic archive",
                         "archive/pc/mod/EquipmentEx.archive.xl": b"synthetic XL",
                         "r6/tweaks/EquipmentEx.yaml": b"synthetic tweak"},
        "top": {"archive/pc/mod/PortableTop.archive": b"synthetic top"},
    }
    versions = {
        "red4ext": "1.31.0", "redscript": "0.5.32", "cet": "1.38.1",
        "archivexl": "1.28.6", "codeware": "1.21.2", "tweakxl": "1.12.0",
        "equipment-ex": "1.2.10", "top": "1.0",
    }
    dependencies = {
        "red4ext": [], "redscript": ["red4ext"], "cet": ["red4ext"],
        "archivexl": ["red4ext", "redscript"],
        "codeware": ["red4ext", "redscript"],
        "tweakxl": ["red4ext", "redscript"],
        "equipment-ex": ["archivexl", "tweakxl", "codeware", "redscript"],
        "top": ["equipment-ex", "cet"],
    }
    framework_versions = {"red4ext": ">=1.31.0", "redscript": ">=0.5.32"}
    extras = {
        "redscript": {"executables": ["engine/tools/scc.exe"]},
        "archivexl": {"dependency_versions": framework_versions},
        "codeware": {"dependency_versions": framework_versions},
        "tweakxl": {"dependency_versions": framework_versions},
    }
    archives = {}
    manifests = {}
    for name, files in payloads.items():
        archives[name] = r.archive(name, files)
        manifests[name] = r.manifest(
            folder, name, archives[name], versions[name], dependencies[name],
            extras.get(name))
    r.cli("catalog-add", ["bucket", "add", "fixture", catalog])
    args = ["install", "fixture/" + GAME + "/top", "--path", r.game,
            "--allow-executable", "engine/tools/scc.exe"]
    supplied = [part for archive in archives.values() for part in ("--file", archive)]
    before = snapshot(r.game)
    preview = r.cli("stack-preview", [*args, "--dry-run", *supplied])
    r.save("stack-preview-independent.json", preview)
    r.check(snapshot(r.game) == before, "stack preview preserves game bytes")
    installed = r.cli("stack-install", [*args, "--yes", *supplied])
    r.check(len(installed["stack"]) == len(payloads), "eight-member prerequisite/dependency stack installed")
    for name, files in payloads.items():
        for path, expected in files.items():
            r.check((r.game / path).read_bytes() == expected, f"payload bytes: {path}")
    listed = r.cli("list-stack", ["list", "--path", r.game])
    r.check(len(listed) == 8, "eight managed packages recorded")
    state = r.idle("installed")
    expected_edges = sum(len(m["dependencies"]) for m in manifests.values())
    r.check(len(state["package_dependencies"]) == expected_edges and expected_edges > 0,
            f"{expected_edges} dependency edges persisted")
    r.cli("dependency-refusal", ["uninstall", "red4ext", "--yes", "--path", r.game], 1)
    base = next(p for p in listed if p["catalog"]["mod_id"] == "red4ext")
    previous = manifests["red4ext"]
    (folder / "red4ext@1.31.0.json").write_text(json.dumps(previous))
    changed = {key: b"version two\n" + value for key, value in payloads["red4ext"].items()}
    v2 = r.archive("red4ext-2", changed)
    r.manifest(folder, "red4ext", v2, "1.32.0")
    update = ["update", "fixture/" + GAME + "/red4ext", "--path", r.game, "--file", v2]
    stable = snapshot(r.game)
    stable_db = r.db()
    report = r.cli("update-preview", [*update, "--dry-run"])
    r.check(report["target"]["version"] == "1.32.0" and not report["blockers"], "update preview selects new release without blockers")
    r.check(snapshot(r.game) == stable and r.db() == stable_db, "update preview preserves files and database")
    r.cli("pin", ["pin", "red4ext", "--path", r.game])
    pinned_db = r.db()
    r.cli("pinned-update", [*update, "--yes"])
    r.check(snapshot(r.game) == stable and r.db() == pinned_db, "pin skips update with unchanged files and package/dependency state")
    r.cli("unpin", ["unpin", "red4ext", "--path", r.game])
    drift_path = r.game / "bin/x64/winmm.dll"
    original = drift_path.read_bytes()
    drift_path.write_bytes(b"fixture drift")
    drift_snapshot, drift_db = snapshot(r.game), r.db()
    rejected = r.cli("update-drift-preflight", [*update, "--yes"], 1)
    r.check("modified" in rejected["stderr"], "drift rejection identifies modified ownership")
    r.check(snapshot(r.game) == drift_snapshot and r.db() == drift_db,
            "preflight rejection preserves drifted files and package/dependency state")
    r.idle("preflight")
    drift_path.write_bytes(original)
    rollback(r, update, base, stable)
    r.cli("update-apply", [*update, "--yes"])
    for path, expected in changed.items():
        r.check((r.game / path).read_bytes() == expected, f"updated bytes: {path}")
    state = r.idle("updated")
    listed = r.cli("list-updated", ["list", "--path", r.game])
    new = next(p for p in listed if p["catalog"]["mod_id"] == "red4ext")
    r.check(new["archive_blake3"] == r.pin(v2) and new["id"] != base["id"], "new package records updated archive identity")
    r.check(len(state["package_dependencies"]) == expected_edges and
            all(row[1] != base["id"] for row in state["package_dependencies"]) and
            any(row[1] == new["id"] for row in state["package_dependencies"]),
            "all dependency edges preserved and repointed to successor")
    r.cli("doctor-updated", ["doctor", "--path", r.game])
    provider(r, folder)
    r.cli("cascade-preview", ["uninstall", "red4ext", "--cascade", "--dry-run", "--path", r.game])
    r.cli("cascade-uninstall", ["uninstall", "red4ext", "--cascade", "--yes", "--path", r.game])
    for package in r.cli("remaining", ["list", "--path", r.game]):
        r.cli("uninstall-remaining", ["uninstall", package["id"], "--yes", "--path", r.game])
    r.check(snapshot(r.game) == baseline, "managed cleanup preserves all unrelated original bytes")
    r.idle("clean")
    r.cli("recover-idle", ["recover", "--yes"])


def rollback(r, update, base, stable):
    files = [f["destination"] for f in base["files"]]
    r.check(len(files) >= 2 and not base["directories"], "rollback package has multiple shared-location targets")
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.CreateFileW.argtypes = [ctypes.c_wchar_p, ctypes.c_uint32, ctypes.c_uint32,
                                  ctypes.c_void_p, ctypes.c_uint32, ctypes.c_uint32, ctypes.c_void_p]
    kernel.CreateFileW.restype = ctypes.c_void_p
    kernel.CloseHandle.argtypes = [ctypes.c_void_p]
    target = r.game / files[-1]
    handle = kernel.CreateFileW(str(target), 0x80000000, 3, None, 3, 0, None)
    r.check(handle not in (None, ctypes.c_void_p(-1).value), "fixture deny-delete handle opened on later target")
    observed = []
    stop = threading.Event()
    first = r.game / files[0]
    def watch():
        while not stop.is_set():
            if not first.exists():
                observed.append(time.time_ns())
            time.sleep(0.0001)
    watcher = threading.Thread(target=watch)
    watcher.start()
    before = r.db()
    try:
        failed = r.cli("update-deny-delete", [*update, "--yes"], 1)
    finally:
        stop.set()
        watcher.join()
        kernel.CloseHandle(handle)
    r.save("rollback-witness.json", dict(order=files, locked=str(target),
           observed_earlier_absent=len(observed), samples_ns=observed[:50],
           restored_first_sha256=sha(first), command_exit=failed["exit"]))
    r.check(bool(observed), "earlier shared target removal observed before later failure")
    r.check("os error 5" in failed["stderr"] or "os error 32" in failed["stderr"],
            "failed update reports native Windows access/share refusal")
    r.check(snapshot(r.game) == stable, "failed update restores every original game byte")
    r.check(r.db() == before, "failed update preserves all package and dependency state")
    r.idle("rollback")
    r.results["rollback"] = "observed removal and restoration, not preflight rejection"


def provider(r, folder):
    """Prove an unreachable HTTPS source fails cleanly without network access."""
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        port = listener.getsockname()[1]
    source = dict(kind="https", url=f"https://127.0.0.1:{port}/provider-probe.zip")
    placeholder = r.archive("provider-pin", {"probe.txt": b"offline provider probe"})
    r.manifest(folder, "provider-probe", placeholder,
               extra={"sources": [source]})
    r.check(not list((r.root / "cache").glob("*.zip")),
            "provider starts with an empty archive cache")
    before_game, before_db = snapshot(r.game), r.db()
    result = r.cli("provider-transport", ["install", "fixture/" + GAME + "/provider-probe",
                   "--dry-run", "--download", "--path", r.game], 1)
    r.save("provider-transport.json", dict(source=source, exit=result["exit"],
                                           stderr=result["stderr"]))
    r.check("download failed" in result["stderr"],
            "unreachable HTTPS source reports a download failure")
    r.check(not list((r.root / "cache").glob("*.zip")),
            "failed transport leaves the archive cache empty")
    r.check(snapshot(r.game) == before_game and r.db() == before_db,
            "provider transport refusal preserves game and manager state")
    r.results["provider"] = (
        "offline HTTPS transport refusal preserved game, state and empty cache; "
        "live acquisition remains a manual release check")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path,
                        default=REPO / "target/release" /
                        ("modn.exe" if os.name == "nt" else "modn"))
    parser.add_argument("--bucket-binary", type=Path,
                        default=REPO / "target/smoke/release" /
                        ("modn.exe" if os.name == "nt" else "modn"),
                        help="test-main-key build used only for bucket update")
    args = parser.parse_args()
    for path in (args.binary, args.bucket_binary):
        if not path.is_file():
            parser.error(f"no executable at {path}")
    r = Run(args.binary.resolve(), args.bucket_binary.resolve())
    error = None
    try:
        lifecycle(r)
    except Exception as caught:
        error = f"{type(caught).__name__}: {caught}"
        raise
    finally:
        assert r.root.resolve().parent == Path(tempfile.gettempdir()).resolve()
        assert r.root.name.startswith("modn-portable-fixture-")
        shutil.rmtree(r.root)
        r.save("summary.json", dict(check_count=len(r.checks), checks=r.checks,
               command_count=len(r.commands), exits=[c["exit"] for c in r.commands],
               results=r.results, error=error, fixture_removed=not r.root.exists()))
        print(f"Evidence: {r.evidence}", flush=True)


if __name__ == "__main__":
    main()
