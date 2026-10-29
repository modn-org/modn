# Game selection and module loading

## Sub-features

- M1: Check `test-game` API v1 identity through the public CLI.
- M2: Reject unregistered `--path` without an explicit `--game`.
- M3: Register test-game and select its recorded game identity.
- M4: Register Cyberpunk alongside test-game; reject ambiguous selection.

## How to get to it (user POV)

`modn module check <directory> --allow-root <root>`; `modn game add test-game --path <game>`; `modn game list`; use global `--game test-game` on unregistered game operations.

## Driving it with Python subprocess

The script copies `tests/fixtures/test-game/` into its temporary module root, checks module identity using the real CLI, and confirms `install --dry-run --path <unregistered-game>` fails without touching state. It previews with `--game test-game`, registers the game, then runs the remaining commands using the recorded identity. A second `game add cyberpunk-2077` uses a distinct fixture, and `scan` without disambiguation fails.

## Gotchas

Module validation does not authorize installation, and an unregistered path requires `--game`. Game IDs must match actual module manifests; no implicit Cyberpunk alias or fallback is provided.
