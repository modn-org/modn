---
name: verify-modn-desktop
description: Verify Modn Desktop's Windows GPUI Kit window, local archive review, confirmed ownership writes, refusals, and process lifecycle with isolated fixtures.
---

# Verify Modn Desktop

Run from the repository root on Windows in an unlocked interactive desktop session.
Use this skill after changes to `apps/modn-desktop/**` or its native driver.
Read [the feature map](features/README.md) before choosing a narrower journey.

## Build and doctor

```powershell
agent-desktop skills get desktop
agent-desktop skills get platform
agent-desktop status
cargo build -p modn-cli --locked
pwsh -NoProfile -File .agents/skills/verify-modn-desktop/scripts/doctor.ps1 -Backend target/debug/modn.exe
cargo build --manifest-path apps/modn-desktop/Cargo.toml --all-features --locked
cargo fmt --manifest-path apps/modn-desktop/Cargo.toml --all -- --check
cargo clippy --manifest-path apps/modn-desktop/Cargo.toml --workspace --all-targets --all-features --locked -- -D warnings
cargo test --manifest-path apps/modn-desktop/Cargo.toml --workspace --all-features --locked
```

Install `agent-desktop` with `npm install -g agent-desktop` if it is absent.
The scripts require Python 3 and use only its standard library.
The doctor checks eight prerequisites and prints a JSON count.
Require exit 0, eight checks, and zero failures.
The backend contract suite includes a real 60-second timeout.
Complete builds and tests before launching a driver.
Wait for that driver to close its app before rebuilding; Windows locks a running executable.
The nested workspace commands do not prove root workspace coverage.

## Launch and drive

```powershell
python .agents/skills/verify-modn-desktop/scripts/verify.py --backend target/debug/modn.exe
python .agents/skills/verify-modn-desktop/scripts/lifecycle.py --app apps/modn-desktop/target/debug/modn-desktop.exe --backend apps/modn-desktop/target/debug/modn-desktop-test-backend.exe
```

Require each script to exit 0 with `status: passed`, positive assertion counts, and zero command failures.
The first script exercises the real CLI through native UI controls.
The second uses the explicit test backend to check responsiveness, close refusal, and failure after a completed write.
The test backend requires a disposable-state marker and cannot use default state.

Each script creates unique `modn-desktop-fixture-*` and `modn-desktop-evidence-*` directories under the system temporary directory.
The fixture contains separate game, state, data, and module directories.
The real driver copies `tests/fixtures/test-game` and registers only its disposable installations.
It creates a manual file, valid ZIP, blocked ZIP, and malformed ZIP.
It never uses a real game or downloads a bucket.

The driver records its launched PID, executable hashes, full configuration, and window ID.
It accepts a window only when the reported PID matches the launched process.
Readiness requires the visible status `Select a registered installation.`.
Use only this window ID for subsequent native actions and screenshots.
Buttons use unique accessible names; take fresh refs after state changes.
The archive field's accessible name is `Absolute path to a local ZIP or 7z`.
The report document's name is `Proposed files, blockers and results`.
The verifier uses native file-dialog control ID 2 for Cancel.
The driver waits for a distinct window with its launched PID.
It reads only these controls to avoid capturing personal folder listings.
The drivers use semantic actions; they need no physical mouse or keyboard input.

## Evidence

Read the printed evidence path and its final `*-summary.json`.
Keep all native command JSON, backend records, app diagnostics, and cropped PNGs.
Classify this evidence as **sensitive** because paths and backend output can identify local data.
The verifier captures only the owned app window.
Do not upload evidence without reviewing it.

The real driver checks game and manager-state hashes after previews, refusals, and cancellation.
It independently reads installed bytes and invokes CLI `list` after confirmed writes.
It changes owned bytes before uninstall and proves strict refusal preserves them.
It restores the fixture bytes, confirms uninstall, and checks the original manual content.
Backend failure exits in backend records are expected in refusal cases.
Native tool and independent CLI commands must still report zero nonzero exits.

## Cleanup

The drivers click the Close control from the recorded window and wait for the launched process.
They do not kill apps by image name.
They remove only their validated unique fixture directory and retain evidence.
Require `app_exit: 0` and `fixture_removed: true` in each passing summary.

If interrupted, read that run's `*-launch.json` before cleanup.
Confirm its executable path, PID, and window ID against a fresh native inventory.
Wait for active work, then close only that window.
Remove only the recorded `modn-desktop-fixture-*` directory after the app exits.
Keep its separate evidence directory, including failure evidence.
Never clean another run by prefix alone.

## Limits

The native journey covers ZIP and the synthetic fixture module.
Native picker selection, 7z archives, Windows 10 hardware, and real-game compatibility need additional proof.
The launcher supports only documented absolute configuration paths.
Forced termination and OS crashes can require CLI recovery.
See [desktop product documentation](../../../docs/mvp/desktop.md) for user-facing limits and licensing.
