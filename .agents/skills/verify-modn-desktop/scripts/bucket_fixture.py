"""Build the disposable bucket contract v2 `main` catalog for the verifier.

The fixture derives the catalog from files under `fixtures/`, never from the
repository's `buckets/main` submodule. A local copy below the fixture data
directory is already a trusted module source (`main`), so it needs no release
signature and no build key.

Layout follows `buckets/README.md`:

    bucket.json
    games/<game-id>/module.json
    games/<game-id>/module.lua
"""

import json
import shutil
from pathlib import Path

FIXTURES = Path(__file__).resolve().parent / "fixtures"
MODULES = {
    "balatro": FIXTURES / "balatro",
    "tools-game": FIXTURES / "tools-game",
}
# One catalog entry keeps the library tile and its Details route present. The
# journeys never install it, so the archive pin only needs a valid shape.
PACKAGES = (
    (
        "balatro",
        "fixture-pack.json",
        {
            "schema_version": 2,
            "id": "fixture-pack",
            "game": "balatro",
            "name": "Fixture Pack",
            "version": "1.0.0",
            "archive_format": "zip",
            "blake3": "0" * 64,
            "sources": [{"kind": "manual", "filename": "fixture-pack.zip"}],
        },
    ),
)


def write_bucket(root, name, title, modules=None, packages=None):
    """Create one contract v2 bucket and copy its game modules into place."""
    root = Path(root)
    root.mkdir(parents=True, exist_ok=True)
    (root / "bucket.json").write_text(
        json.dumps({"schema_version": 2, "name": name, "title": title}),
        encoding="utf-8",
    )
    for game, source in (modules or MODULES).items():
        source = Path(source)
        if not source.is_dir():
            raise AssertionError(f"Missing module fixture {source}")
        shutil.copytree(source, root / "games" / game, dirs_exist_ok=True)
    for game, file_name, record in (packages or PACKAGES):
        folder = root / "mods" / game
        folder.mkdir(parents=True, exist_ok=True)
        (folder / file_name).write_text(json.dumps(record), encoding="utf-8")
    return root


def write_main_bucket(target):
    """Write the fixture's own v2 `main` bucket and return its directory."""
    return write_bucket(target, "main", "Main")
