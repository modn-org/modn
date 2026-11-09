"""Check audit.py against a fixture with known findings.

Usage: python -I test_audit.py

Builds the fixture headless with figcraft-cli, runs audit.py on it, and
compares the reported (layer, field) pairs with the expected ones. Run it
after a FigCraft upgrade: a changed file format makes audit.py miss findings.
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_CLI = r"C:\Program Files\ArtCraft\FigCraft\figcraft-cli.exe"

FIXTURE = [
    {"command": "variable.createCollection", "params": {"name": "theme"}},
    {"command": "variable.create", "params": {"collection": "theme", "name": "modn/color/surface", "type": "color", "value": "#FFFFFF"}},
    {"command": "variable.create", "params": {"collection": "theme", "name": "modn/color/text", "type": "color", "value": "#111111"}},
    {"command": "variable.create", "params": {"collection": "theme", "name": "modn/color/danger", "type": "color", "value": "#CC3300"}},
    {"command": "style.create", "params": {"type": "text", "name": "label", "value": {"fontFamily": "Inter", "fontStyle": "Regular", "fontSize": 14, "lineHeight": 20, "letterSpacing": "0%"}}},
    {"command": "page.rename", "params": {"name": "Main"}},
    {"command": "node.create", "as": "chip", "params": {"type": "frame", "name": "Chip", "width": 80, "height": 24, "props": {"fill": {"$var": "modn/color/surface"}}}},
    {"command": "node.create", "as": "chip-label", "params": {"type": "text", "name": "label", "parent": {"$ref": "chip", "field": "id"}, "characters": "Chip", "props": {"fill": {"$var": "modn/color/text"}}}},
    {"command": "style.apply", "params": {"ids": [{"$ref": "chip-label", "field": "id"}], "style": "label"}},
    {"command": "component.create", "as": "component", "params": {"ids": [{"$ref": "chip", "field": "id"}]}},
    {"command": "component.addProperty", "params": {"id": {"$ref": "chip-label", "field": "id"}, "name": "label", "type": "text"}},
    {"command": "node.create", "params": {"type": "frame", "name": "bound", "props": {"fill": {"$var": "modn/color/surface"}}}},
    {"command": "node.create", "params": {"type": "frame", "name": "raw-fill", "props": {"fill": "#FF00FF"}}},
    {"command": "node.create", "as": "second", "params": {"type": "frame", "name": "second-paint", "props": {"fills": ["#FFFFFF", "#FF0000"]}}},
    {"command": "variable.bind", "params": {"ids": [{"$ref": "second", "field": "id"}], "field": "fills/0/color", "variable": "modn/color/surface"}},
    {"command": "node.create", "params": {"type": "frame", "name": "raw-gap", "props": {"fill": "none", "layout": "horizontal", "gap": 8}}},
    {"command": "node.create", "params": {"type": "text", "name": "no-style", "characters": "x"}},
    {"command": "instance.create", "as": "via-prop", "params": {"component": {"$ref": "component", "field": "id"}}},
    {"command": "instance.setProperties", "params": {"ids": [{"$ref": "via-prop", "field": "id"}], "properties": {"label": "Install"}}},
    {"command": "object.rename", "params": {"ids": [{"$ref": "via-prop", "field": "id"}], "name": "via-prop"}},
    {"command": "instance.create", "as": "raw-child", "params": {"component": {"$ref": "component", "field": "id"}}},
    {"command": "object.rename", "params": {"ids": [{"$ref": "raw-child", "field": "id"}], "name": "raw-child"}},
    {"command": "node.find", "as": "raw-child-label", "params": {"root": {"$ref": "raw-child", "field": "id"}, "name": "label"}},
    {"command": "node.set", "params": {"ids": [{"$ref": "raw-child-label", "field": "ids/0"}], "name": "raw-child-label", "fill": "#FF00FF"}},
    {"command": "instance.create", "as": "swap", "params": {"component": {"$ref": "component", "field": "id"}}},
    {"command": "object.rename", "params": {"ids": [{"$ref": "swap", "field": "id"}], "name": "swap"}},
    {"command": "node.find", "as": "swap-label", "params": {"root": {"$ref": "swap", "field": "id"}, "name": "label"}},
    {"command": "variable.bind", "params": {"ids": [{"$ref": "swap-label", "field": "ids/0"}], "field": "fill", "variable": "modn/color/danger"}},
    {"command": "object.rename", "params": {"ids": [{"$ref": "swap-label", "field": "ids/0"}], "name": "swap-label"}},
    {"command": "instance.create", "as": "direct", "params": {"component": {"$ref": "component", "field": "id"}}},
    {"command": "object.rename", "params": {"ids": [{"$ref": "direct", "field": "id"}], "name": "direct"}},
    {"command": "node.find", "as": "direct-label", "params": {"root": {"$ref": "direct", "field": "id"}, "name": "label"}},
    {"command": "node.set", "params": {"ids": [{"$ref": "direct-label", "field": "ids/0"}], "characters": "Typed"}},
    {"command": "object.rename", "params": {"ids": [{"$ref": "direct-label", "field": "ids/0"}], "name": "direct-label"}},
    {"command": "instance.create", "as": "shadow", "params": {"component": {"$ref": "component", "field": "id"}}},
    {"command": "object.rename", "params": {"ids": [{"$ref": "shadow", "field": "id"}], "name": "shadow"}},
    {"command": "effect.add", "params": {"ids": [{"$ref": "shadow", "field": "id"}], "effect": {"type": "dropShadow", "color": "#00000040", "offset": [0, 2], "radius": 6, "spread": 0}}},
    {"command": "page.add", "params": {"name": "Other"}},
    {"command": "node.create", "params": {"type": "frame", "name": "other-raw", "props": {"fill": "#00FF00"}}},
]

EXPECTED_MAIN = {
    ("raw-fill", "fill"),
    ("second-paint", "fills/1"),
    ("no-style", "fill"),
    ("no-style", "textStyle"),
    ("raw-gap", "gap"),
    ("raw-child-label", "override"),
    ("swap-label", "override"),
    ("direct-label", "override"),
    ("shadow", "effects"),
}
EXPECTED_ALL = EXPECTED_MAIN | {("other-raw", "fill")}


def findings(audit_output):
    pairs = set()
    for line in audit_output.splitlines():
        if ": " not in line or " = " not in line:
            continue
        head, _, rest = line.partition(": ")
        name = head.split(" ", 1)[1].strip("'\"")
        pairs.add((name, rest.split(" = ")[0]))
    return pairs


def main():
    cli = shutil.which("figcraft-cli") or DEFAULT_CLI
    audit = os.path.join(HERE, "audit.py")
    failures = 0
    with tempfile.TemporaryDirectory() as scratch:
        script = os.path.join(scratch, "fixture.json")
        fixture = os.path.join(scratch, "fixture.figcraft")
        with open(script, "w", encoding="utf-8") as out:
            json.dump(FIXTURE, out)
        built = subprocess.run([cli, "run", script, "--no-system-fonts", "--out", fixture], capture_output=True, text=True, encoding="utf-8")
        if built.returncode != 0:
            sys.exit(f"fixture build failed:\n{built.stdout}{built.stderr}")
        for args, expected in (([], EXPECTED_ALL), (["--page", "Main"], EXPECTED_MAIN)):
            run = subprocess.run([sys.executable, "-I", audit, fixture, *args], capture_output=True, text=True, encoding="utf-8")
            got = findings(run.stdout)
            label = " ".join(args) or "whole file"
            if got == expected and run.returncode == 1:
                print(f"PASS {label}: {len(got)} findings")
            else:
                failures += 1
                print(f"FAIL {label}: exit {run.returncode}")
                print(f"  missing: {sorted(expected - got)}")
                print(f"  unexpected: {sorted(got - expected)}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
