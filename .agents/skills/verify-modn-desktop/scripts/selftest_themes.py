"""Selftest checks for the theme fixtures and their pure helpers.

The checks write into a temporary directory only. Nothing here starts the
desktop, the CLI or any process.
"""

import json
import tempfile
from pathlib import Path

import theme_fixture

OFFICIAL_ORDER = [
    "fixture-official",
    "fixture-official/quiet",
    "fixture-official/bold",
    "fixture-local",
    "fixture-community",
]


def _manifest(folder):
    """Return the parsed `theme.json` of one theme folder."""
    return json.loads((Path(folder) / "theme.json").read_text(encoding="utf-8"))


def _has_manifest(folder, theme_id, variants):
    """Return True when the folder holds the manifest for this theme."""
    try:
        manifest = _manifest(folder)
    except (OSError, ValueError):
        return False
    return (
        manifest.get("schema_version") == 1
        and manifest.get("theme_api") == 1
        and manifest.get("id") == theme_id
        and manifest.get("game") == theme_fixture.THEME_GAME
        and isinstance(manifest.get("name"), str)
        and manifest["name"] != ""
        and manifest.get("variants") == variants
    )


def _css(folder, name="theme.css"):
    """Return the text of one stylesheet in a theme folder."""
    return (Path(folder) / name).read_text(encoding="utf-8")


def _raises_value_error(call):
    """Return True when `call` raises `ValueError`."""
    try:
        call()
    except ValueError:
        return True
    return False


def _manifest_name(bucket):
    """Return the `name` field of one bucket's `bucket.json`, or None."""
    try:
        return json.loads((Path(bucket) / "bucket.json").read_text(encoding="utf-8"))[
            "name"
        ]
    except (OSError, ValueError, KeyError):
        return None


def checks():
    """Returns one boolean per selftest decision."""
    results = {}
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary)
        main = theme_fixture.build_main_with_official(root / "main")
        community = theme_fixture.build_community_bucket(root / "community")
        local = theme_fixture.build_local_theme(root / "local")
        protected = theme_fixture.build_protected_theme(root / "protected")

        results["theme_buckets_carry_bucket_json"] = (
            _manifest_name(main) == "main"
            and _manifest_name(community) == "community"
        )

        official = main / "themes" / theme_fixture.THEME_GAME / "fixture-official"
        results["official_theme_manifest_matches_layout"] = _has_manifest(
            official, "fixture-official", ["bold", "quiet"]
        )
        results["official_theme_sheets_set_the_fixture_heights"] = (
            "56px" in _css(official)
            and "72px" in _css(official, "theme.bold.css")
            and "64px" in _css(official, "theme.quiet.css")
        )

        community_theme = (
            community / "themes" / theme_fixture.THEME_GAME / "fixture-community"
        )
        results["community_theme_manifest_matches_layout"] = (
            _has_manifest(community_theme, "fixture-community", [])
            and "88px" in _css(community_theme)
        )

        broken = community / "themes" / theme_fixture.THEME_GAME / "fixture-broken"
        results["broken_theme_uses_position_fixed"] = "position: fixed" in _css(broken)

        results["local_theme_manifest_matches_layout"] = _has_manifest(
            local, "fixture-local", []
        ) and "80px" in _css(local)

        results["protected_theme_hides_the_approval_region"] = (
            ".modn-approval" in _css(protected)
            and _has_manifest(protected, "fixture-protected", [])
        )

        heights = list(theme_fixture.HEIGHTS.values())
        results["theme_heights_are_distinct_and_tall_enough"] = (
            len(set(heights)) == len(heights) and min(heights) > 48
        )

        results["expected_order_sorts_by_height"] = (
            theme_fixture.expected_order(list(theme_fixture.HEIGHTS)) == OFFICIAL_ORDER
        )

        results["expected_order_rejects_missing_heights"] = (
            _raises_value_error(
                lambda: theme_fixture.expected_order([theme_fixture.DEFAULT_KEY])
            )
            and _raises_value_error(
                lambda: theme_fixture.expected_order(["fixture-about-to-break"])
            )
        )

        results["taller_compares_rectangle_heights"] = (
            theme_fixture.taller(80, 72)
            and not theme_fixture.taller(72, 72)
            and not theme_fixture.taller(72, 73)
        )
    return results
