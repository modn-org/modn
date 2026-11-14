"""Verify Modn Desktop through UI Automation on disposable fixtures.

Run from the repository root:

    uv run --no-project python .agents/skills/verify-modn-desktop/scripts/verify.py --build

The script builds the app and the test-instruction-budget CLI, then runs every
journey against a fresh temporary fixture. It never touches a real game, a
real store library or the owner's Modn state, and it only addresses the Modn
window of the process it started.

Binaries are read from `CARGO_TARGET_DIR` when it is set, otherwise from
`<repo>/target`. `--app`, `--cli` and `--test-backend` override one each.
"""

import argparse
import json
import os
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

REPO = Path(__file__).resolve().parents[4]

# (number, name, "module:function") in run order.
JOURNEYS = [
    (1, "detection", "journeys_core:journey_detection"),
    (2, "progress", "journeys_core:journey_progress"),
    (3, "cancel", "journeys_core:journey_cancel"),
    (4, "finishing", "journeys_core:journey_finishing"),
    (5, "lifecycle", "lifecycle:journey_lifecycle"),
    (6, "blocked", "journeys_core:journey_blocked"),
    (7, "approval", "lifecycle:journey_approval"),
    (8, "no-cli-output", "journeys_core:journey_no_cli_output"),
    (9, "play", "journeys_core:journey_play"),
    (10, "remove", "journeys_remove:journey_remove"),
    (11, "catalog", "journeys_catalog:journey_catalog"),
]


def parse_args(argv=None):
    parser = argparse.ArgumentParser(description="Verify Modn Desktop through UI Automation.")
    parser.add_argument("--build", action="store_true", help="build the app and the fixture CLI first")
    parser.add_argument("--list", action="store_true", help="list the journeys in run order and exit")
    parser.add_argument(
        "--only", default="",
        help="comma-separated journey names or numbers to run (default: all)",
    )
    parser.add_argument("--keep", action="store_true", help="keep fixtures and evidence")
    parser.add_argument("--selftest", action="store_true", help="check dialog_for against fake windows")
    parser.add_argument("--app", metavar="PATH", help="override the built modn-desktop.exe")
    parser.add_argument("--cli", metavar="PATH", help="override the built modn.exe")
    parser.add_argument("--test-backend", metavar="PATH", help="override the built test backend")
    return parser, parser.parse_args(argv)


def select(parser, only):
    """The journeys named by --only, in run order."""
    if not only.strip():
        return list(JOURNEYS)
    selected = []
    for item in only.split(","):
        item = item.strip()
        matches = [entry for entry in JOURNEYS if item == entry[1] or item == str(entry[0])]
        if not matches:
            valid = ", ".join(f"{number} {name}" for number, name, _ in JOURNEYS)
            parser.error(f"unknown journey {item!r}; valid journeys: {valid}")
        selected.extend(matches)
    return selected


def resolve(spec):
    """Imports a journey from its `module:function` spec."""
    module_name, function_name = spec.split(":", 1)
    module = __import__(module_name)
    return getattr(module, function_name)


class _FakeWindow:
    def __init__(self, name):
        self.Name = name


class _FakeSession:
    """The `extra_windows` surface `dialog_for` reads."""

    def __init__(self, names):
        self.names = names

    def extra_windows(self):
        return [_FakeWindow(name) for name in self.names]


def selftest():
    """Checks `dialog_for` ordering and timeout against fake window lists."""
    import uia  # noqa: PLC0415 - harness has bootstrapped the interpreter

    results = {}
    results["ignores_untitled"] = (
        uia.dialog_for(_FakeSession(["", "Modn"]), timeout=0.5).Name == "Modn"
    )
    results["returns_first_of_two"] = (
        uia.dialog_for(_FakeSession(["Modn", "Second"]), timeout=0.5).Name == "Modn"
    )
    started = time.monotonic()
    try:
        uia.dialog_for(_FakeSession([""]), timeout=0.4)
    except uia.Failure:
        results["raises_on_timeout"] = time.monotonic() - started >= 0.3
    else:
        results["raises_on_timeout"] = False
    passed = all(results.values())
    print(json.dumps({"selftest": "passed" if passed else "failed", "checks": results}))
    return 0 if passed else 1


def build():
    commands = [
        ["cargo", "build", "-p", "modn-desktop", "--features", "verification", "--locked"],
        ["cargo", "build", "-p", "modn-cli", "--features", "modn-core/test-instruction-budget", "--locked"],
    ]
    for command in commands:
        print("+ " + " ".join(command))
        result = subprocess.run(command, cwd=REPO, env=os.environ.copy())
        if result.returncode != 0:
            raise SystemExit(f"build failed: {' '.join(command)}")


def main():
    parser, args = parse_args()
    if args.list:
        for number, name, _ in JOURNEYS:
            print(f"{number} {name}")
        return 0
    if args.selftest:
        import harness  # noqa: F401,PLC0415 - bootstrap must run before uia is imported

        return selftest()
    selected = select(parser, args.only)
    import harness  # noqa: PLC0415 - bootstrap must run before uia is imported

    if args.build:
        build()
    harness.BINARIES.override(app=args.app, cli=args.cli, test_backend=args.test_backend)
    missing = harness.BINARIES.missing()
    if missing:
        parser.error(
            f"missing binary {missing[0]}; build it with --build or pass --app, --cli or --test-backend"
        )
    runner = harness.Runner(args)
    started = time.monotonic()
    try:
        for number, name, spec in selected:
            function = resolve(spec)
            runner.journey(f"{number} {name}", lambda function=function: function(runner))
    finally:
        runner.clean()
    passed = sum(1 for r in runner.results if r["status"] == "passed")
    summary = {
        "status": "passed" if passed == len(runner.results) and runner.results else "failed",
        "journeys": len(runner.results),
        "passed": passed,
        "failed": len(runner.results) - passed,
        "assertions": runner.assertions,
        "seconds": round(time.monotonic() - started, 1),
        "results": runner.results,
    }
    print(json.dumps(summary))
    return 0 if summary["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
