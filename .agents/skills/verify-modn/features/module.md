# Fictional game module discovery and validation

## Sub-features

- M1: Load an ordinary `test-game` module through host API v1.
- M2: Grant only a disposable filesystem root for module identity checks.
- M3: Record CLI game selection and installation as unsupported until PR #12.

## How to get to it (user POV)

Use `modn module check <directory> --allow-root <game>` (optionally `--json`), `modn game add test-game --path <game>`, or `modn install <zip> --dry-run --path <game>`.

## Driving it with Python subprocess

The driver creates `bin/TestGame.exe` under its isolated test-game root and invokes `modn --json --state-dir <state> module check tests/fixtures/test-game --allow-root <game>`. Assert `game_id == "test-game"`, `api_version == 1`, and unchanged fixture contents. Then invoke game registration and a test-game dry run as negative CLI tests: both currently fail and must not write into the game directory.

The normal module includes `validate`, `scan`, `doctor`, `steam_name`, `is_epic`, and schema-v1 `plan` functions, but the current CLI does not invoke them for the `test-game` ID. Their integration is not claimed verified here.

## Gotchas

Module `check` validates identity/compatibility; it does **not** activate or register a game or execute a plan. A fake Cyberpunk executable or Rust test-only override would invalidate the architecture proof. PR #12 should replace these negative tests with the full test-game lifecycle.
