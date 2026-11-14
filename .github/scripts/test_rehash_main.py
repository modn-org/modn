"""Tests for .github/scripts/rehash_main.py.

Run with `python .github/scripts/test_rehash_main.py`. The tests build every
bucket and archive in a temporary directory. They never touch the network or
the real buckets/main content. A test that needs BLAKE3 skips when no built
Modn binary is available.
"""

from __future__ import annotations

import contextlib
import hashlib
import io
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent
REPO = SCRIPTS.parents[1]
sys.path.insert(0, str(SCRIPTS))

import rehash_main  # noqa: E402

GAME = "fixture-game"
COMMIT = "a" * 40
MANUAL_MODULE = {
    "game_id": GAME,
    "version": "1.0.0",
    "api_version": 1,
    "entrypoint": "module.lua",
}
MODULE_LUA = 'return { game_id = "fixture-game", api_version = 1 }\n'


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def snapshot(root: Path) -> dict[str, str]:
    """Map every file below a directory to its SHA-256."""
    return {
        path.relative_to(root).as_posix(): sha256(path)
        for path in sorted(root.rglob("*"))
        if path.is_file()
    }


def modn_binary() -> Path | None:
    """Find a built Modn binary through --modn, MODN_BIN or target/debug."""
    configured = os.environ.get("MODN_BIN")
    if configured:
        path = Path(configured)
        if not path.is_absolute():
            path = REPO / path
        return path if path.is_file() else None
    target = os.environ.get("CARGO_TARGET_DIR")
    base = Path(target) if target else REPO / "target"
    for name in ("modn", "modn.exe"):
        candidate = base / "debug" / name
        if candidate.is_file():
            return candidate
    return None


MODN = modn_binary()
NEEDS_MODN = unittest.skipUnless(
    MODN is not None,
    "no built Modn binary; set MODN_BIN or CARGO_TARGET_DIR, or build target/debug",
)


