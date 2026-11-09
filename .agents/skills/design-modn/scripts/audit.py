"""List design values in a .figcraft file that no variable or style holds.

Usage: audit.py FILE.figcraft [--page NAME]

--page limits the check to one page.

Outside instances, checks each layer's visible fill and stroke paints, corner
radius, gap and padding, opacity, effects and text style. Inside instances,
reports each override that does not go through a component property: a
variant, text or boolean property leaves no override. A resized top-level
instance and a property set on an exposed nested instance are allowed. The read-only library
copies that a page may store are skipped. Prints one line per finding and a
count. Exits 1 when it finds one, 0 when there is none.
"""

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile

DEFAULT_CLI = r"C:\Program Files\ArtCraft\FigCraft\figcraft-cli.exe"
LAYOUT_MODES = ("horizontal", "vertical", "grid")
PADDING_SIDES = ("paddingTop", "paddingRight", "paddingBottom", "paddingLeft")
OVERRIDE_FIELDS = {"fills": "fill", "strokes": "stroke", "effects": "effects", "opacity": "opacity", "textStyle": "textStyle"}


def load(path):
    cli = shutil.which("figcraft-cli") or DEFAULT_CLI
    with tempfile.TemporaryDirectory() as scratch:
        out = os.path.join(scratch, "document.json")
        subprocess.run([cli, "convert", path, out], check=True, capture_output=True)
        with open(out, encoding="utf-8") as source:
            return json.load(source)


def raw_paints(node, kind, bound, styles):
    if styles.get(kind):
        return
    for index, paint in enumerate(node.get(f"{kind}s") or []):
        if not paint.get("visible", True):
            continue
        covered = f"{kind}s/{index}/color" in bound or (index == 0 and kind in bound)
        if not covered:
            yield kind if index == 0 else f"{kind}s/{index}", paint.get("color") or paint.get("type")


def raw_values(node):
    bound = node.get("bound") or {}
    styles = node.get("styles") or {}
    yield from raw_paints(node, "fill", bound, styles)
    yield from raw_paints(node, "stroke", bound, styles)
    radii = (node.get("corners") or {}).get("radii") or []
    if any(radii) and "cornerRadius" not in bound:
        yield "cornerRadius", radii
    if node.get("opacity", 1) != 1 and "opacity" not in bound:
        yield "opacity", node["opacity"]
    if node.get("effects") and not styles.get("effect") and not any(key.startswith("effects") for key in bound):
        yield "effects", [effect.get("type") for effect in node["effects"]]
    if node["type"] == "text" and not styles.get("text"):
        yield "textStyle", None
    layout = node.get("layout") or {}
    if layout.get("mode") in LAYOUT_MODES:
        values = {"gap": layout.get("gap", 0)}
        if layout.get("wrap"):
            values["crossGap"] = layout.get("cross_gap", 0)
        values.update(zip(PADDING_SIDES, layout.get("padding") or [0, 0, 0, 0]))
        for field, value in values.items():
            if value and field not in bound:
                yield field, value


def overrides(node, reported, top_level):
    allowed = {"name"}
    if top_level:
        allowed.add("size")
    if node.get("exposed"):
        allowed.add("props")
    for key in node.get("overrides") or []:
        if key not in allowed and OVERRIDE_FIELDS.get(key) not in reported:
            yield "override", key


def main():
    parser = argparse.ArgumentParser(usage=__doc__)
    parser.add_argument("file")
    parser.add_argument("--page")
    options = parser.parse_args()
    sys.stdout.reconfigure(encoding="utf-8")
    document = load(options.file)
    listed = document["nodes"]
    nodes = {node["id"]: node for node in (listed if isinstance(listed, list) else listed.values())}
    pages = {node["name"] for node in nodes.values() if node["type"] == "page"}
    if options.page is not None and options.page not in pages:
        sys.exit(f"no page {options.page!r}; pages: {', '.join(sorted(pages))}")

    library_store = (document.get("libraries") or {}).get("store")

    def ancestors(node):
        parent = nodes.get(node.get("parent"))
        while parent:
            yield parent
            parent = nodes.get(parent.get("parent"))

    def in_scope(node):
        for parent in ancestors(node):
            if parent["id"] == library_store:
                return False
            if parent["type"] == "page":
                return options.page is None or parent["name"] == options.page
        return False

    findings = 0
    layers = 0
    for node in nodes.values():
        if node["type"] in ("page", "componentSet") or not in_scope(node):
            continue
        layers += 1
        inside_instance = any(parent["type"] == "instance" for parent in ancestors(node))
        found = [] if inside_instance else list(raw_values(node))
        if inside_instance or node["type"] == "instance":
            found += overrides(node, {field for field, _ in found}, top_level=not inside_instance)
        for field, value in found:
            print(f"{node['id']} {node.get('name')!r}: {field} = {json.dumps(value)}")
            findings += 1
    print(f"{findings} findings in {layers} layers")
    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main())
