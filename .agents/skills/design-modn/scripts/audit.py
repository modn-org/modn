"""List design values in a .figcraft file that no variable or style holds.

Usage: audit.py FILE.figcraft [--page NAME]

--page limits the check to one page.

Checks every layer outside instances and outside the read-only library copies
that a page stores: visible fills and strokes, corner
radius, gap and padding, opacity, effects and text styles. Prints one line per
raw value and a count. Exits 1 when it finds one, 0 when every value is bound.
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


def load(path):
    cli = shutil.which("figcraft-cli") or DEFAULT_CLI
    with tempfile.TemporaryDirectory() as scratch:
        out = os.path.join(scratch, "document.json")
        subprocess.run([cli, "convert", path, out], check=True, capture_output=True)
        with open(out, encoding="utf-8") as source:
            return json.load(source)


def visible(paints):
    return [paint for paint in paints or [] if paint.get("visible", True)]


def raw_values(node):
    bound = node.get("bound") or {}
    styles = node.get("styles") or {}
    if visible(node.get("fills")) and "fill" not in bound and not styles.get("fill"):
        yield "fill", visible(node["fills"])[0].get("color")
    if visible(node.get("strokes")) and "stroke" not in bound and not styles.get("stroke"):
        yield "stroke", visible(node["strokes"])[0].get("color")
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

    def skipped(node):
        parent = nodes.get(node.get("parent"))
        while parent:
            if parent["type"] == "instance" or parent["id"] == library_store:
                return True
            if parent["type"] == "page" and options.page is not None and parent["name"] != options.page:
                return True
            parent = nodes.get(parent.get("parent"))
        return False

    findings = 0
    for node in nodes.values():
        if node["type"] in ("page", "componentSet") or skipped(node):
            continue
        for field, value in raw_values(node):
            print(f"{node['id']} {node.get('name')!r}: {field} = {json.dumps(value)}")
            findings += 1
    print(f"{findings} raw values in {len(nodes)} layers")
    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main())
