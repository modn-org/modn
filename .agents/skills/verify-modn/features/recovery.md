# Transaction recovery

## Sub-features

- R1: Completed transactions leave no pending recovery work.
- R2: Crash-injection recovery is not yet exercised by this CLI-only verifier.

## How to get to it (user POV)

`modn recover --yes --json` and `modn installed --path <game> --json`.

## Driving it with Python subprocess

After a complete disposable installation and uninstall, the driver invokes `recover --yes` against the same state and asserts `recovered == 0`. It also checks the managed-package list is empty and the pre-existing unmanaged file remains intact. Existing Rust transaction integration tests cover injected crashes, but this driver has no public CLI crash injection switch, so it must not claim crash recovery was exercised.

## Gotchas

Do not simulate a crash by killing a CLI while writing to a real installation. A future safe test harness may cover interrupted transactions using only disposable state and a supported injection entry point. Report this path as unverified by `verify-modn`.