def make_archive(path: Path, marker: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(path, "w", zipfile.ZIP_STORED) as archive:
        archive.writestr("content.txt", marker)


class Fixture:
    """A disposable contract v1 bucket with five records and four archives."""

    def __init__(self, root: Path) -> None:
        self.root = root
        self.bucket = root / "main-v1"
        self.archives = root / "archives"
        self.bucket.mkdir()
        self.archives.mkdir()
        (self.bucket / "bucket.json").write_text(
            json.dumps({"schema_version": 1, "name": "fixture", "title": "Fixture Main"}),
            encoding="utf-8",
        )
        (self.bucket / "PROVENANCE.md").write_text(
            "# Provenance\n\nOld pin table.\n", encoding="utf-8"
        )
        games = self.bucket / "games" / GAME
        games.mkdir(parents=True)
        (games / "module.json").write_text(json.dumps(MANUAL_MODULE), encoding="utf-8")
        (games / "module.lua").write_text(MODULE_LUA, encoding="utf-8")
        self.routes: dict[str, Path] = {}
        self.expected: dict[str, dict] = {}
        self.archives_by_id: dict[str, Path] = {}
        self._build_records(games)

    def add_archive(self, name: str, marker: str) -> tuple[Path, str]:
        path = self.archives / name
        make_archive(path, marker)
        return path, sha256(path)

    def record(
        self,
        name: str,
        value: dict,
        archive: Path | None = None,
        runtime_sha: str | None = None,
    ) -> None:
        source = value["source"]
        if source["kind"] in ("https", "github-release"):
            url = rehash_main.download_url(source)
            self.routes[url] = archive
        if runtime_sha is not None:
            value = {**value, "sha256": runtime_sha}
        (self.bucket / "games" / GAME / f"{name}.json").write_text(
            json.dumps(value), encoding="utf-8"
        )
        self.expected[name] = value
        if archive is not None and runtime_sha is not None:
            self.archives_by_id[name] = archive

    def _build_records(self, games: Path) -> None:
        archive, pin = self.add_archive("https-mod.zip", "https archive")
        self.record(
            "https-mod",
            {
                "schema_version": 1,
                "id": "https-mod",
                "game": GAME,
                "name": "HTTPS Mod",
                "version": "1.0",
                "archive_format": "zip",
                "source": {"kind": "https", "url": "https://mods.example.org/https-mod.zip"},
                "dependencies": [],
                "homepage": "https://example.org/https-mod",
            },
            archive,
            pin,
        )
        archive, pin = self.add_archive("release-mod.zip", "release archive")
        self.record(
            "release-mod",
            {
                "schema_version": 1,
                "id": "release-mod",
                "game": GAME,
                "name": "Release Mod",
                "version": "1.0",
                "archive_format": "zip",
                "source": {
                    "kind": "github-release",
                    "repository": "example/release-mod",
                    "tag": "v1.0.0",
                    "asset": "release-mod.zip",
                },
                "native_injector": True,
            },
            archive,
            pin,
        )
        archive, pin = self.add_archive("Manual Mod 1.0.zip", "manual archive")
        self.record(
            "manual-mod",
            {
                "schema_version": 1,
                "id": "manual-mod",
                "game": GAME,
                "name": "Manual Mod",
                "version": "1.0",
                "archive_format": "zip",
                "source": {"kind": "manual", "filename": "Manual Mod 1.0.zip"},
                "dependencies": ["https-mod"],
                "dependency_versions": {"https-mod": ">=1.0"},
            },
            archive,
            pin,
        )
        archive, pin = self.add_archive("Manual Mod 0.9.zip", "manual older archive")
        self.record(
            "manual-mod@0.9",
            {
                "schema_version": 1,
                "id": "manual-mod",
                "game": GAME,
                "name": "Manual Mod",
                "version": "0.9",
                "archive_format": "zip",
                "source": {"kind": "manual", "filename": "Manual Mod 0.9.zip"},
                "dependencies": ["https-mod"],
            },
            archive,
            pin,
        )
        self.record(
            "git-mod",
            {
                "schema_version": 1,
                "id": "git-mod",
                "game": GAME,
                "name": "Git Mod",
                "version": "2.0",
                "source": {
                    "kind": "git",
                    "repository": "example/git-mod",
                    "commit": COMMIT,
                },
                "dependencies": ["manual-mod"],
                "homepage": "https://example.org/git-mod",
            },
        )

    def routes_for(self) -> dict[str, Path]:
        return dict(self.routes)


class RehashTestCase(unittest.TestCase):
    """Shared temporary directories, download substitution and assertions."""

    def setUp(self) -> None:
        self.root = Path(tempfile.mkdtemp(prefix="modn-rehash-test-"))
        self.addCleanup(shutil.rmtree, self.root, ignore_errors=True)
        self.fixture = Fixture(self.root)
        self.out = self.root / "main-v2"
        self.download_calls: list[str] = []
        self.fail_urls: set[str] = set()
        self.original_download = rehash_main.download_archive
        rehash_main.download_archive = self.fake_download
        self.addCleanup(self.restore_download)

    def restore_download(self) -> None:
        rehash_main.download_archive = self.original_download

    def fake_download(self, url: str, destination: Path) -> None:
        self.download_calls.append(url)
        if url in self.fail_urls:
            raise rehash_main.DownloadError(f"download failed for {url}: connection refused")
        route = self.fixture.routes_for().get(url)
        if route is None:
            raise AssertionError(f"unexpected download: {url}")
        shutil.copyfile(route, destination)

    def run_main(self, *arguments: object) -> tuple[int, str, str]:
        stdout, stderr = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            code = rehash_main.main([str(argument) for argument in arguments])
        return code, stdout.getvalue(), stderr.getvalue()

    def convert_arguments(self) -> list[object]:
        return [
            self.fixture.bucket,
            "--out",
            self.out,
            "--archive-dir",
            self.fixture.archives,
            "--modn",
            MODN if MODN is not None else "missing-modn",
        ]

    def assert_clean_failure(
        self,
        code: int,
        stderr: str,
        expected: list[str],
        before: dict[str, str],
    ) -> None:
        self.assertNotEqual(code, 0)
        for fragment in expected:
            self.assertIn(fragment, stderr)
        self.assertFalse(self.out.exists(), "a failed run must not create --out")
        self.assert_no_staging()
        self.assertEqual(before, snapshot(self.fixture.bucket))

    def assert_no_staging(self) -> None:
        leftovers = list(self.root.glob(rehash_main.STAGING_PREFIX + "*"))
        self.assertEqual(leftovers, [], "a failed run must remove its staging directory")

    def provenance_text(self) -> str:
        return (self.out / "PROVENANCE.md").read_text(encoding="utf-8")


class ConversionTests(RehashTestCase):
    @NEEDS_MODN
    def test_valid_v1_bucket_converts_to_the_expected_v2_tree(self) -> None:
        before = snapshot(self.fixture.bucket)
        code, stdout, stderr = self.run_main(*self.convert_arguments())
        self.assertEqual(code, 0, stderr)
        self.assertIn("Rehashed 5 packages", stdout)

        files = {
            path.relative_to(self.out).as_posix()
            for path in self.out.rglob("*")
            if path.is_file()
        }
        self.assertEqual(
            files,
            {
                "bucket.json",
                "PROVENANCE.md",
                "games/fixture-game/module.json",
                "games/fixture-game/module.lua",
                "mods/fixture-game/git-mod.json",
                "mods/fixture-game/https-mod.json",
                "mods/fixture-game/manual-mod.json",
                "mods/fixture-game/manual-mod@0.9.json",
                "mods/fixture-game/release-mod.json",
            },
        )
        metadata = json.loads((self.out / "bucket.json").read_text(encoding="utf-8"))
        self.assertEqual(
            metadata, {"schema_version": 2, "name": "fixture", "title": "Fixture Main"}
        )
        for module in ("module.json", "module.lua"):
            self.assertEqual(
                (self.out / "games" / GAME / module).read_bytes(),
                (self.fixture.bucket / "games" / GAME / module).read_bytes(),
            )

        for name in ("https-mod", "release-mod", "manual-mod", "manual-mod@0.9"):
            expected = self.fixture.expected[name]
            manifest = json.loads(
                (self.out / "mods" / GAME / f"{name}.json").read_text(encoding="utf-8")
            )
            pin = self.modn_hash(self.fixture.archives_by_id[name])
            self.assertEqual(
                manifest,
                {
                    "schema_version": 2,
                    "id": expected["id"],
                    "game": GAME,
                    "name": expected["name"],
                    "version": expected["version"],
                    "archive_format": "zip",
                    "blake3": pin,
                    "sources": [expected["source"]],
                    "redistributable": False,
                    "upstream_sha256": expected["sha256"],
                    **{
                        field: expected[field]
                        for field in (
                            "dependencies",
                            "homepage",
                            "dependency_versions",
                            "native_injector",
                        )
                        if field in expected
                    },
                },
                name,
            )
            self.assertEqual(len(pin), 64)

        git_manifest = json.loads(
            (self.out / "mods" / GAME / "git-mod.json").read_text(encoding="utf-8")
        )
        self.assertEqual(
            git_manifest,
            {
                "schema_version": 2,
                "id": "git-mod",
                "game": GAME,
                "name": "Git Mod",
                "version": "2.0",
                "sources": [
                    {"kind": "git", "repository": "example/git-mod", "commit": COMMIT}
                ],
                "redistributable": False,
                "dependencies": ["manual-mod"],
                "homepage": "https://example.org/git-mod",
            },
        )

        provenance = self.provenance_text()
        self.assertIn("## Rehash to contract v2", provenance)
        self.assertIn("converted 5 packages", provenance)
        self.assertRegex(provenance, r"on \d{4}-\d{2}-\d{2} \(UTC\)")
        for name in ("https-mod", "release-mod", "manual-mod", "manual-mod@0.9"):
            expected = self.fixture.expected[name]
            self.assertIn(f"`{GAME}/{name}`", provenance)
            self.assertIn(f"`{expected['sha256']}`", provenance)
        self.assertIn(f"`{GAME}/git-mod` | git source | git commit pin |", provenance)
        self.assertIn("Old pin table.", provenance)
        self.assertFalse(list(self.out.rglob("*.zip")))
        self.assertEqual(before, snapshot(self.fixture.bucket))
        self.assert_no_staging()

    def modn_hash(self, archive: Path) -> str:
        completed = subprocess.run(
            [str(MODN), "catalog", "hash", str(archive)],
            capture_output=True,
            text=True,
            encoding="utf-8",
            timeout=120,
            check=False,
        )
        self.assertEqual(completed.returncode, 0, completed.stderr)
        return completed.stdout.strip()

    @NEEDS_MODN
    def test_empty_out_directory_is_accepted(self) -> None:
        self.out.mkdir()
        code, _, stderr = self.run_main(*self.convert_arguments())
        self.assertEqual(code, 0, stderr)
        self.assertTrue((self.out / "mods" / GAME / "https-mod.json").is_file())
        self.assert_no_staging()

    @NEEDS_MODN
    def test_converted_tree_loads_as_a_local_bucket(self) -> None:
        code, _, stderr = self.run_main(*self.convert_arguments())
        self.assertEqual(code, 0, stderr)
        data = self.root / "data"
        official = data / "modn" / "buckets" / "main"
        official.mkdir(parents=True)
        (official / "bucket.json").write_text(
            json.dumps({"schema_version": 2, "name": "main", "title": "Fixture Main"}),
            encoding="utf-8",
        )
        environment = {
            key: value for key, value in os.environ.items() if not key.startswith("MODN_")
        }
        environment.update(
            LOCALAPPDATA=str(data),
            MODN_CACHE=str(self.root / "cache"),
            XDG_STATE_HOME=str(self.root / "xdg"),
        )
        state = self.root / "state"
        added = subprocess.run(
            [str(MODN), "--json", "--state-dir", str(state), "bucket", "add", "fixture",
             str(self.out)],
            capture_output=True,
            text=True,
            encoding="utf-8",
            timeout=180,
            env=environment,
            check=False,
        )
        self.assertEqual(added.returncode, 0, added.stderr)
        self.assertEqual(json.loads(added.stdout)["name"], "fixture")
        listed = subprocess.run(
            [str(MODN), "--json", "--state-dir", str(state), "bucket", "list"],
            capture_output=True,
            text=True,
            encoding="utf-8",
            timeout=180,
            env=environment,
            check=False,
        )
        self.assertEqual(listed.returncode, 0, listed.stderr)
        names = [entry["name"] for entry in json.loads(listed.stdout)]
        self.assertIn("fixture", names)
        shown = subprocess.run(
            [str(MODN), "--json", "--state-dir", str(state), "bucket", "show",
             f"fixture/{GAME}/manual-mod"],
            capture_output=True,
            text=True,
            encoding="utf-8",
            timeout=180,
            env=environment,
            check=False,
        )
        self.assertEqual(shown.returncode, 0, shown.stderr)
        self.assertEqual(json.loads(shown.stdout)["version"], "1.0")


class CheckTests(RehashTestCase):
    def test_check_writes_nothing_and_passes_when_every_pin_matches(self) -> None:
        before = snapshot(self.fixture.bucket)
        code, stdout, stderr = self.run_main(
            self.fixture.bucket, "--archive-dir", self.fixture.archives, "--check"
        )
        self.assertEqual(code, 0, stderr)
        self.assertIn("Verified 5 packages", stdout)
        self.assertFalse(self.out.exists())
        self.assertEqual(before, snapshot(self.fixture.bucket))
        self.assert_no_staging()

    def test_check_fails_on_a_tampered_archive_and_writes_nothing(self) -> None:
        tampered = self.fixture.archives / "Manual Mod 1.0.zip"
        with tampered.open("ab") as handle:
            handle.write(b"tampered")
        actual = sha256(tampered)
        before = snapshot(self.fixture.bucket)
        code, _, stderr = self.run_main(
            self.fixture.bucket, "--archive-dir", self.fixture.archives, "--check"
        )
        self.assert_clean_failure(
            code,
            stderr,
            [
                f"{GAME}/manual-mod",
                f"expected {self.fixture.expected['manual-mod']['sha256']}",
                f"actual {actual}",
            ],
            before,
        )


class FailureTests(RehashTestCase):
    @NEEDS_MODN
    def test_invalid_record_aborts_with_nothing_written(self) -> None:
        games = self.fixture.bucket / "games" / GAME
        (games / "aaa-invalid.json").write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "id": "aaa-invalid",
                    "game": GAME,
                    "name": "Invalid Record",
                    "version": "1.0",
                    "archive_format": "zip",
                    "source": {"kind": "manual", "filename": "aaa-invalid.zip"},
                }
            ),
            encoding="utf-8",
        )
        before = snapshot(self.fixture.bucket)
        code, _, stderr = self.run_main(*self.convert_arguments())
        self.assert_clean_failure(
            code,
            stderr,
            [f"{GAME}/aaa-invalid", "sha256 must be 64 lowercase hexadecimal characters"],
            before,
        )

    @NEEDS_MODN
    def test_tampered_archive_aborts_with_expected_and_actual_sha256(self) -> None:
        tampered = self.fixture.archives / "Manual Mod 1.0.zip"
        with tampered.open("ab") as handle:
            handle.write(b"tampered")
        actual = sha256(tampered)
        before = snapshot(self.fixture.bucket)
        code, _, stderr = self.run_main(*self.convert_arguments())
        self.assert_clean_failure(
            code,
            stderr,
            [
                f"{GAME}/manual-mod",
                f"expected {self.fixture.expected['manual-mod']['sha256']}",
                f"actual {actual}",
                "wrote nothing",
            ],
            before,
        )

    @NEEDS_MODN
    def test_missing_manual_archive_aborts(self) -> None:
        (self.fixture.archives / "Manual Mod 1.0.zip").unlink()
        before = snapshot(self.fixture.bucket)
        code, _, stderr = self.run_main(*self.convert_arguments())
        self.assert_clean_failure(
            code,
            stderr,
            [f"{GAME}/manual-mod", "missing archive Manual Mod 1.0.zip"],
            before,
        )

    @NEEDS_MODN
    def test_failing_download_aborts_and_names_the_package(self) -> None:
        self.fail_urls.add("https://mods.example.org/https-mod.zip")
        before = snapshot(self.fixture.bucket)
        code, _, stderr = self.run_main(*self.convert_arguments())
        self.assert_clean_failure(
            code,
            stderr,
            [f"{GAME}/https-mod", "download failed", "connection refused"],
            before,
        )

    def test_missing_modn_binary_aborts_before_any_download(self) -> None:
        before = snapshot(self.fixture.bucket)
        code, _, stderr = self.run_main(
            self.fixture.bucket,
            "--out",
            self.out,
            "--archive-dir",
            self.fixture.archives,
            "--modn",
            self.root / "missing-modn",
        )
        self.assert_clean_failure(code, stderr, ["modn binary not found"], before)
        self.assertEqual(self.download_calls, [])

    @NEEDS_MODN
    def test_http_source_is_refused(self) -> None:
        games = self.fixture.bucket / "games" / GAME
        (games / "bad-http.json").write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "id": "bad-http",
                    "game": GAME,
                    "name": "Bad HTTP",
                    "version": "1.0",
                    "archive_format": "zip",
                    "sha256": "0" * 64,
                    "source": {"kind": "https", "url": "http://example.org/bad-http.zip"},
                }
            ),
            encoding="utf-8",
        )
        before = snapshot(self.fixture.bucket)
        code, _, stderr = self.run_main(*self.convert_arguments())
        self.assert_clean_failure(
            code, stderr, [f"{GAME}/bad-http", "refused source", "http is refused"], before
        )

    @NEEDS_MODN
    def test_modn_org_host_is_refused(self) -> None:
        games = self.fixture.bucket / "games" / GAME
        (games / "bad-host.json").write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "id": "bad-host",
                    "game": GAME,
                    "name": "Bad Host",
                    "version": "1.0",
                    "archive_format": "zip",
                    "sha256": "0" * 64,
                    "source": {
                        "kind": "https",
                        "url": "https://modn-org.example.org/bad-host.zip",
                    },
                }
            ),
            encoding="utf-8",
        )
        before = snapshot(self.fixture.bucket)
        code, _, stderr = self.run_main(*self.convert_arguments())
        self.assert_clean_failure(
            code,
            stderr,
            [f"{GAME}/bad-host", "refused source", "modn-org hosts no package copies"],
            before,
        )

    @NEEDS_MODN
    def test_non_empty_out_is_refused_before_any_download(self) -> None:
        self.out.mkdir()
        marker = self.out / "keep.txt"
        marker.write_text("keep", encoding="utf-8")
        before = snapshot(self.fixture.bucket)
        code, _, stderr = self.run_main(*self.convert_arguments())
        self.assertNotEqual(code, 0)
        self.assertIn("must be missing or empty", stderr)
        self.assertEqual(self.download_calls, [])
        self.assertEqual(marker.read_text(encoding="utf-8"), "keep")
        self.assertEqual(before, snapshot(self.fixture.bucket))
        self.assert_no_staging()


