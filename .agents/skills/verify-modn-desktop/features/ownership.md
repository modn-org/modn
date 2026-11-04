# Owned package lifecycle

Users confirm each installation and strict uninstall separately.

## Sub-features

- O1: explicitly confirm installation after review.
- O2: list the resulting owned package and independently read its bytes.
- O3: cancel uninstall without changes.
- O4: preserve modified owned files and manual files during strict uninstall.

## How to get to it (user POV)

After review, select Install reviewed archive..., then Confirm install.
Select the resulting owned-package button in the sidebar.
Select Uninstall selected package..., then Confirm uninstall or Cancel review.
Refresh owned packages reloads the CLI inventory.

## Driving it with agent-desktop

Run the full `verify.py` command in [SKILL.md](../SKILL.md).
It confirms ZIP and genuine 7z installation through native buttons and checks bytes through Python.
It verifies ownership with a separate CLI list invocation.
It cancels one uninstall, then changes owned bytes and checks strict refusal.
It restores those disposable bytes and confirms uninstall.
It checks that only the owned file disappears and manual content remains intact.

## Gotchas

Owned package IDs are the current button labels.
Uninstall never passes force, cascade, or dependency-removal flags.
All writes use the disposable test game and explicit manager state.
