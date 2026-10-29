---
name: verify-modn
description: Exercise Modn's CLI against disposable game fixtures and preserve independent evidence; use for game-module, installation, ownership, or recovery changes.
---

# Verify Modn

Run this repository-owned CLI verification after implementation changes. No real game installations, external accounts, browser, or test-only Rust hooks are required. The driver uses Python's standard library, invokes the actual `modn` executable, and keeps evidence outside the disposable game directory.

## Launch

From the repository root:

```powershell
python .agents/skills/verify-modn/scripts/verify.py --build
```

On Linux use `python3` instead of `python`. Requires Python 3.10+ and a working Rust/Cargo toolchain. To reuse an already built executable, omit `--build`; or pass `--binary <path-to-modn>`. The normal build is `cargo build -p modn-cli --locked`.

The driver creates two isolated directories using the OS temporary directory: `modn-verify-fixture-*` for game roots, ZIPs and SQLite state, and `modn-verify-evidence-*` for permanent proof. No server is launched or process left running; each CLI invocation is short-lived.

## Doctor

Before acting, the script checks that the executable and `tests/fixtures/test-game/{module.json,main.luau}` exist, calls `modn --help`, and validates the **ordinary** `test-game` module with:

```text
modn --json --state-dir <fixture>/state module check tests/fixtures/test-game --allow-root <fixture>/test-game
```

Success requires `game_id: "test-game"` and API version 1. The module is not privileged. The test-game marker is `bin/TestGame.exe`; its ZIP policy maps `package/plugins/*.plugin` to `addons/*.plugin`, intentionally unlike Cyberpunk.

**Current limitation (before PR #12):** `modn game add test-game` and `modn install --path <test-game>` are not wired to runtime game selection. The driver proves these paths reject the fixture; it does **not** call a Rust internal API or substitute a Cyberpunk marker to claim test-game installation works. A separate disposable Cyberpunk-shaped fixture exercises the available real CLI installation path.

## Drive

Run the driver above. It exercises the flows in [features/README.md](features/README.md): module check, ZIP inspection and dry-run immutability, real `--yes` install, inventory, conflict rejection, modified-file uninstall protection, uninstall and idle recovery. Failures return nonzero.

The script checks physical file hashes/content and SQLite/state side effects independently of exit codes and JSON reports. It requires explicit `--yes` only on its own disposable installation. It never uses real user game paths or state directories. Re-run it after changing any mapped CLI flow.

## Evidence

The script prints its `modn-verify-evidence-*` directory on success or failure. Each numbered `<step>.json` includes the exact CLI argv, exit code, stdout and stderr. `summary.json` records verified results and unsupported routes. `build.json` records optional build output. No secrets are used; evidence lives in the OS temporary directory, **not** in a repository note/worklog.

Read `summary.json` plus the command transcripts before reporting success. A clean build alone does not prove behavior. Keep results from failed runs as troubleshooting evidence.

## Cleanup

The driver removes only the uniquely created `modn-verify-fixture-*` directory in a `finally` block. It does not remove `modn-verify-evidence-*`, any user installation, other test runs, or persistent processes. Evidence cleanup is a separate deliberate operator action.

This skill does not replace `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`, or `cargo test --workspace --locked` when Rust changes require them.
