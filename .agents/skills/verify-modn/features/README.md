# Modn CLI feature map

The verification driver is `../scripts/verify.py`; run it from the repository root using `python .agents/skills/verify-modn/scripts/verify.py --build` (or `python3` on Linux). Requires Cargo, Python 3.10+, and no live game installation. All ZIP archives, game roots and state are disposable. The printed `modn-verify-evidence-*` directory contains numbered command transcripts and `summary.json`; it remains after cleanup.

| Feature | Current verification |
| --- | --- |
| [Game module](module.md) | `test-game` Luau module check and explicit unsupported game registration/install |
| [Archive preview](archive-preview.md) | ZIP inspection and read-only Cyberpunk-shaped dry run |
| [Install and ownership](install-ownership.md) | Actual approved install, duplicate conflict, inventory, integrity and uninstall |
| [Recovery](recovery.md) | Idle `recover --yes` on isolated state (injected crash pending) |

The test-game module intentionally uses `bin/TestGame.exe`, `package/plugins/*.plugin`, and `addons/*.plugin`. The current production CLI still hardcodes Cyberpunk for game registration, scan, install and uninstall. **No test-game installation claim is made.** Add and verify that journey in PR #12 without bypasses or hidden game-specific Rust adapters. Record unsupported/unverified routes distinctly from successful exercises.
