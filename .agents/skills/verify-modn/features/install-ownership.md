# Transactional install, inventory and ownership

## Sub-features

- I1: Explicit `--yes` installation and disk content.
- I2: `installed` and `scan` distinguish owned and unmanaged files.
- I3: Repeated install is blocked, preserving existing content.
- I4: Modified-file uninstall refusal, restoration and safe uninstall.

## How to get to it (user POV)

`modn install <zip> --yes --path <game>`, `modn installed --path <game>`, `modn scan --path <game>`, `modn doctor --path <game>`, `modn uninstall <package-id> --yes --path <game>`; use global `--json` to return machine-readable results.

## Driving it with Python subprocess

The fixture starts with an unmanaged `archive/pc/mod/unmanaged.archive`. The driver installs a separate ZIP, checks actual bytes, reads its package ID from JSON, and verifies the independent `installed` and `scan` commands. A second install must fail without changing either file. After deliberately editing the owned file, `doctor` must report `OWNED_FILE_MODIFIED` and uninstall must fail without data loss. Restore the bytes, uninstall, and assert only the owned file disappears while the unmanaged file and ownership ledger remain consistent.

## Gotchas

This is real CLI behavior on an isolated **Cyberpunk-compatible** fixture, not proof of the test-game installation path. Do not run `--yes` against an actual game. Disk content and inventory must both be checked; a successful exit alone is insufficient.
