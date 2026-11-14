"""Contract v1 to v2 record conversion and tree writing for rehash_main.py.

This module holds the pure rules: record validation, source validation, the v2
manifest shape, the provenance section and the staged tree. It performs no
downloads and runs no subprocess.
"""

from __future__ import annotations

import json
import shutil
import urllib.parse
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Iterable, Sequence

SCHEMA_V1 = 1
SCHEMA_V2 = 2
MODULE_FILES = ("module.json", "module.lua")
STAGING_PREFIX = ".rehash-main-"
SOURCE_KINDS = ("manual", "https", "github-release", "git", "nexus")
ALLOWED_V1_FIELDS = {
    "schema_version",
    "id",
    "game",
    "name",
    "version",
    "archive_format",
    "sha256",
    "source",
    "dependencies",
    "homepage",
    "executables",
    "dependency_versions",
    "conflicts",
    "native_injector",
}
COPIED_FIELD_ORDER = (
    "dependencies",
    "homepage",
    "executables",
    "dependency_versions",
    "conflicts",
    "native_injector",
)
SOURCE_FIELDS = {
    "manual": {"kind", "filename"},
    "https": {"kind", "url"},
    "github-release": {"kind", "repository", "tag", "asset"},
    "git": {"kind", "repository", "commit"},
    "nexus": {"kind", "game_domain", "mod_id", "file_id", "file_name"},
}


class RehashError(Exception):
    """A package or bucket problem that stops the rehash without writes."""


class SourceRefused(RehashError):
    """A source that this maintainer tool will never download."""


@dataclass(frozen=True)
class Record:
    """One v1 record read from `games/<game>/<name>.json`."""

    game: str
    filename: str
    label: str
    value: dict[str, Any]


@dataclass(frozen=True)
class Converted:
    """A v2 manifest plus the archive data needed for the provenance table."""

    record: Record
    manifest: dict[str, Any]
    source: dict[str, Any]
    sha256: str | None


@dataclass(frozen=True)
class ProvenanceRow:
    """One row of the "Rehash to contract v2" provenance table."""

    label: str
    sha256: str | None
    blake3: str | None


def is_plain_name(value: Any) -> bool:
    """Return true for a single file name with no path part."""
    return (
        isinstance(value, str)
        and 0 < len(value) <= 255
        and value not in (".", "..")
        and not any(character in value for character in ("/", "\\"))
        and not any(ord(character) < 32 or 127 <= ord(character) <= 159 for character in value)
    )


def is_digest(value: Any) -> bool:
    """Return true for 64 lowercase hexadecimal characters."""
    return (
        isinstance(value, str)
        and len(value) == 64
        and all(character in "0123456789abcdef" for character in value)
    )


def valid_id(value: Any) -> bool:
    """Return true for a bucket identifier, mirroring the v2 path rule."""
    lowercase = "abcdefghijklmnopqrstuvwxyz"
    return (
        isinstance(value, str)
        and 0 < len(value) <= 64
        and value[0] in lowercase
        and all(character in lowercase or character.isdigit() or character == "-"
                for character in value)
        and not value.endswith("-")
    )


def check_url(url: str) -> str:
    """Refuse a URL that is not HTTPS or that names a modn-org host."""
    if any(character.isspace() for character in url):
        raise SourceRefused(f"refused source {url}: a URL must not contain whitespace")
    parsed = urllib.parse.urlsplit(url)
    scheme = parsed.scheme.lower()
    if scheme != "https":
        reason = "http is refused" if scheme == "http" else f"{scheme or 'no scheme'} is refused"
        raise SourceRefused(f"refused source {url}: only https downloads are allowed; {reason}")
    host = (parsed.hostname or "").lower()
    if not host:
        raise SourceRefused(f"refused source {url}: the URL has no host")
    if "modn-org" in host:
        raise SourceRefused(
            f"refused source {url}: modn-org hosts no package copies; list only the author's origin"
        )
    return url


def download_url(source: dict[str, Any]) -> str:
    """Return the HTTPS URL to download for an archive source."""
    kind = source.get("kind")
    if kind == "https":
        return check_url(str(source.get("url", "")))
    if kind == "github-release":
        repository = source.get("repository")
        tag = source.get("tag")
        asset = source.get("asset")
        if not isinstance(repository, str) or not isinstance(tag, str) or not is_plain_name(asset):
            raise RehashError("invalid github-release source")
        parts = repository.split("/")
        if len(parts) != 2 or not all(parts) or any(part in (".", "..") for part in parts):
            raise RehashError("invalid github-release repository")
        url = "https://github.com/{}/releases/download/{}/{}".format(
            repository,
            urllib.parse.quote(tag, safe=""),
            urllib.parse.quote(asset, safe=""),
        )
        return check_url(url)
    raise RehashError(f"source kind {kind!r} is not downloadable")


def source_file_name(source: dict[str, Any]) -> str:
    """Return the archive file name that a manual or nexus source names."""
    kind = source.get("kind")
    name = source.get("filename") if kind == "manual" else source.get("file_name")
    if not is_plain_name(name):
        raise RehashError(f"invalid {kind} archive file name")
    return name


