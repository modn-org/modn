"""Verify a portable release CLI with disposable Cyberpunk policy fixtures."""
import argparse
import ctypes
from contextlib import closing
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request
import zipfile

REPO = Path(__file__).resolve().parents[4]
GAME = "cyberpunk-2077"


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def snapshot(root):
    return {p.relative_to(root).as_posix(): sha(p)
            for p in root.rglob("*") if p.is_file()}


class Run:
    def __init__(self, binary):
        self.root = Path(tempfile.mkdtemp(prefix="modn-portable-fixture-"))
        self.evidence = Path(tempfile.mkdtemp(prefix="modn-portable-evidence-"))
        self.binary = self.root / "modn.exe"
        shutil.copy2(binary, self.binary)
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

    def cli(self, label, args, expected=0):
        command = [str(self.binary), "--json", "--state-dir", str(self.state),
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

    def manifest(self, folder, name, archive, version="1.0", dependencies=(), extra=None):
        value = dict(schema_version=1, id=name, game=GAME, name=name,
                     version=version, archive_format="zip", sha256=sha(archive),
                     source=dict(kind="manual", filename=archive.name),
                     dependencies=list(dependencies))
        value.update(extra or {})
        folder.mkdir(parents=True, exist_ok=True)
        (folder / f"{name}.json").write_text(json.dumps(value), encoding="utf-8")
        return value


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
    spec = importlib.util.spec_from_file_location("pack", REPO / ".github/scripts/pack-main-bucket.py")
    pack = importlib.util.module_from_spec(spec)
    sys.dont_write_bytecode = True
    spec.loader.exec_module(pack)
    bucket = r.root / "main.zip"
    pack.pack(REPO / "buckets/main", bucket)
    r.env["MODN_MAIN_BUCKET_SOURCE"] = str(bucket)
    r.save("environment.json", dict(head=subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=REPO, text=True).strip(),
        binary_sha256=sha(r.binary), bucket_sha256=sha(bucket), path=r.env["PATH"],
        fixture=str(r.root), evidence=str(r.evidence), static_crt=True))
    result = r.cli("bootstrap", ["bucket", "update"])
    r.check(result["bucket"] == "main" and result["changed"], "explicit deterministic main ZIP bootstrap")
    r.cli("search", ["search"])
    (r.game / "bin/x64").mkdir(parents=True)
    (r.game / "bin/x64/Cyberpunk2077.exe").write_bytes(b"fake executable marker")
    (r.game / "unrelated.bin").write_bytes(b"preserve unrelated bytes")
    baseline = snapshot(r.game)
    r.cli("register", ["game", "add", GAME, "--path", r.game])
    r.cli("doctor-clean", ["doctor", "--path", r.game])
    r.check(snapshot(r.game) == baseline, "registration and doctor preserve fake game")
    catalog = r.root / "catalog"
    catalog.mkdir()
    (catalog / "bucket.json").write_text(json.dumps(dict(schema_version=1, name="fixture", title="Synthetic policy stack")))
    folder = catalog / "games" / GAME
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
    archives = {}
    manifests = {}
    for name, files in payloads.items():
        archives[name] = r.archive(name, files)
        official = REPO / f"buckets/main/games/{GAME}/{name}.json"
        value = json.loads(official.read_text()) if official.exists() else {}
        deps = value.get("dependencies", ["equipment-ex", "cet"] if name == "top" else [])
        extra = {k: value[k] for k in ("executables", "dependency_versions") if k in value}
        manifests[name] = r.manifest(folder, name, archives[name], value.get("version", "1.0"), deps, extra)
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
    v1 = manifests["red4ext"]
    (folder / "red4ext@1.31.0.json").write_text(json.dumps(v1))
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
    r.check(new["archive_sha256"] == sha(v2) and new["id"] != base["id"], "new package records updated archive identity")
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
    source = dict(kind="github-release", repository="BurntSushi/ripgrep", tag="14.1.1",
                  asset="ripgrep-14.1.1-x86_64-pc-windows-msvc.zip")
    url = f'https://github.com/{source["repository"]}/releases/download/{source["tag"]}/{source["asset"]}'
    try:
        request = urllib.request.Request(url, headers={"User-Agent": "modn-portable-verification"})
        with urllib.request.urlopen(request, timeout=60) as response:
            content = response.read(16 * 1024 * 1024)
        pin = hashlib.sha256(content).hexdigest()
        independent = r.evidence / source["asset"]
        independent.write_bytes(content)
        placeholder = r.root / "provider-pin.zip"
        placeholder.write_bytes(content)
        r.manifest(folder, "provider-probe", placeholder, extra={"source": source})
        r.check(not list((r.root / "cache").glob("*.zip")), "provider starts with an empty archive cache")
        before_game, before_db = snapshot(r.game), r.db()
        result = r.cli("provider-live", ["install", "fixture/" + GAME + "/provider-probe",
                       "--dry-run", "--download", "--path", r.game])
        r.save("provider-result.json", result)
        cached = [p for p in (r.root / "cache").rglob("*") if p.is_file() and sha(p) == pin]
        r.check(result["plan"]["status"] == "blocked", "provider ZIP layout blocked after acquisition")
        r.check(len(cached) == 1, "production provider HTTPS acquired exact independently hashed ZIP into cache")
        r.check(snapshot(r.game) == before_game and r.db() == before_db, "provider download preview preserves game and manager state")
        r.save("provider-live.json", dict(source=source, sha256=pin, bytes=len(content),
                                        cache_paths=list(map(str, cached)), status=result.get("plan", {}).get("status")))
        r.results["provider"] = "live production metadata/HTTPS/cache verified; catalog ZIP compatibility separately blocked"
    except Exception as error:
        r.results["provider"] = f"UNVERIFIED: {type(error).__name__}: {error}"
        r.save("provider-blocker.json", dict(error=str(error), url=url))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    args = parser.parse_args()
    r = Run(args.binary.resolve())
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
