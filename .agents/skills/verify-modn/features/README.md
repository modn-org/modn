# Modn verification feature map

Run `python .agents/skills/verify-modn/scripts/verify.py --build` from the repository root (on Linux, `python3`). Python 3.10+ and Rust/Cargo are required; no real game. The normal executable is built and driven through subprocess.

The driver creates disposable game roots, a module directory, ZIPs and SQLite state in `modn-verify-fixture-*`. It retains independent JSON CLI command transcripts and summary in `modn-verify-evidence-*` after removing the fixture. A failed command terminates the run and preserves evidence.

- [Game selection](module.md): test-game module check, explicit selection and registration, no implicit game, multiple registrations and selector-free read-only state.
- [Archive and dry run](archive-preview.md): ZIP metadata and fake-game Lua mapping without writes.
- [Install and ownership](install-ownership.md): approved real install, owned/unmanaged scan, conflicts, drift, concurrent ownership with a second game, and independent uninstalls. The Cyberpunk fixture also verifies a wrapped redscript/ArchiveXL/TweakXL ZIP using synthetic framework markers, with no writes to real games.
- [Operation history and undo](history-undo.md): one install record, undo selection and its uninstall record, conflict and missing-approval refusals, dry-run safety, unknown IDs and manual-source restore refusal.
- [Portable Windows lifecycle](portable-mvp.md): release binary bootstrap, synthetic first-party-policy prerequisite stack, update, pin, observed Windows rollback and live provider acquisition.
- [Recovery](recovery.md): idle recovery only; injected crash recovery remains covered by Rust tests.

The fake game deliberately differs from Cyberpunk: `bin/TestGame.exe`, source `Demo/package/plugins/example.plugin`, destination `addons/example.plugin`. Both modules load through the same runtime without special-case Rust branches.
