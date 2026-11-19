"""Write offline catalog releases into the fixture's contract v2 `main` bucket.

A release is one ZIP built from `files`, pinned by its BLAKE3 digest. The
bucket publishes it at `mods/<game>/<mod>@<version>.json`; the highest version
seen so far is also the current release at `mods/<game>/<mod>.json`. Calling
the helper again with a higher version therefore advertises an update.
"""

import json
import subprocess
from dataclasses import dataclass
from pathlib import Path

from fixtures import HIDDEN, require, write

URL_ROOT = "https://fixture.invalid"
BLAKE3_SCRIPT = (
    "import sys, blake3; "
    "sys.stdout.write(blake3.blake3(sys.stdin.buffer.read()).hexdigest())"
)


@dataclass(frozen=True)
class CatalogRelease:
    """One offline catalog release and the fixture paths it owns."""

    game: str
    mod_id: str
    version: str
    blake3: str
    url: str
    archive: Path
    manifest: Path


def blake3_hex(data):
    """Return the lowercase BLAKE3 digest of `data`."""
    try:
        import blake3
    except ImportError:
        result = subprocess.run(
            ["uv", "run", "--no-project", "--with", "blake3", "python", "-c", BLAKE3_SCRIPT],
            input=data,
            capture_output=True,
            timeout=120,
            creationflags=HIDDEN,
        )
        require(result.returncode == 0, f"blake3 failed: {result.stderr.decode(errors='replace')}")
        return result.stdout.decode().strip()
    return blake3.blake3(data).hexdigest()


def _version_key(version):
    """Order dotted numeric versions like the CLI does."""
    digits = version[1:] if version.startswith("v") else version
    parts = digits.split(".")
    require(
        parts and all(part.isdigit() for part in parts),
        f"Catalog fixture version is not dotted numeric: {version!r}",
    )
    return tuple(int(part) for part in parts)


def _newer(left, right):
    """True when dotted numeric version `left` is newer than `right`."""
    width = max(len(left), len(right))
    for index in range(width):
        a = left[index] if index < len(left) else 0
        b = right[index] if index < len(right) else 0
        if a != b:
            return a > b
    return False


def _records(folder, mod_id):
    """Read every already published version of `mod_id`, keyed by version."""
    records = {}
    paths = [folder / f"{mod_id}.json", *folder.glob(f"{mod_id}@*.json")]
    for path in paths:
        if path.is_file():
            record = json.loads(path.read_text(encoding="utf-8"))
            records[record["version"]] = record
    return records


def _write_releases(folder, mod_id, record):
    """Publish `record` and move the current-release file to the highest version."""
    records = _records(folder, mod_id)
    records[record["version"]] = record
    highest = next(iter(records))
    for version in records:
        if _newer(_version_key(version), _version_key(highest)):
            highest = version
    for path in [folder / f"{mod_id}.json", *folder.glob(f"{mod_id}@*.json")]:
        if path.is_file():
            path.unlink()
    for version, item in records.items():
        name = f"{mod_id}.json" if version == highest else f"{mod_id}@{version}.json"
        write(folder / name, json.dumps(item))
    return folder / f"{mod_id}.json"


def add_catalog_release(
    fixture,
    game,
    mod_id,
    version,
    files,
    native_injector=False,
    executables=(),
    name=None,
):
    """Publish one offline catalog release and seed its verified cache entry.

    `files` maps archive-relative paths to bytes or text. The release uses an
    `https` source so uninstall can restore it from the cache. Returns the
    release paths, digest and version the journeys need.
    """
    bucket = fixture.main_bucket()
    archive = fixture.zip(
        f"{mod_id}-{version}.zip", files, comment=f"{mod_id}@{version}"
    )
    payload = archive.read_bytes()
    digest = blake3_hex(payload)
    url = f"{URL_ROOT}/{mod_id}-{version}.zip"
    record = {
        "schema_version": 2,
        "id": mod_id,
        "game": game,
        "name": name or mod_id.replace("-", " ").title(),
        "version": version,
        "archive_format": "zip",
        "blake3": digest,
        "sources": [{"kind": "https", "url": url}],
        "native_injector": bool(native_injector),
    }
    if executables:
        record["executables"] = list(executables)
    folder = bucket / "mods" / game
    folder.mkdir(parents=True, exist_ok=True)
    manifest = _write_releases(folder, mod_id, record)
    cache = fixture.root / "cache"
    cache.mkdir(exist_ok=True)
    url_hash = blake3_hex(url.encode())[:7]
    (cache / f"{game}#{mod_id}#{version}#{url_hash}.zip").write_bytes(payload)
    return CatalogRelease(
        game=game,
        mod_id=mod_id,
        version=version,
        blake3=digest,
        url=url,
        archive=archive,
        manifest=manifest,
    )
