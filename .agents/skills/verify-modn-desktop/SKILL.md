---
name: verify-modn-desktop
description: Verify the Modn Desktop Tauri window through UI Automation on disposable fixtures: startup detection, one-click install progress and cancel, finishing close refusal, blocked plans, play, diagnostics and CLI-output containment.
---

# Verify Modn Desktop

Run from the repository root on Windows in an unlocked interactive desktop
session with WebView2 installed.
Use this skill after changes to `apps/modn-desktop/**`, its UI hooks or its
backend. Read [the feature map](features/README.md) before choosing a narrower
journey.

## Run

```powershell
uv run --no-project python .agents/skills/verify-modn-desktop/scripts/verify.py --build
```

`--build` runs both builds first:

```powershell
cargo build -p modn-desktop --features verification --locked
cargo build -p modn-cli --features modn-core/test-instruction-budget --locked
```

Without `--build` the script uses the existing `target/debug` binaries.
Use `--only 3` to run one journey and `--keep` to keep fixtures after a run.

The script imports `uiautomation`. When the interpreter lacks it, it re-runs
itself under `uv run --no-project --with uiautomation`.
It exits 0 only when every journey passes and prints one JSON summary line
with `journeys`, `passed`, `failed`, `assertions` and `seconds`.
Journeys run in the listed order against separate fixtures.

## Prerequisites

- Windows 10 or later with an interactive desktop; the app needs a real
  WebView2 window, so a service session is insufficient.
- `uv` and Python 3 on `PATH`; `rustc` on `PATH` for the play fixture.
- The default cargo target directory. No `CARGO_TARGET_DIR` is set.
- The repository's `buckets/main` folder for the real-CLI journeys.

## What it checks

| # | Journey | Assertions |
|---|---------|------------|
| 1 | Startup detection | a fake Steam library makes Home show `Add Balatro at …`; one click makes it `Open Balatro at …`; the CLI lists it; game bytes unchanged |
| 2 | Install progress | the archive button becomes `Cancel …`, the slow backend completes, the toast offers `Undo` and the backend recorded the write |
| 3 | Cancel before commit | `Cancel` shows `Cancelled. Nothing changed.`; no write, no manager-state or game change |
| 4 | Finishing | during the commit phase the button is disabled and closing shows the native `still working` dialog; the write still completes |
| 5 | Play and details | `Play balatro` starts the fixture exe, which writes `play-marker.txt`; Mod details shows plan and history; `Copy diagnostics` confirms |
| 6 | Blocked plan | the button shows the blocked reason, is disabled, opens no dialog, and leaks no backend text |
| 8 | No CLI output | backend stdout (`invalid fixture response`) and stderr (`fixture refusal`) never appear in the window tree |

Journey 7 (native-code ask) is not reachable in this app revision: installing a
catalog mod needs `--file` for an offline `manual` source, and no backend
emits `APPROVAL_REQUIRED`. See [process.md](features/process.md).

## Fixtures and safety

Every run creates one `modn-desktop-fixture-*` directory and one
`modn-desktop-evidence-*` directory under the system temporary directory.
Inside the fixture:

- `state/` is the `--state-dir` profile.
- `data/` is both `LOCALAPPDATA` and `--data-dir`; `buckets/main` is copied
  here so the official bucket is read offline.
- `appdata/` is `MODN_APPDATA`, the content root for installed mods.
- `steam/` is a fake Steam library with `libraryfolders.vdf` and one
  `appmanifest_*.acf`.
- `archives/` holds the ZIPs the journeys pick, named after the test backend
  scenario each one selects.

`MODN_STORE_LIBRARIES` names a JSON file inside the fixture, because the core
reads that variable as a path.
The app forwards only its allowlist of environment variables, so every journey
passes `--state-dir`, `--data-dir` and `--module-dir` explicitly.

The drivers never read the real `%LOCALAPPDATA%\modn` state, a real Steam, GOG
or Epic library, or a real game.
Every fixture game is a folder under the fixture root; the play executable is a
compiled stub that writes one marker file.
UI Automation is pointed at the Modn window of the launched process id only.
Other windows are never enumerated for interaction, and no mouse movement is
used for app controls.

## Evidence

Read the printed evidence path and its `uia-*-*.txt` snapshots, `launch.json`
and `app-stderr.txt`.
Classify this evidence as **sensitive**: paths and backend output can identify
local data.
Do not upload evidence without reviewing it.

## Cleanup

The driver closes its own window through `WM_CLOSE` and waits for the
process.
If the app does not exit, it kills only the process it started.
It removes only its validated `modn-desktop-fixture-*` directory and keeps the
evidence directory.
Every journey registers its cleanup before the first UI action, so a failed
assertion still closes the app and removes the fixture.

## Limits

The verifier proves UI Automation behaviour and the operations listed above.
It does not cover a real game, Windows 10 hardware, or a real store client.
A native-code ask, a one-click uninstall and both undo paths are unreachable
until the app can install a catalog mod that pins an offline archive.
See [desktop product documentation](../../../docs/desktop.md) for user-facing
limits.
