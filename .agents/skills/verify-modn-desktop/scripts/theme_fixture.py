"""Build disposable themes for the desktop `themes` journey (theme API v1).

A theme is a folder with `theme.json`, `theme.css` and one
`theme.<variant>.css` per listed variant, as [docs/themes.md] specifies.
Every fixture theme sets `min-height` on the stable hook
`.modn-theme-button`, so UI Automation bounding rectangles order the themes
by height even when DPI scaling changes absolute pixels.

[docs/themes.md]: ../../../../docs/themes.md
"""

import json
from pathlib import Path

from bucket_fixture import write_bucket, write_main_bucket

THEME_GAME = "balatro"
DEFAULT_KEY = "default"
HEIGHTS = {
    "fixture-official": 56,
    "fixture-official/bold": 72,
    "fixture-official/quiet": 64,
    "fixture-local": 80,
    "fixture-community": 88,
}


def _write_text(path, text):
    """Write UTF-8 text with LF line endings, as a theme requires."""
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8", newline="\n") as handle:
        handle.write(text)
    return path


def height_css(px):
    """Return a stylesheet that sets the theme button height in pixels."""
    return f".modn-theme-button {{ min-height: {px}px; }}\n"


def write_theme(folder, theme_id, name, base_css, variants=None, game=THEME_GAME):
    """Write one theme folder and return it.

    `variants` maps a variant id to its stylesheet. Each entry adds one
    `theme.<variant>.css` file and the variant id to `variants` in
    `theme.json`. The manifest holds only the fields `docs/themes.md` lists.
    """
    folder = Path(folder)
    variant_css = dict(variants or {})
    manifest = {
        "schema_version": 1,
        "id": theme_id,
        "game": game,
        "name": name,
        "theme_api": 1,
        "variants": list(variant_css),
    }
    _write_text(folder / "theme.json", json.dumps(manifest) + "\n")
    _write_text(folder / "theme.css", base_css)
    for variant, css in variant_css.items():
        _write_text(folder / f"theme.{variant}.css", css)
    return folder


def build_main_with_official(target):
    """Write the fixture `main` bucket with the official theme.

    The official theme has the `bold` and `quiet` variants. Returns the
    bucket root; the theme sits at `themes/balatro/fixture-official/`.
    """
    root = Path(write_main_bucket(target))
    write_theme(
        root / "themes" / THEME_GAME / "fixture-official",
        "fixture-official",
        "Fixture Official",
        height_css(HEIGHTS["fixture-official"]),
        variants={
            "bold": height_css(HEIGHTS["fixture-official/bold"]),
            "quiet": height_css(HEIGHTS["fixture-official/quiet"]),
        },
    )
    return root


def build_community_bucket(target):
    """Write the fixture `community` bucket with one valid and one broken theme.

    `fixture-broken` uses `position: fixed`, which is the rule id
    `forbidden-position-fixed`. Returns the bucket root.
    """
    root = write_bucket(target, "community", "Community")
    themes = root / "themes" / THEME_GAME
    write_theme(
        themes / "fixture-community",
        "fixture-community",
        "Fixture Community",
        height_css(HEIGHTS["fixture-community"]),
    )
    write_theme(
        themes / "fixture-broken",
        "fixture-broken",
        "Fixture Broken",
        ".modn-theme-button { position: fixed; }\n",
    )
    return root


def build_local_theme(target):
    """Write the local theme folder the journey registers, and return it."""
    return write_theme(
        Path(target) / "fixture-local",
        "fixture-local",
        "Fixture Local",
        height_css(HEIGHTS["fixture-local"]),
    )


def build_protected_theme(target):
    """Write the theme that restyles a protected region, and return it.

    The rule `.modn-approval { display: none; }` is `protected-region`.
    """
    return write_theme(
        Path(target) / "fixture-protected",
        "fixture-protected",
        "Fixture Protected",
        ".modn-approval { display: none; }\n",
    )


def expected_order(keys):
    """Return theme keys sorted by fixture height, shortest first.

    Raises `ValueError` for the default key, which has no fixture height, and
    for any key missing from `HEIGHTS`.
    """
    ordered = []
    for key in keys:
        if key == DEFAULT_KEY:
            raise ValueError("The default theme has no fixture height.")
        if key not in HEIGHTS:
            raise ValueError(f"Unknown theme key: {key}")
        ordered.append(key)
    return sorted(ordered, key=lambda key: HEIGHTS[key])


def taller(first_px, second_px, tolerance=1):
    """Return True when `first_px` exceeds `second_px` by more than `tolerance`."""
    return first_px > second_px + tolerance
