# Metadata inspection and dry-run

## Sub-features

- A1: Inspect a ZIP with fictional test-game source names.
- A2: Preview known legacy archive mapping under the currently selectable game.
- A3: Prove dry run is read-only through independent filesystem/state checks.

## How to get to it (user POV)

`modn inspect <zip> --json` and `modn install <zip> --dry-run --path <game> --json` (global `--json` flag may precede the subcommand).

## Driving it with Python subprocess

The driver creates a ZIP with `Demo/package/plugins/example.plugin`, inspects it, and checks the entry list. It separately creates a disposable Cyberpunk-compatible game marker plus `archive/pc/mod/managed.archive` ZIP and calls `modn --json --state-dir <state> install <zip> --dry-run --path <cyberpunk-fixture>`. Assert `status == "reviewable"`, `metadata_only == true`, and the declared destination. Compare file-content hashes before and after and assert the state directory was never created.

## Gotchas

ZIP inspection alone does not exercise test-game's Luau `plan` method. Preview output is not install approval or proof that extracted content is safe. Only the Cyberpunk-shaped fixture is currently recognized by the CLI; PR #12 must add actual test-game policy execution via normal selection.