class CheckCleanupTests(RehashTestCase):
    """`--check` downloads into a temporary directory and must remove it."""

    def check_staging(self) -> set[Path]:
        return set(Path(tempfile.gettempdir()).glob("modn-rehash-check-*"))

    def test_check_removes_its_staging_archives_on_success(self) -> None:
        before = self.check_staging()
        code, _, stderr = self.run_main(
            self.fixture.bucket, "--archive-dir", self.fixture.archives, "--check"
        )
        self.assertEqual(code, 0, stderr)
        self.assertEqual(
            before,
            self.check_staging(),
            "a successful --check left its downloaded archives behind",
        )

    def test_check_removes_its_staging_archives_on_failure(self) -> None:
        tampered = self.fixture.archives / "Manual Mod 1.0.zip"
        with tampered.open("ab") as handle:
            handle.write(b"tampered")
        before = self.check_staging()
        code, _, stderr = self.run_main(
            self.fixture.bucket, "--archive-dir", self.fixture.archives, "--check"
        )
        self.assertNotEqual(code, 0, stderr)
        self.assertEqual(
            before,
            self.check_staging(),
            "a failed --check left its downloaded archives behind",
        )


class OutputContainmentTests(RehashTestCase):
    """`--out` and archive names must resolve before they are compared."""

    def fake_modn(self) -> Path:
        path = self.root / "fake-modn"
        path.write_text("", encoding="utf-8")
        return path

    def test_out_inside_a_relative_bucket_alias_is_refused(self) -> None:
        previous = Path.cwd()
        os.chdir(self.root)
        try:
            code, _, stderr = self.run_main(
                "main-v1",
                "--out",
                self.fixture.bucket / "v2",
                "--modn",
                self.fake_modn(),
            )
        finally:
            os.chdir(previous)
        self.assertNotEqual(code, 0)
        self.assertIn("--out must not be inside the v1 bucket directory", stderr)
        self.assertFalse((self.fixture.bucket / "v2").exists())
        self.assertEqual(self.download_calls, [])

    def test_out_that_contains_the_bucket_is_refused(self) -> None:
        before = snapshot(self.fixture.bucket)
        code, _, stderr = self.run_main(
            self.fixture.bucket, "--out", self.root, "--modn", self.fake_modn()
        )
        self.assertNotEqual(code, 0)
        self.assertIn("--out must not contain the v1 bucket directory", stderr)
        self.assertEqual(before, snapshot(self.fixture.bucket))
        self.assertEqual(self.download_calls, [])

    def test_out_equal_to_the_archive_directory_is_refused(self) -> None:
        code, _, stderr = self.run_main(
            self.fixture.bucket,
            "--out",
            self.fixture.archives,
            "--archive-dir",
            self.fixture.archives,
            "--modn",
            self.fake_modn(),
        )
        self.assertNotEqual(code, 0)
        self.assertIn("--out must not be inside the archive directory", stderr)
        self.assertEqual(self.download_calls, [])

    @NEEDS_MODN
    def test_manual_archive_symlink_out_of_the_archive_dir_is_refused(self) -> None:
        outside = self.root / "outside.zip"
        make_archive(outside, "outside archive")
        link = self.fixture.archives / "linked.zip"
        try:
            os.symlink(outside, link)
        except (OSError, NotImplementedError) as error:
            self.skipTest(f"cannot create a symlink: {error}")
        self.fixture.record(
            "linked",
            {
                "schema_version": 1,
                "id": "linked",
                "game": GAME,
                "name": "Linked Mod",
                "version": "1.0",
                "archive_format": "zip",
                "sha256": sha256(outside),
                "source": {"kind": "manual", "filename": "linked.zip"},
            },
        )
        code, _, stderr = self.run_main(
            self.fixture.bucket,
            "--out",
            self.out,
            "--archive-dir",
            self.fixture.archives,
            "--modn",
            MODN,
        )
        self.assertNotEqual(code, 0)
        self.assertIn("archive file name escapes --archive-dir", stderr)
        self.assertFalse(self.out.exists())

    def test_manual_archive_parent_escape_is_refused(self) -> None:
        games = self.fixture.bucket / "games" / GAME
        (games / "escape.json").write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "id": "escape",
                    "game": GAME,
                    "name": "Escape Mod",
                    "version": "1.0",
                    "archive_format": "zip",
                    "sha256": "0" * 64,
                    "source": {"kind": "manual", "filename": "../escape.zip"},
                }
            ),
            encoding="utf-8",
        )
        code, _, stderr = self.run_main(
            self.fixture.bucket, "--out", self.out, "--archive-dir", self.fixture.archives,
            "--modn", self.fake_modn(),
        )
        self.assertNotEqual(code, 0)
        self.assertIn("invalid manual archive file name", stderr)

    def test_manual_archive_absolute_path_is_refused(self) -> None:
        games = self.fixture.bucket / "games" / GAME
        absolute = str(Path(self.root, "escape.zip").resolve())
        (games / "escape.json").write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "id": "escape",
                    "game": GAME,
                    "name": "Escape Mod",
                    "version": "1.0",
                    "archive_format": "zip",
                    "sha256": "0" * 64,
                    "source": {"kind": "manual", "filename": absolute},
                }
            ),
            encoding="utf-8",
        )
        code, _, stderr = self.run_main(
            self.fixture.bucket, "--out", self.out, "--archive-dir", self.fixture.archives,
            "--modn", self.fake_modn(),
        )
        self.assertNotEqual(code, 0)
        self.assertIn("invalid manual archive file name", stderr)


