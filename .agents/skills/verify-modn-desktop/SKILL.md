---
name: verify-modn-desktop
description: "Verify the Modn Desktop Tauri window through Windows UI Automation on disposable fixtures: startup detection, install progress and cancel, finishing, the local archive lifecycle, blocked plans, native-code and executable approval, CLI-output containment, play, remove, catalog update and 7z."
---

# Verify Modn Desktop

Run this verifier from the repository root on Windows 10 or later, in an
unlocked interactive desktop session with WebView2 installed.
Use it after changes to `apps/modn-desktop/**`, its UI, its backend or its
approval policy. Read [the feature map](features/README.md) before you choose a
narrower journey.

## Run

```powershell
uv run --no-project python .agents/skills/verify-modn-desktop/scripts/verify.py --build
```

Options:

- `--build` runs `cargo build -p modn-desktop --features verification --locked`
  and `cargo build -p modn-cli --features modn-core/test-instruction-budget
  --locked` first.
- `--list` prints each journey number and name in run order, then exits.
- `--only <names or numbers>` selects journeys; separate several with a comma.
- `--keep` keeps the fixture directory of the run and prints every kept path.
- `--app PATH`, `--cli PATH` and `--test-backend PATH` replace one built binary
  each.
- `--selftest` checks the dialog helper, failure tracking, evidence reporting
  and skill front matter against fakes, then exits.

Without `--build`, the verifier reads `modn-desktop.exe`, `modn.exe` and
`modn-desktop-test-backend.exe` from `$CARGO_TARGET_DIR/debug` when
`CARGO_TARGET_DIR` is set, otherwise from `target/debug`.
Use a `modn.exe` built with `modn-core/test-instruction-budget` for `--cli`.
That feature supplies the fixture store and content-root overrides. A CLI
without them would read the real Steam library and install into real game
folders. The verifier searches the CLI binary for the `MODN_APPDATA` and
`MODN_STORE_LIBRARIES` names. It exits with status 2 before any journey runs
when a name is missing.

When the interpreter lacks a required package, the script re-runs itself under
`uv run --no-project --with uiautomation --with blake3 --with pyyaml python`.
It runs the selected journeys in order, each against a fresh fixture.
It exits 0 only when every selected journey passes.
Each journey prints its status and an `evidence: <path>` line.
The last line is one JSON summary with `status`, `journeys`, `passed`, `failed`,
`assertions`, `seconds` and `results`.
Each result carries the same `evidence` paths as a list.

## Prerequisites

- Windows 10 or later with an unlocked interactive desktop and WebView2. A
  service session has no window to drive.
- `uv` and Python 3 on `PATH`. `uv` fetches `uiautomation`, `blake3` and
  `pyyaml` when the interpreter lacks them.
- 7-Zip (`7z`) on `PATH` or in `%ProgramFiles%\7-Zip` for the `catalog`
  journey.
- No Rust compiler for the journeys. `--build` needs `cargo`.

## Journeys

| # | Journey | User action and visible result | CLI or manager-state check | File-bytes check |
|---|---------|-------------------------------|----------------------------|------------------|
| 1 | `detection` | Home shows `Add Balatro at <path> to Modn`; one click makes it `Open Balatro at <path>` | `modn game list` names `balatro` | the game folder is unchanged after detection and after registration |
| 2 | `progress` | the local archive picker accepts `slow.zip`; the button becomes `Cancel`; the toast `Install done` offers `Undo` | the test backend records the write in `write.json` | none |
| 3 | `cancel` | `Cancel` before the commit shows `Cancelled. Nothing changed.` | no write and no new manager state | the state and game folders are unchanged |
| 4 | `finishing` | the commit phase disables the button; closing shows the native `Modn is still working. Wait for the operation to finish.` dialog; the write then completes | `write.json` appears after the dialog is dismissed | none |
| 5 | `lifecycle` | archive install, toast undo, catalog install, uninstall and history undo show `Install done`, `Undo done` and `Uninstall done`; details report `No approval needed.` | CLI list and history agree on five operations | each undo and uninstall restores the earlier content bytes |
| 6 | `blocked` | the disabled button reads `Modn blocked this operation. Open details for the reason.` and opens no dialog | no backend text reaches the window tree | none |
| 7 | `approval` | an ask reads `This change needs your approval.` with `Allow native code through the native injector.` or `Allow these executables:`; `Decline` writes nothing; a release that adds native code asks again | `state.sqlite` table `mod_approvals` stores one row per mod; CLI lists the version | an ask and a `Decline` change no game or content bytes; `Allow` writes the archive bytes |
| 8 | `no-cli-output` | two scans run while the backend prints invalid JSON and then fails; the window stays open | `invalid fixture response` and `fixture refusal` never appear | none |
| 9 | `play` | `Play balatro` starts the fixture executable; details show history; `Copy diagnostics` shows `Diagnostics copied.` | the started process id is found by image path and stopped | the game folder is unchanged |
| 10 | `remove` | `Remove from Modn` opens the native Yes/No dialog; `No` keeps the registration and `Yes` unregisters it | `modn game list` lists and then omits `balatro` | the game folder is unchanged after both answers |
| 11 | `catalog` | a catalog install, an update to `Update available`, a toast undo and a genuine 7z install and uninstall each take one click | CLI lists version 1.0.0, then 2.0.0, then 1.0.0 | installed bytes match each release; undo and uninstall restore the earlier bytes |

