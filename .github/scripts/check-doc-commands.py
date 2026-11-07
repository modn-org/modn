"""Check that documented script commands pass every mandatory parameter.

A command passes when it names every mandatory parameter of at least one
of the script's PowerShell parameter sets.
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
PARAMETER = re.compile(r"((?:\[Parameter\([^)]*\)\]\s*)?)\[[^\]]+\]\s*\$(\w+)")
SET_NAME = re.compile(r"ParameterSetName\s*=\s*'([^']+)'", re.I)
CONTINUATION = re.compile(r"`\r?\n\s*")


def required_sets(script: pathlib.Path) -> list[set[str]]:
    block = re.search(r"^param\((.*?)^\)", script.read_text(encoding="utf-8"), re.M | re.S)
    if not block:
        return []
    shared, by_set = set(), {}
    for attribute, name in PARAMETER.findall(block.group(1)):
        set_name = SET_NAME.search(attribute)
        if set_name:
            by_set.setdefault(set_name.group(1), set())
        if "mandatory" not in attribute.lower():
            continue
        if set_name:
            by_set[set_name.group(1)].add(name)
        else:
            shared.add(name)
    sets = [shared | names for names in by_set.values()] or [shared]
    return sets if any(sets) else []


def main() -> int:
    scripts = {
        path.name: required_sets(path)
        for path in sorted((ROOT / ".github/scripts").glob("*.ps1"))
    }
    docs = [ROOT / "README.md", *sorted((ROOT / "docs").glob("*.md"))]
    checked, missing = 0, []
    for doc in docs:
        text = CONTINUATION.sub(" ", doc.read_text(encoding="utf-8"))
        for line in text.splitlines():
            for name, sets in scripts.items():
                if not sets or not re.search(rf"-File\s+\S*{re.escape(name)}", line):
                    continue
                checked += 1
                given = {p.lower() for p in re.findall(r"(?<!\w)-(\w+)", line)}
                if not any({p.lower() for p in names} <= given for names in sets):
                    options = " or ".join(
                        " ".join(f"-{p}" for p in sorted(names)) for names in sets
                    )
                    missing.append(f"{doc.relative_to(ROOT).as_posix()}: {name} needs {options}")
    for problem in missing:
        print(problem, file=sys.stderr)
    print(f"Checked {checked} documented commands against {len(scripts)} scripts.")
    return 1 if missing else 0


if __name__ == "__main__":
    sys.exit(main())