class FieldTypeTests(RehashTestCase):
    """Converted fields must hold the type that the v2 manifest requires."""

    @NEEDS_MODN
    def test_wrong_field_types_name_the_package_file_and_field(self) -> None:
        cases = [
            ("type-id", {"id": 7}, "field id must be a string"),
            ("type-game", {"game": 7}, "field game must be a string"),
            ("type-name", {"name": 7}, "field name must be a string"),
            ("type-version", {"version": 1}, "field version must be a string"),
            ("type-format", {"archive_format": 1}, "field archive_format must be a string"),
            ("type-sha", {"sha256": 7}, "field sha256 must be a string"),
            (
                "type-dependencies",
                {"dependencies": ["https-mod", 1]},
                "field dependencies must be a list of strings",
            ),
            ("type-source", {"source": "https://example.org/type.zip"}, "field source must be an object"),
        ]
        records = self.fixture.bucket / "games" / GAME
        for name, override, expected in cases:
            with self.subTest(field=name):
                value = {
                    "schema_version": 1,
                    "id": name,
                    "game": GAME,
                    "name": "Typed Mod",
                    "version": "1.0",
                    "archive_format": "zip",
                    "sha256": self.fixture.expected["https-mod"]["sha256"],
                    "source": {"kind": "manual", "filename": "Manual Mod 1.0.zip"},
                }
                value.update(override)
                path = records / f"{name}.json"
                path.write_text(json.dumps(value), encoding="utf-8")
                before = snapshot(self.fixture.bucket)
                code, _, stderr = self.run_main(
                    *self.convert_arguments(),
                )
                try:
                    self.assert_clean_failure(
                        code,
                        stderr,
                        [f"{GAME}/{name}", f"{name}.json", expected],
                        before,
                    )
                finally:
                    path.unlink()

if __name__ == "__main__":
    unittest.main()

