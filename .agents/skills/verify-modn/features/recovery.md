# Transaction recovery

## Sub-features

- R1: Completed install/uninstall leaves no pending transactions.
- R2: Crash-injected rollback and recovery remain unverified by this CLI driver.

## How to get to it (user POV)

`modn recover --yes --json`, with the same `--state-dir` used by installation.

## Driving it with Python subprocess

Following the full test-game install/uninstall and second-game preview, the driver runs `recover --yes` on its isolated store and checks `recovered == 0`. Ownership and filesystem tests prove no accidental unmanaged deletion.

## Gotchas

No public CLI crash-injection capability exists; the skill does not fabricate one. Existing Rust transaction tests cover deliberate interrupted states. The driver deletes only its own temporary game roots and preserves evidence.
