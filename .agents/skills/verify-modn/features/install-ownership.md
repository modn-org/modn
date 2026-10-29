# Approved installation, inventory and uninstall

## Sub-features

- I1: Register and install the fictional test-game mod via the normal CLI.
- I2: Preserve a pre-existing unmanaged add-on.
- I3: Reconcile owned and unmanaged files using test-game's Luau scanner.
- I4: Reject conflicting installs and modified-file uninstalls.
- I5: Diagnose drift through test-game's Luau doctor and uninstall safely.

## How to get to it (user POV)

`modn game add test-game --path <game>`; `modn install <zip> --yes --path <game>`; `modn installed --path <game>`; `modn scan --path <game>`; `modn doctor --path <game>`; `modn uninstall <package-id> --yes --path <game>`. Registration provides the game ID for later commands.

## Driving it with Python subprocess

The driver creates `addons/manual.plugin` before installation, then installs `addons/example.plugin` through the Luau-planned ZIP. It checks bytes on disk, owned package IDs, scan `kind: test-plugin`, and unmanaged detection. A duplicate install fails; manually modified managed bytes trigger `OWNED_FILE_DRIFT` and an uninstall refusal. After restoring bytes, uninstall removes only the owned file and the package disappears from the ledger.

## Gotchas

Always isolate game and database paths. `--yes` is required for writes, and one recorded game can be selected without a redundant `--game`; multiple registered games without a path or ID are ambiguous. No database state is placed inside the fixture game.
