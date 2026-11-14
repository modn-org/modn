---
name: verify-modn
description: Verify Modn's real CLI game selection, Lua ZIP installation, ownership and recovery using disposable game fixtures and retained evidence.
---

# Verify Modn

Use for code changing game module loading, game selection, ZIP planning, install, inventory, diagnostics, uninstall or recovery. The driver runs the **real** `modn` binary with a fictional `test-game` Lua module and a second Cyberpunk fixture. It uses no privileged Rust test hooks and never accesses user games.

## Launch

From the repository root run:

```powershell
python .agents/skills/verify-modn/scripts/verify.py --build
```

Use `python3` on Linux. Requires Python 3.10+, Rust and Cargo. Or omit `--build` for an already built `target/debug/modn[.exe]`, or pass `--binary <path>`. It uses OS-temporary `modn-verify-fixture-*` and `modn-verify-evidence-*` directories. Each CLI command terminates; no service remains running. A disposable contract v2 bucket, signed with a throwaway key, supplies the Cyberpunk module and a pinned test package. Modules are copied to the disposable `modules/` root, selected via the production `MODN_GAME_MODULES_DIR` environment override. The driver never reads the `buckets/main` submodule.

## Doctor

The driver checks its binary and test-game module fixture, runs `modn --help`, then invokes `modn module check tests/fixtures/test-game --allow-root <fixture-game>` through the real CLI. It requires the returned game ID `test-game` and host API version 1. The fixture uses `bin/TestGame.exe` and `addons/*.plugin`, not Cyberpunk conventions.

## Drive

The driver performs [feature-mapped](features/README.md) CLI actions, including:

- Confirm an **unregistered game path without `--game`** fails without creating a database.
- Preview test-game ZIP mapping with explicit `--game test-game`; verify no writes.
- Register `test-game`, then install through its recorded identity without `--game`.
- Independently verify written bytes, owned/unmanaged scan, conflict rejection, doctor drift, modified-file uninstall protection, safe uninstall and idle recovery.
- Register and install a second `cyberpunk-2077` fixture concurrently, confirm independent ownership for both games, reject ambiguous selection, and uninstall each without changing the other.
- Exercise a wrapped redscript/ArchiveXL/TweakXL ZIP on disposable Cyberpunk paths with **synthetic presence-only** framework markers, verifying read-only preview, approved install, owned scan, doctor and uninstall without altering manual markers or the other game.
- Exercise `modn history` and `modn undo` on a standalone install: one install record with its owned file, undo without an ID, a repeated undo refused with `UNDO_CONFLICT`, `INVALID_ARGUMENTS`, dry-run and `UNDO_NOT_FOUND` guards, and `UNDO_SOURCE_MANUAL` for a manual-source uninstall, each checked against game bytes and a before/after snapshot.

The `test-game` module uses ordinary runtime Lua policy and ZIP operations; its full CLI lifecycle is now verified. No game-specific Rust adapter or default ID is used.

## Evidence

Each step produces a numbered JSON command transcript (arguments, exit, stdout, stderr) in the printed `modn-verify-evidence-*` directory. `summary.json` contains the independently checked results; optional `build.json` records compilation. Check the resulting game file bytes and inventory records, not just success exit codes.

## Cleanup

The driver deletes only the uniquely created disposable `modn-verify-fixture-*` directory, even on failure. It preserves evidence after cleanup. It never touches real installations, other test processes or other runs.

Optional metadata-only ZIP layout benchmarking with synthetic dependency markers:  `python .agents/skills/verify-modn/scripts/benchmark_cp77.py <download-directory>` uses a disposable game root and local ZIP metadata/CLI dry-runs only. It never executes, copies or extracts downloaded mod payloads or creates state; results remain local and are not committed. This is separate from the synthetic write-path tests.

Crash-injected recovery is not covered by this CLI exercise. Keep existing Rust crash-recovery tests and project-mandated cargo fmt/clippy/tests as separate required checks. CI exercises this skill on Linux and Windows.