Journey 7 proves the policy of issue #152. A native-code or executable mod asks
once per mod. A new release that adds native code asks again.
Adoption and forced cleanup are not offered by the app.
Journey 6 proves that a blocked plan shows its reason and never asks.

Journeys 1, 2, 3 and 8 cover the Milestone 6 exit gate: detection at
startup, install progress and cancel, cancel before the commit with the game
unchanged, and no CLI output in the window.

## Fixtures and safety

Every fixture is a fresh `modn-desktop-fixture-*` directory under the system
temporary directory. Its evidence lives in a separate
`modn-desktop-evidence-*` directory there.

Inside the fixture:

- `state/` is the `--state-dir` profile.
- `data/` is `LOCALAPPDATA` and the `--data-dir`; it receives the generated
  bucket contract v2 `main` bucket when a journey needs it.
- `appdata/` is `MODN_APPDATA`, the content root for installed mods.
- `steam/` is a fake Steam library with one manifest for the journeys that
  detect a store installation.
- `archives/` holds the ZIP and 7z files the journeys pick.

The verifier never reads the user's real `%LOCALAPPDATA%\modn`, a real game or a
real store library. The app forwards only its environment allowlist, so each
journey passes `--state-dir`, `--data-dir` and, when the fixture has modules,
`--module-dir` explicitly.

A fixture that detects, registers or starts a game uses a byte copy of a signed
Windows binary. Other fixtures write a short marker for an executable that no
journey starts. Nothing is compiled, so no unsigned stub exists for Windows
Defender to quarantine. The `play` journey copies `mstsc.exe`; it briefly shows
a Remote Desktop Connection window and stops only the process id that `play`
started. An earlier compiled stub was flagged as `Trojan:Win32/Bearfoos.A!ml`,
a false positive. Never add a Defender exclusion.

UI Automation addresses only the Modn window of the process id the verifier
started. Other windows are never enumerated for interaction.

## Evidence

The evidence directory is always kept. It holds the `uia-*.txt` snapshots,
`launch.json`, `app-stderr.txt` and the journey JSON files.
Each journey prints its evidence path as an `evidence: <path>` line, and its
result in the JSON summary repeats the path in `evidence`.
Record those paths, whether or not the run uses `--keep`.
With `--keep`, the fixture directory stays too and the run prints its path.
Classify this evidence as **sensitive**: paths and backend output can identify
local data. Review it before you upload it.

## Cleanup

Each journey registers its cleanup before the first UI action. A passing
journey ends with `App.close`: it asks the window to close, answers only OK or
Cancel on an extra dialog, waits up to 45 seconds, and kills the process only
when the wait times out. A failed journey runs the registered `App.kill`, which
terminates only the process the verifier started, by pid, at once. Cleanup
removes only the validated `modn-desktop-fixture-*` directory, unless `--keep`
is set. A failed assertion still kills the app and cleans up.

## Limits

The verifier proves the window and the listed operations on disposable
fixtures. It does not cover a real game, Windows 10 hardware or a real store
client.

The CLI preview ignores explicit executable approvals for a content-root game
(`apps/modn-cli/src/catalog_install.rs:328-331`, `install.rs:70-72`). The
executable ask therefore runs on the `tools-game` fixture.

The [MCP verifier](../verify-modn-mcp/SKILL.md) covers progress notifications
and cancellation separately. See [desktop.md](../../../docs/desktop.md) for
user-facing limits.
