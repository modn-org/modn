"""List the top-level nodes of each page and the pairs that overlap.

Usage: overlap.py FILE.figcraft [--page NAME]

--page limits the check to one page.

A top-level node is a direct child of a page; component sets and other layers
count as frames. Its canvas position is the translation of its transform, and a
layer without one sits at 0, 0. Two nodes overlap when their boxes share area.
Touching edges count too, because each pair needs a positive gap. Prints one
line per node, a summary line per page and the name of each overlapping pair.
Exits 1 when a page has an overlapping pair, 0 when none does.
"""

import argparse
import os
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from audit import load


def position(node):
    transform = node.get("transform")
    return (transform[4], transform[5]) if transform else (0.0, 0.0)


def overlapping(first, second):
    ax, ay = position(first)
    bx, by = position(second)
    # Equal edges mean a gap of 0, and a gap is required, so the test is inclusive.
    return (
        ax <= bx + second["width"]
        and bx <= ax + first["width"]
        and ay <= by + second["height"]
        and by <= ay + first["height"]
    )


def main():
    parser = argparse.ArgumentParser(usage=__doc__)
    parser.add_argument("file")
    parser.add_argument("--page")
    options = parser.parse_args()
    sys.stdout.reconfigure(encoding="utf-8")
    document = load(options.file)
    listed = document["nodes"]
    nodes = list(listed.values()) if isinstance(listed, dict) else listed
    pages = [node for node in nodes if node["type"] == "page"]
    names = [page["name"] for page in pages]
    if options.page is not None and options.page not in names:
        sys.exit(f"no page {options.page!r}; pages: {', '.join(sorted(names))}")

    findings = 0
    for page in pages:
        if options.page is not None and page["name"] != options.page:
            continue
        frames = [node for node in nodes if node.get("parent") == page["id"]]
        for frame in frames:
            x, y = position(frame)
            print(f"{page['name']} | {frame.get('name')} | {x:g} {y:g} {frame['width']:g} {frame['height']:g}")
        pairs = [
            (first, second)
            for index, first in enumerate(frames)
            for second in frames[index + 1:]
            if overlapping(first, second)
        ]
        print(f"{page['name']}: {len(frames)} top-level nodes, {len(pairs)} overlapping pairs")
        for first, second in pairs:
            print(f"  {first.get('name')} overlaps {second.get('name')}")
        findings += len(pairs)
    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main())
