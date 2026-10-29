#!/usr/bin/env python3
"""Read-only local ZIP compatibility benchmark against a disposable game root.

The source archive is never copied, extracted, edited or executed. Reports only
metadata and CLI dry-run status. Keep user archive paths out of source control.
"""
import argparse
import collections
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import zipfile

REPO = Path(__file__).resolve().parents[4]
CET = "bin/x64/plugins/cyber_engine_tweaks/mods/"
RED4EXT = "red4ext/plugins/"
MAX_ZIP_ENTRIES = 2048
MAX_ZIP_INDEX_BYTES = 2 * 1024 * 1024


def zip_index_within_budget(archive):
    # zipfile constructs every entry on open; bound the directory first.
    with archive.open("rb") as source:
        source.seek(max(0, archive.stat().st_size - (22 + 65535)))
        tail = source.read()
    offset = tail.rfind(b"PK\x05\x06")
    if offset < 0 or offset + 22 > len(tail):
        raise zipfile.BadZipFile("Missing ZIP end-of-central-directory record")
    # A ZIP64 locator overrides these ordinary counts, even without sentinels.
    if offset >= 20 and tail[offset - 20:offset - 16] == b"PK\x06\x07":
        return False
    comment_size = int.from_bytes(tail[offset + 20:offset + 22], "little")
    if offset + 22 + comment_size != len(tail):
        raise zipfile.BadZipFile("Invalid ZIP end-of-central-directory comment")
    entries = int.from_bytes(tail[offset + 10:offset + 12], "little")
    index_bytes = int.from_bytes(tail[offset + 12:offset + 16], "little")
    return entries < 0xFFFF and entries <= MAX_ZIP_ENTRIES and index_bytes <= MAX_ZIP_INDEX_BYTES


def run(archive_dir, binary, max_archive_mib):
    archive_dir = archive_dir.expanduser().resolve(strict=True)
    max_bytes = max_archive_mib * 1024 * 1024
    reports = []
    with tempfile.TemporaryDirectory(prefix="modn-cp77-benchmark-") as scratch:
        root = Path(scratch)
        game = root / "fictional-cyberpunk"
        executable = game / "bin/x64/Cyberpunk2077.exe"
        executable.parent.mkdir(parents=True)
        executable.write_bytes(b"disposable-marker")
        state = root / "manager-state"
        for archive in sorted(archive_dir.glob("*"), key=lambda path: path.name.lower()):
            if not archive.is_file() or archive.suffix.lower() not in (".zip", ".rar", ".7z"):
                continue
            item = {"archive": archive.name, "bytes": archive.stat().st_size}
            reports.append(item)
            if archive.suffix.lower() != ".zip":
                item["result"] = "skipped-other-format"
                continue
            if archive.stat().st_size > max_bytes:
                item["result"] = "skipped-size-cap"
                continue
            try:
                if not zip_index_within_budget(archive):
                    item["result"] = "skipped-index-cap"
                    continue
                with zipfile.ZipFile(archive) as reader:
                    names = [entry.filename for entry in reader.infolist() if not entry.is_dir()]
                normalized = [name.replace("\\", "/").lower() for name in names]
                item["file_count"] = len(names)
                item["cet_files"] = sum(CET in name for name in normalized)
                item["red4ext_files"] = sum(RED4EXT in name for name in normalized)
                if item["cet_files"] == 0 and item["red4ext_files"] == 0:
                    item["result"] = "skipped-other-layout"
                    continue
                command = [
                    str(binary), "--json", "--state-dir", str(state),
                    "--game", "cyberpunk-2077", "install", str(archive),
                    "--dry-run", "--path", str(game),
                ]
                output = subprocess.run(
                    command, cwd=REPO, capture_output=True, text=True,
                    encoding="utf-8", errors="replace", timeout=90)
                if output.returncode != 0:
                    item["result"] = "inspection-error"
                    item["error"] = output.stderr.strip()[:300]
                else:
                    response = json.loads(output.stdout)
                    item["result"] = response["status"]
                    item["operations"] = len(response.get("operations", []))
                    item["blockers"] = dict(collections.Counter(
                        issue["code"] for issue in response.get("blockers", [])))
            except (OSError, ValueError, zipfile.BadZipFile, subprocess.TimeoutExpired) as error:
                item["result"] = "error"
                item["error"] = str(error)[:300]
            if state.exists() or sorted(path for path in game.rglob("*") if path.is_file()) != [executable]:
                raise AssertionError("dry-run changed fixture or initialized state")
    counts = dict(collections.Counter(item["result"] for item in reports))
    print(json.dumps({"archive_count": len(reports), "outcomes": counts,
                      "archive_results": reports}, indent=2, ensure_ascii=False))
    return 0 if not any(item["result"] == "error" for item in reports) else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archives_dir", type=Path, help="Local archive folder, read-only")
    parser.add_argument("--binary", type=Path,
                        default=REPO / "target/debug" / ("modn.exe" if os.name == "nt" else "modn"))
    parser.add_argument("--max-archive-mib", type=int, default=64)
    options = parser.parse_args()
    if not options.binary.is_file():
        parser.error("Build first with cargo build -p modn-cli --locked")
    if options.max_archive_mib < 1:
        parser.error("--max-archive-mib must be positive")
    return run(options.archives_dir, options.binary.resolve(), options.max_archive_mib)


if __name__ == "__main__":
    sys.exit(main())
