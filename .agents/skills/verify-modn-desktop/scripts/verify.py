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
    (7, "approval", "journeys_approval:journey_approval"),
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
    parser.add_argument("--selftest", action="store_true", help="check dialog_for and the CLI override guard against fakes")
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


class _FakeToastSession:
    """The `exists` surface the toast guard reads."""

    def __init__(self, visible_polls):
        self.visible_polls = visible_polls

    def exists(self, name=None, prefix=None, control_type=None):
        self.visible_polls -= 1
        return self.visible_polls > 0


class _FakeNamedToasts:
    """Shows only the controls named in `visible`."""

    def __init__(self, visible):
        self.visible = set(visible)

    def exists(self, name=None, prefix=None, control_type=None):
        return name in self.visible


def selftest():
    """Checks the helpers the journeys rely on against fakes and temp files."""
    import shutil  # noqa: PLC0415 - used only by the evidence check
    import tempfile  # noqa: PLC0415 - used only by the CLI override check

    import fixtures  # noqa: PLC0415 - harness has bootstrapped the interpreter
    import harness  # noqa: PLC0415 - harness has bootstrapped the interpreter
    import journeys_core  # noqa: PLC0415 - pure helpers only
    import lifecycle  # noqa: PLC0415 - pure helpers only
    import uia  # noqa: PLC0415 - harness has bootstrapped the interpreter
    import yaml  # noqa: PLC0415 - bootstrapped with the other packages

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

    # V2: an error toast must not count as a copied diagnostics report.
    results["diagnostics_success_passes"] = journeys_core.diagnostics_copied(
        "Diagnostics copied."
    )
    results["diagnostics_error_fails"] = not journeys_core.diagnostics_copied(
        "Something went wrong"
    )

    # V1: the toast guard returns once the toast is gone and fails while it stays.
    harness.wait_toasts_gone(_FakeToastSession(0), "Undo done", "selftest", timeout=0.3)
    harness.wait_toasts_gone(_FakeToastSession(0), ("Install done", "Undo done"), "selftest", timeout=0.3)
    results["toast_guard_passes_when_gone"] = True
    try:
        harness.wait_toasts_gone(_FakeToastSession(10**6), "Undo done", "selftest", timeout=0.3)
    except uia.Failure:
        results["toast_guard_fails_when_stale"] = True
    else:
        results["toast_guard_fails_when_stale"] = False
    try:
        harness.wait_toasts_gone(
            _FakeNamedToasts({"Undo done"}), ("Install done", "Undo done"), "selftest", timeout=0.3
        )
    except uia.Failure:
        results["toast_guard_checks_every_name"] = True
    else:
        results["toast_guard_checks_every_name"] = False

    # V3: a failure with an empty message still fails the journey.
    def raise_assertion():
        raise AssertionError()

    def raise_exception():
        raise Exception()

    runner = harness.Runner(None)
    runner.journey("empty-assertion", raise_assertion)
    runner.journey("empty-exception", raise_exception)
    runner.journey("normal", lambda: None)
    statuses = [result["status"] for result in runner.results]
    results["empty_assertion_fails"] = statuses[0].startswith("failed")
    results["empty_exception_fails"] = statuses[1].startswith("failed")
    results["normal_journey_passes"] = statuses[2] == "passed"

    # V5: each journey result carries the evidence directory it created.
    holder = {}

    def journey_with_fixture():
        holder["fixture"] = fixtures.Fixture()

    runner = harness.Runner(None)
    runner.journey("evidence", journey_with_fixture)
    fixture = holder["fixture"]
    try:
        results["result_carries_evidence"] = runner.results[0].get("evidence") == [
            str(fixture.evidence)
        ]
    finally:
        shutil.rmtree(fixture.root, ignore_errors=True)
        shutil.rmtree(fixture.evidence, ignore_errors=True)
        if fixture.evidence in fixtures.CREATED:
            fixtures.CREATED.remove(fixture.evidence)

    # V4: the front matter of this skill, and of every sibling, is valid YAML.
    def front_matter(path):
        parts = Path(path).read_text(encoding="utf-8").split("---", 2)
        if len(parts) < 3:
            return {}
        try:
            data = yaml.safe_load(parts[1])
        except yaml.YAMLError:
            return {}
        return data if isinstance(data, dict) else {}

    skill = Path(__file__).resolve().parents[1] / "SKILL.md"
    own = front_matter(skill)
    results["front_matter_name"] = isinstance(own.get("name"), str)
    results["front_matter_description"] = isinstance(own.get("description"), str)
    results["all_skill_front_matter"] = all(
        isinstance(front_matter(path).get("name"), str)
        and isinstance(front_matter(path).get("description"), str)
        for path in sorted(skill.parent.parent.glob("*/SKILL.md"))
    )

    with tempfile.TemporaryDirectory() as folder:
        both = Path(folder) / "both.exe"
        both.write_bytes(b"x MODN_APPDATA y MODN_STORE_LIBRARIES z")
        bare = Path(folder) / "bare.md"
        bare.write_text("# A skill without front matter\n", encoding="utf-8")
        open_only = Path(folder) / "open-only.md"
        open_only.write_text("---\nname: x\n", encoding="utf-8")
        results["front_matter_without_markers_is_invalid"] = (
            front_matter(bare) == {} and front_matter(open_only) == {}
        )
        one = Path(folder) / "one.exe"
        one.write_bytes(b"MODN_APPDATA")
        results["accepts_a_cli_with_both_overrides"] = harness.lacks_test_overrides(both) == []
        results["rejects_a_cli_without_one_override"] = harness.lacks_test_overrides(one) == [
            "MODN_STORE_LIBRARIES"
        ]
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
        lines = "".join(f"{number} {name}\n" for number, name, _ in JOURNEYS)
        sys.stdout.buffer.write(lines.encode())
        sys.stdout.flush()
        return 0
    if args.selftest:
        import harness  # noqa: F401,PLC0415 - bootstrap must run before uia is imported

        return selftest()
    selected = select(parser, args.only)
    import fixtures  # noqa: PLC0415 - harness imports this module during bootstrap
    import harness  # noqa: PLC0415 - bootstrap must run before uia is imported

    fixtures.KEEP = args.keep
    if args.build:
        build()
    harness.BINARIES.override(app=args.app, cli=args.cli, test_backend=args.test_backend)
    missing = harness.BINARIES.missing()
    if missing:
        parser.error(
            f"missing binary {missing[0]}; build it with --build or pass --app, --cli or --test-backend"
        )
    absent = harness.lacks_test_overrides(harness.BINARIES.cli)
    if absent:
        parser.error(
            f"{harness.BINARIES.cli} was built without modn-core/test-instruction-budget "
            f"({', '.join(absent)} not found). It would read the real Steam library and "
            "install into real game folders. Build the verifier CLI with --build."
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
