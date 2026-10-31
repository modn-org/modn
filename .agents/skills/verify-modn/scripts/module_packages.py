"""Unsigned local package lifecycle through the ordinary CLI, without rebuilding."""
import json
import zipfile


def verify_packages(driver, game, fixture):
    state = driver.root / "module-state"

    def call(label, args, *, success=True, override=False):
        output = driver.run("package-" + label,
                            ["--json", "--state-dir", state, *args],
                            success=success, use_override=override)
        return json.loads(output) if success else None

    def require(condition, message):
        if not condition:
            raise AssertionError(message)

    def files(root):
        return {p.relative_to(root).as_posix(): p.read_bytes()
                for p in root.rglob("*") if p.is_file()}

    def package(version, destination):
        manifest = json.loads((fixture / "module.json").read_text(encoding="utf-8"))
        manifest.update(package_format=1, version=version)
        source = (fixture / "main.lua").read_text(encoding="utf-8").replace(
            'destination = "addons/" .. suffix,',
            f'destination = "{destination}/" .. suffix,')
        path = driver.root / f"module-{version}.zip"
        with zipfile.ZipFile(path, "w", compression=zipfile.ZIP_DEFLATED) as archive:
            archive.writestr("module.json", json.dumps(manifest))
            archive.writestr("main.lua", source)
        return path

    a = package("1.0", "addons")
    b = package("2.0", "updated-addons")
    before = files(game)
    preview = driver.root / "module-preview.zip"
    with zipfile.ZipFile(preview, "w") as archive:
        archive.writestr("package/plugins/example.plugin", b"local fixture")

    def policy(label, destination):
        report = call(label, ["--game", "test-game", "install", preview,
                              "--dry-run", "--path", game])
        require(report["operations"][0]["destination"] == destination + "/example.plugin",
                "wrong installed Lua policy at " + label)

    require(call("empty-list", ["module", "list"]) == [], "unexpected module store")
    call("inspect", ["module", "inspect", a])
    call("dry-run", ["module", "install", a, "--dry-run"])
    call("requires-yes", ["module", "install", a], success=False)
    require(not state.exists(), "read-only package operations created state")
    info = call("install-a", ["module", "install", a, "--yes"])
    status = call("installed-inactive", ["module", "status", "test-game"])
    require(not status["enabled"], "installation silently enabled a module")
    call("enable-a", ["module", "enable", "test-game", "1.0", "--yes"])
    policy("runtime-a", "addons")
    call("update-b", ["module", "update", b, "--yes"])
    policy("runtime-b", "updated-addons")
    status = call("operator-override", ["module", "status", "test-game"], override=True)
    require(status["effective_source"]["origin"] == "override", "override priority changed")
    call("rollback-a", ["module", "rollback", "test-game", "--yes"])
    policy("runtime-rollback", "addons")
    require(len(call("retained-versions", ["module", "list"])) == 2, "update deleted a version")
    call("disable", ["module", "disable", "test-game", "--yes"])
    call("re-enable", ["module", "enable", "test-game", "1.0", "--yes"])
    database_before = files(state)
    conflicting = package("1.0", "conflicting")
    call("reject-version-replacement", ["module", "update", conflicting, "--yes"], success=False)
    require(files(state) == database_before, "failed update mutated the module store")
    script = state / "game-modules" / "versions" / info["sha256"] / "main.lua"
    original = script.read_bytes()
    script.write_bytes(b"error('tampered source executed')")
    call("reject-tampering", ["--game", "test-game", "install", preview,
                              "--dry-run", "--path", game], success=False)
    call("disable-for-repair", ["module", "disable", "test-game", "--yes"])
    script.write_bytes(original)
    call("enable-repaired", ["module", "enable", "test-game", "1.0", "--yes"])
    policy("runtime-repaired", "addons")
    require(files(game) == before, "module lifecycle changed game files")
    require(not (state / "state.sqlite").exists(), "module lifecycle initialized game state")