def contained_archive(archive_dir: Path, name: str) -> Path:
    """Resolve an archive named by a record and refuse one outside `archive_dir`."""
    root = archive_dir.resolve()
    path = (archive_dir / name).resolve()
    if path.parent != root:
        raise RehashError(f"archive file name escapes --archive-dir: {name}")
    return path


def validate_source(source: Any) -> dict[str, Any]:
    """Validate one v1 source object and return it unchanged."""
    if not isinstance(source, dict):
        raise RehashError("source must be an object")
    kind = source.get("kind")
    if kind not in SOURCE_KINDS:
        raise RehashError(f"unsupported source kind {kind!r}")
    expected = SOURCE_FIELDS[kind]
    if set(source) != expected:
        raise RehashError(f"{kind} source must hold exactly {sorted(expected)}")
    if kind == "git":
        if not isinstance(source.get("repository"), str) or not source["repository"]:
            raise RehashError("invalid git repository")
        commit = source.get("commit")
        if not (isinstance(commit, str) and len(commit) == 40
                and all(character in "0123456789abcdef" for character in commit)):
            raise RehashError("git commit must be 40 lowercase hexadecimal characters")
    if kind in ("manual", "nexus"):
        source_file_name(source)
    if kind == "nexus":
        for field in ("mod_id", "file_id"):
            if not isinstance(source.get(field), int) or isinstance(source.get(field), bool):
                raise RehashError(f"nexus {field} must be an integer")
        if not isinstance(source.get("game_domain"), str) or not source["game_domain"]:
            raise RehashError("invalid nexus game domain")
    if kind in ("github-release", "https"):
        download_url(source)
    return source


def check_field_types(record: Record) -> None:
    """Refuse a v1 record whose copied fields have the wrong JSON type."""
    value = record.value
    for field in ("id", "game", "name", "version", "archive_format", "sha256"):
        if field in value and value[field] is not None and not isinstance(value[field], str):
            raise RehashError(f"{record.filename}: field {field} must be a string")
    if "dependencies" in value:
        dependencies = value["dependencies"]
        if not isinstance(dependencies, list) or not all(
            isinstance(item, str) for item in dependencies
        ):
            raise RehashError(
                f"{record.filename}: field dependencies must be a list of strings"
            )
    if "source" in value and not isinstance(value["source"], dict):
        raise RehashError(f"{record.filename}: field source must be an object")


def convert_record(record: Record) -> Converted:
    """Validate one v1 record and build its v2 manifest, without the archive."""
    value = record.value
    unknown = sorted(set(value) - ALLOWED_V1_FIELDS)
    if unknown:
        raise RehashError(f"invalid v1 fields: {', '.join(unknown)}")
    if value.get("schema_version") != SCHEMA_V1:
        raise RehashError(f"bucket package schema_version must be {SCHEMA_V1}")
    check_field_types(record)
    package_id = value.get("id")
    file_stem = record.filename[:-5]
    if not valid_id(package_id) or package_id != file_stem.split("@")[0]:
        raise RehashError("package id disagrees with its games/<game>/<file>.json name")
    if value.get("game") != record.game or not valid_id(record.game):
        raise RehashError("package game disagrees with its games/<game>/ directory")
    if not isinstance(value.get("name"), str) or not value["name"].strip():
        raise RehashError("invalid package name")
    version = value.get("version")
    if not isinstance(version, str) or not version or any(
            character in "/\\" or not 33 <= ord(character) <= 126 for character in version):
        raise RehashError("invalid package version")
    if "@" in record.label and record.label.split("@", 1)[1] != version:
        raise RehashError("release file name disagrees with its version")
    source = validate_source(value.get("source"))
    git_only = source["kind"] == "git"
    archive_format = value.get("archive_format")
    sha256 = value.get("sha256")
    if git_only:
        if sha256 is not None:
            raise RehashError("git source records must not declare sha256")
        if archive_format is not None and archive_format != "zip":
            raise RehashError("git source archive_format must be zip when present")
    else:
        if archive_format not in ("zip", "7z"):
            raise RehashError("archive_format must be zip or 7z for an archive source")
        if not is_digest(sha256):
            raise RehashError("sha256 must be 64 lowercase hexadecimal characters")
        if source["kind"] in ("manual", "nexus"):
            extension = ".zip" if archive_format == "zip" else ".7z"
            if not source_file_name(source).lower().endswith(extension):
                raise RehashError(f"archive file name must end with {extension}")
    manifest: dict[str, Any] = {
        "schema_version": SCHEMA_V2,
        "id": package_id,
        "game": record.game,
        "name": value["name"],
        "version": version,
    }
    if archive_format is not None:
        manifest["archive_format"] = archive_format
    if not git_only:
        manifest["blake3"] = ""
    manifest["sources"] = [source]
    manifest["redistributable"] = False
    if not git_only:
        manifest["upstream_sha256"] = sha256
    for field in COPIED_FIELD_ORDER:
        if field in value:
            manifest[field] = value[field]
    return Converted(record=record, manifest=manifest, source=source, sha256=sha256)


