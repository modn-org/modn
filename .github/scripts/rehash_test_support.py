"""Shared fixtures for the rehash_main.py test modules.

The modules build every bucket and archive in a temporary directory.
They never touch the network or the real buckets/main content. A test
that needs BLAKE3 skips when no built Modn binary is available.
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

import rehash_bucket  # noqa: E402
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
