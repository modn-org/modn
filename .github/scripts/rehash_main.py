"""Rehash a contract v1 main bucket checkout into a contract v2 tree.

The script reads each v1 package record, acquires its archive, verifies the
recorded SHA-256, computes the BLAKE3 pin with `modn catalog hash`, and writes
a v2 record with its `sources` list. It stages the whole tree and renames it
into place only when every package succeeds.

Record rules and tree writing live in rehash_bucket.py; archive acquisition
stays here so tests can replace `download_archive` at module level.
"""

from __future__ import annotations

import argparse
import datetime
import hashlib
import os
import shutil
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request
from pathlib import Path
from typing import Sequence

from rehash_bucket import (
    STAGING_PREFIX,
    Converted,
    ProvenanceRow,
    RehashError,
    check_url,
    contained_archive,
    convert_record,
    copy_modules,
    download_url,
    is_digest,
    prepare_output,
    scan_bucket,
    source_file_name,
    write_json,
    write_provenance,
)

__all__ = ["DownloadError", "RehashError", "download_archive", "main"]

MAX_DOWNLOAD_BYTES = 4 * 1024 * 1024 * 1024
DOWNLOAD_TIMEOUT_SECONDS = 30


class DownloadError(RehashError):
    """A failed archive download."""


def download_archive(url: str, destination: Path) -> None:
    """Download one archive over HTTPS into a file, streaming with a size cap.

    Tests replace this module-level function; they never use the network.
    """
    check_url(url)
    request = urllib.request.Request(url, headers={"User-Agent": "modn-rehash-main/1"})
    try:
        with urllib.request.urlopen(request, timeout=DOWNLOAD_TIMEOUT_SECONDS) as response:
            check_url(response.geturl())
            length = response.headers.get("Content-Length")
            if length is not None and length.isdigit() and int(length) > MAX_DOWNLOAD_BYTES:
                raise DownloadError(f"download refused for {url}: the archive exceeds 4 GiB")
            written = 0
            with destination.open("wb") as handle:
                while True:
                    chunk = response.read(1024 * 1024)
                    if not chunk:
                        break
                    written += len(chunk)
                    if written > MAX_DOWNLOAD_BYTES:
                        raise DownloadError(f"download refused for {url}: the archive exceeds 4 GiB")
                    handle.write(chunk)
    except DownloadError:
        raise
    except (urllib.error.URLError, OSError, ValueError) as error:
        raise DownloadError(f"download failed for {url}: {error}") from error


def sha256_file(path: Path) -> str:
    """Return the SHA-256 of a file, streamed."""
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def resolve_modn(value: str | None) -> Path:
    """Resolve the Modn binary from `--modn`, the default `modn` name, or PATH."""
    candidate = value or "modn"
    path = Path(candidate)
    if path.is_file():
        return path.resolve()
    found = shutil.which(candidate)
    if found is not None:
        return Path(found).resolve()
    raise RehashError(
        f"modn binary not found: {candidate}; pass --modn with a build that has `modn catalog hash`"
    )


def blake3_pin(modn: Path, archive: Path) -> str:
    """Return the BLAKE3 pin that `modn catalog hash` prints."""
    try:
        completed = subprocess.run(
            [str(modn), "catalog", "hash", str(archive)],
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            check=False,
        )
    except OSError as error:
        raise RehashError(f"cannot run modn catalog hash: {error}") from error
    if completed.returncode != 0:
        detail = (completed.stderr.strip().splitlines() or ["no error output"])[-1]
        raise RehashError(f"modn catalog hash failed: {detail}")
    pin = completed.stdout.strip()
    if not is_digest(pin):
        raise RehashError(f"modn catalog hash printed an invalid BLAKE3 pin: {pin!r}")
    return pin


def acquire(
    converted: Converted,
    index: int,
    archive_dir: Path | None,
    staging: Path | None,
) -> Path:
    """Return the archive path for one record, reading or downloading it."""
    kind = converted.source["kind"]
    if kind in ("manual", "nexus"):
        if archive_dir is None:
            raise RehashError(
                f"{kind} source needs --archive-dir with {source_file_name(converted.source)}"
            )
        path = contained_archive(archive_dir, source_file_name(converted.source))
        if not path.is_file():
            raise RehashError(f"missing archive {path.name} in --archive-dir")
        return path
    if staging is None:
        raise RehashError("internal error: no staging directory for a download")
    suffix = ".zip" if converted.manifest.get("archive_format") == "zip" else ".bin"
    if converted.manifest.get("archive_format") == "7z":
        suffix = ".7z"
    destination = staging / f"archive-{index:03d}{suffix}"
    url = download_url(converted.source)
    try:
        download_archive(url, destination)
    except RehashError:
        raise
    except Exception as error:
        raise DownloadError(f"download failed for {url}: {error}") from error
    if not destination.is_file():
        raise DownloadError(f"download failed for {url}: no file was written")
    return destination


