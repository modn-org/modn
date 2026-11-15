"""Shared checks for the approval journey: asks, approvals and toasts."""

import json
import sqlite3
import time

import uia
from lifecycle import _cli, _leaks

TOAST_MESSAGES = ("Install done", "Uninstall done", "Update done", "Undo done")


def wait_enabled(session, name, timeout=90):
    session.wait_for(
        lambda s: [c for c in s.find_all(name=name) if c.IsEnabled],
        f"enabled {name}",
        timeout=timeout,
    )


def version_of(packages, mod_id):
    for package in packages:
        if package.get("mod_id") == mod_id:
            return package.get("version")
    return None


def packages(fx, game_id, path):
    result = _cli(fx, "--game", game_id, "list", "--path", path)
    uia.require(result.returncode == 0, f"CLI list failed: {result.stderr}")
    return json.loads(result.stdout)


def approval_rows(fx, mod_id):
    """Every stored approval of `mod_id`, read read-only from the live state."""
    path = fx.state / "state.sqlite"
    if not path.is_file():
        return []
    try:
        connection = sqlite3.connect(
            f"file:{path.as_posix()}?mode=ro", uri=True, timeout=10
        )
        try:
            rows = connection.execute(
                "SELECT native_injector, executables FROM mod_approvals"
                " WHERE mod_id = ? ORDER BY bucket",
                (mod_id,),
            ).fetchall()
        finally:
            connection.close()
    except sqlite3.OperationalError:
        return []
    return [(bool(native), json.loads(executables)) for native, executables in rows]


def check_approval(runner, fx, mod_id, native, executables, context):
    rows = approval_rows(fx, mod_id)
    runner.check(
        len(rows) == 1,
        f"{context}: expected one stored approval for {mod_id}, found {rows}",
    )
    runner.check(
        rows[0] == (native, executables),
        f"{context}: stored approval is {rows[0]!r}, not {(native, executables)!r}",
    )


def ask_groups(session):
    return session.find_all(name="Approval request", control_type="GroupControl")


def wait_ask(session, description, timeout=60):
    session.wait_for(lambda s: ask_groups(s), description, timeout=timeout)
    groups = ask_groups(session)
    uia.require(
        len(groups) == 1,
        f"Expected one approval group for {description}, found {len(groups)}",
    )
    return groups[0]


def check_ask(runner, session, native, executables, context):
    """Asserts the exact ask the webview renders for a refusal."""
    runner.check(
        len(ask_groups(session)) == 1,
        f"{context}: the approval group is missing or repeated",
    )
    runner.check(
        session.exists(name="This change needs your approval."),
        f"{context}: the approval question is missing",
    )
    runner.check(
        session.exists(name="Allow native code through the native injector.") == native,
        f"{context}: the native-code line does not match the refusal",
    )
    runner.check(
        session.exists(name="Allow these executables:") == bool(executables),
        f"{context}: the executable line does not match the refusal",
    )
    for path in executables:
        entries = session.find_all(name=path, control_type="ListItemControl")
        runner.check(
            len(entries) == 1,
            f"{context}: expected one ask path entry for {path}, found {len(entries)}",
        )
        runner.check(
            bool(session.find_all(name=path, control_type="TextControl")),
            f"{context}: ask path {path} has no text entry",
        )
    runner.check(
        session.exists(name="Allow", control_type="ButtonControl"),
        f"{context}: the Allow button is missing",
    )
    runner.check(
        session.exists(name="Decline", control_type="ButtonControl"),
        f"{context}: the Decline button is missing",
    )


def wait_toast_no_ask(session, message, context, timeout=90):
    """Waits for a toast while rejecting a dialog, a CLI leak and any ask.

    Polling the whole tree also records an ask that flashes and closes.
    """
    seen_ask = False
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        windows = [window.Name for window in uia.titled_windows(session)]
        if windows:
            raise uia.Failure(f"A titled window opened during {context}: {windows}")
        tree = session.text()
        leaked = _leaks(tree)
        if leaked:
            raise uia.Failure(f"CLI output leaked into the UI during {context}: {leaked}")
        if ask_groups(session):
            seen_ask = True
        if message in tree:
            return seen_ask
        time.sleep(0.1)
    raise uia.Failure(f"Timed out waiting for {message!r} after {context}")


def clear_toasts(session, timeout=15):
    """Lets a toast expire so the next wait cannot match a stale message."""
    session.wait_for(
        lambda s: not any(s.exists(name=message) for message in TOAST_MESSAGES),
        "toasts to expire",
        timeout=timeout,
    )


def check_bytes(runner, base, files, context):
    for name, content in files.items():
        expected = content.encode() if isinstance(content, str) else content
        path = base / name
        runner.check(path.is_file(), f"{context}: {name} is missing")
        runner.check(path.read_bytes() == expected, f"{context}: {name} has other bytes")