def scan_bucket(bucket: Path) -> tuple[dict[str, Any], list[Record], dict[str, list[Path]]]:
    """Read bucket.json, every v1 package record and every game module."""
    metadata_path = bucket / "bucket.json"
    try:
        metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise RehashError(f"cannot read {metadata_path}: {error}") from error
    if not isinstance(metadata, dict):
        raise RehashError("bucket.json must be an object")
    if metadata.get("schema_version") != SCHEMA_V1:
        raise RehashError(
            f"bucket.json schema_version must be {SCHEMA_V1}; this is not a contract v1 checkout"
        )
    for field in ("name", "title"):
        if not isinstance(metadata.get(field), str) or not metadata[field].strip():
            raise RehashError(f"bucket.json needs a non-empty {field}")
    records: list[Record] = []
    modules: dict[str, list[Path]] = {}
    games = bucket / "games"
    if not games.exists():
        return metadata, records, modules
    if not games.is_dir():
        raise RehashError("games must be a directory")
    for game_dir in sorted(games.iterdir(), key=lambda path: path.name):
        game = game_dir.name
        if not game_dir.is_dir() or not valid_id(game):
            raise RehashError(f"invalid game directory: {game_dir.name}")
        game_modules: list[Path] = []
        for entry in sorted(game_dir.iterdir(), key=lambda path: path.name):
            if not entry.is_file():
                raise RehashError(f"unsupported entry in games/{game}: {entry.name}")
            if entry.name in MODULE_FILES:
                game_modules.append(entry)
                continue
            if entry.suffix != ".json":
                raise RehashError(f"unsupported entry in games/{game}: {entry.name}")
            try:
                value = json.loads(entry.read_text(encoding="utf-8"))
            except (OSError, json.JSONDecodeError) as error:
                raise RehashError(f"cannot read {entry}: {error}") from error
            if not isinstance(value, dict):
                raise RehashError(f"{entry}: a package record must be an object")
            records.append(
                Record(game=game, filename=entry.name, label=f"{game}/{entry.stem}", value=value)
            )
        if game_modules:
            modules[game] = game_modules
    records.sort(key=lambda record: (record.game, record.filename))
    return metadata, records, modules


def write_json(path: Path, value: dict[str, Any]) -> None:
    """Write one deterministic JSON file."""
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def provenance_section(rows: Iterable[ProvenanceRow], date: str) -> str:
    """Return the appended "Rehash to contract v2" provenance section."""
    rows = list(rows)
    lines = [
        "## Rehash to contract v2",
        "",
        f"`rehash_main.py` converted {len(rows)} packages to bucket contract v2 on {date} (UTC).",
        "Each archive was verified against its old SHA-256 pin before `modn catalog hash`",
        "printed its BLAKE3 pin. Git-only records keep their commit pin and have no archive digest.",
        "",
        "| Package | Old SHA-256 | New BLAKE3 |",
        "| --- | --- | --- |",
    ]
    for row in rows:
        old = f"`{row.sha256}`" if row.sha256 else "git source"
        new = f"`{row.blake3}`" if row.blake3 else "git commit pin"
        lines.append(f"| `{row.label}` | {old} | {new} |")
    lines.append("")
    return "\n".join(lines)


def copy_modules(modules: dict[str, list[Path]], tree: Path) -> None:
    """Copy every game module byte for byte into the v2 tree."""
    for game, files in sorted(modules.items()):
        target = tree / "games" / game
        target.mkdir(parents=True, exist_ok=True)
        for source in sorted(files, key=lambda path: path.name):
            shutil.copyfile(source, target / source.name)


def write_provenance(bucket: Path, tree: Path, rows: Sequence[ProvenanceRow], date: str) -> None:
    """Copy the input provenance note and append the rehash section."""
    existing = bucket / "PROVENANCE.md"
    text = existing.read_text(encoding="utf-8") if existing.is_file() else ""
    if text and not text.endswith("\n"):
        text += "\n"
    if text:
        text += "\n"
    text += provenance_section(rows, date)
    (tree / "PROVENANCE.md").write_text(text, encoding="utf-8")


def prepare_output(out: Path, bucket: Path, archive_dir: Path | None = None) -> None:
    """Refuse an unsafe or non-empty output directory before any work."""
    resolved_out = out.resolve()
    blocked = [("the v1 bucket directory", bucket.resolve())]
    if archive_dir is not None:
        blocked.append(("the archive directory", archive_dir.resolve()))
    for label, directory in blocked:
        if resolved_out == directory or directory in resolved_out.parents:
            raise RehashError(f"--out must not be inside {label}")
        if resolved_out in directory.parents:
            raise RehashError(f"--out must not contain {label}")
    if not out.parent.is_dir():
        raise RehashError(f"--out parent directory does not exist: {out.parent}")
    if not out.exists():
        return
    if not out.is_dir():
        raise RehashError(f"--out exists and is not a directory: {out}")
    if any(out.iterdir()):
        raise RehashError(f"--out must be missing or empty; it already contains files: {out}")