def rehash(
    bucket: Path,
    out: Path | None,
    archive_dir: Path | None,
    modn: Path | None,
    check: bool,
) -> int:
    """Run one conversion or pin check and return the process exit code."""
    if not bucket.is_dir():
        print(f"error: the v1 bucket directory does not exist: {bucket}", file=sys.stderr)
        return 1
    try:
        metadata, records, modules = scan_bucket(bucket)
        if not check:
            if out is None:
                raise RehashError("--out is required unless --check is used")
            prepare_output(out, bucket, archive_dir)
    except RehashError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1

    date = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%d")
    failures: list[str] = []
    rows: list[ProvenanceRow] = []
    count = 0
    staging_root: Path | None = None
    staging_archives: Path | None = None
    tree: Path | None = None
    try:
        if check:
            staging_archives = Path(tempfile.mkdtemp(prefix="modn-rehash-check-"))
        else:
            assert out is not None
            staging_root = Path(tempfile.mkdtemp(prefix=STAGING_PREFIX, dir=str(out.parent)))
            staging_archives = staging_root / "archives"
            tree = staging_root / "tree"
            staging_archives.mkdir()
            tree.mkdir()
            write_json(
                tree / "bucket.json",
                {
                    "schema_version": 2,
                    "name": metadata["name"],
                    "title": metadata["title"],
                },
            )

        for index, record in enumerate(records):
            try:
                converted = convert_record(record)
                pin = None
                if converted.sha256 is not None:
                    archive = acquire(converted, index, archive_dir, staging_archives)
                    actual = sha256_file(archive)
                    if actual != converted.sha256:
                        raise RehashError(
                            f"archive {archive.name} does not match its recorded sha256: "
                            f"expected {converted.sha256}, actual {actual}"
                        )
                    if not check:
                        assert modn is not None
                        pin = blake3_pin(modn, archive)
                count += 1
                rows.append(ProvenanceRow(record.label, converted.sha256, pin))
                if tree is not None:
                    manifest = dict(converted.manifest)
                    if pin is not None:
                        manifest["blake3"] = pin
                    write_json(tree / "mods" / record.game / record.filename, manifest)
            except RehashError as error:
                failures.append(f"{record.label}: {error}")
            except OSError as error:
                failures.append(f"{record.label}: cannot read archive: {error}")

        if failures:
            for failure in failures:
                print(f"failed: {failure}", file=sys.stderr)
            action = "Pin check" if check else "Rehash"
            print(
                f"{action} failed for {len(failures)} package(s); wrote nothing.",
                file=sys.stderr,
            )
            return 1

        if check:
            print(f"Verified {count} packages; every sha256 pin matches.")
            return 0

        assert out is not None and tree is not None and staging_root is not None
        copy_modules(modules, tree)
        write_provenance(bucket, tree, rows, date)
        assert staging_archives is not None
        shutil.rmtree(staging_archives)
        if out.exists():
            out.rmdir()
        os.rename(tree, out)
        print(f"Rehashed {count} packages to contract v2 in {out}.")
        return 0
    finally:
        if staging_root is not None:
            shutil.rmtree(staging_root, ignore_errors=True)
        elif staging_archives is not None:
            shutil.rmtree(staging_archives, ignore_errors=True)


def parse_args(argv: Sequence[str] | None) -> argparse.Namespace:
    """Parse the maintainer command line."""
    parser = argparse.ArgumentParser(
        description="Convert a contract v1 main bucket checkout into a contract v2 tree."
    )
    parser.add_argument("bucket", type=Path, help="the contract v1 bucket directory")
    parser.add_argument("--out", type=Path, help="the new contract v2 tree (missing or empty)")
    parser.add_argument("--archive-dir", type=Path, help="manual and nexus archives by file name")
    parser.add_argument("--modn", help="Modn binary with `catalog hash` (default: modn on PATH)")
    parser.add_argument("--check", action="store_true", help="verify pins and write nothing")
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    """Run the script as a maintainer command."""
    args = parse_args(argv)
    archive_dir: Path | None = args.archive_dir
    if archive_dir is not None and not archive_dir.is_dir():
        print(f"error: --archive-dir is not a directory: {archive_dir}", file=sys.stderr)
        return 1
    modn: Path | None = None
    if not args.check:
        try:
            modn = resolve_modn(args.modn)
        except RehashError as error:
            print(f"error: {error}", file=sys.stderr)
            return 1
    return rehash(args.bucket, args.out, archive_dir, modn, args.check)


if __name__ == "__main__":
    sys.exit(main())
