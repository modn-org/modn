# Operation history and undo
## Sub-features
- U1: After a standalone install, `modn history --json` shows one install record for `test-game` with its owned file and no `undoes` field.
- U2: `modn undo --yes` without an ID removes the newest install, clears its inventory and records an uninstall whose `undoes` names the install.
- U3: A second undo of the same install is refused with `UNDO_CONFLICT` and changes nothing.
- U4: Missing `--yes` yields `INVALID_ARGUMENTS`, `--dry-run` describes the inverse without writing, and an unknown ID yields `UNDO_NOT_FOUND`.
- U5: Undoing an uninstall whose release has a `manual` source is refused with `UNDO_SOURCE_MANUAL` and changes nothing.
## How to get to it (user POV)
`modn history --game test-game --json`; `modn install <zip> --yes --path <game>`; `modn undo --yes --path <game>`; `modn undo <operation-id> --yes --path <game>`; `modn undo <operation-id> --dry-run --path <game>`; `modn uninstall <package-id> --yes --path <game>`. Undo without an ID selects the newest operation for the selected game. Error codes arrive on stderr as JSON when `--json` is set.
## Driving it with Python subprocess
The driver runs the real `modn` binary through `Driver.run`/`Driver.json`, so every command leaves a numbered JSON transcript in the evidence directory. `undo_checks.verify_undo_history` owns U1..U5 and calls the existing install, list, scan and uninstall helpers. It reads the game file bytes, inventory and `history --json` itself instead of trusting a success exit code. Each history assertion compares record counts with a snapshot taken before the command.

The driver runs U1, then U4, then U2, U3 and U5. U4 must precede U2: once U2 writes an undo record, the CLI treats it as a later operation on the same files and refuses the install ID with `UNDO_CONFLICT` before it reaches the missing-`--yes` or dry-run checks.
## Gotchas
A standalone archive has a `manual` source. Undoing its uninstall cannot fetch an archive, so the CLI refuses with `UNDO_SOURCE_MANUAL`; this is expected behavior, not a driver failure. A successful undo writes a new record and never edits the target record, so history grows by one. `--dry-run` conflicts with `--yes`. `modn history` has no `--path`; select the fixture with `--game test-game`. Always keep the fixture game root and `--state-dir` disposable, and compare a full file snapshot before and after every refusal.
