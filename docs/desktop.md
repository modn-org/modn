# Windows desktop app

Modn Desktop is the Windows window over the `modn` CLI. It lists game
installations, shows a mod library and runs one-click operations. The
[CLI](cli.md) remains the only writer of game files.

## Architecture

The app is a Tauri 2 program in the root Cargo workspace at
`apps/modn-desktop`. Its manifest names `tauri` and `tauri-plugin-dialog` for
Windows targets only. `cargo build -p modn-desktop --locked` builds
`target/debug/modn-desktop.exe`.

`tauri.conf.json` sets `frontendDist` to `ui`, so the executable embeds the
frontend. The UI is plain HTML, CSS and ES modules in `apps/modn-desktop/ui/`.
It uses no bundler, no package manager and no Node toolchain. `build.rs`
declares the 19 commands to `tauri-build`.

On other platforms the entry point prints that Modn Desktop requires Windows
and exits 1. Linux builds compile no webview library.

## Security

The webview runs under a strict content security policy. It allows only
`'self'` for scripts, styles and fonts. It allows images from `'self'` and
`data:`. It connects only to `ipc:` and `http://ipc.localhost`. Object, base,
frame and form sources are `'none'`.

The main window capability grants `core:default` and the app's own 19 commands.
`build.rs`, `capabilities/default.json` and `ui/js/api.js` list the same names,
and the command tests compare those lists. The app loads one plugin,
`tauri-plugin-dialog`, for native pickers and messages. No filesystem, shell or
HTTP plugin is present. The webview can call no file, process or network API of
its own.

Rust shows every native dialog. The folder picker and archive picker return a
path to Rust, and the webview never supplies or edits the chosen folder.

`src/backend/progress.rs` holds the only process-spawn helper. `Backend::call`
and `Backend::call_with_progress` both use it. The helper runs the configured
`modn.exe` with a fixed prefix, `--json --state-dir <directory>`, and the
arguments that `src/app/commands/args.rs` builds. It clears the inherited
environment, then forwards only these values when they are set:

- `SystemRoot`;
- `TEMP` and `TMP`, only as absolute Windows paths;
- `LOCALAPPDATA`, unless `--data-dir` replaces it;
- `MODN_CACHE`, `MODN_STORE_LIBRARIES` and `MODN_APPDATA`;
- `MODN_GAME_MODULES_DIR`, unless `--module-dir` replaces it.

The helper never invokes a shell, never searches `PATH` and starts the child
without a console window. A command handler passes it only values that the
handler has checked.

A command returns the CLI JSON value, or an error object with a stable `code`
and a short `message`. CLI output never reaches the webview.

## Configure and launch

Every configuration path must be absolute. `--backend` must name an existing
`.exe` file. `--state-dir` selects the manager state and has no default. The
optional `--data-dir` sets the backend's `LOCALAPPDATA` for the official
bucket. The optional `--module-dir` selects a development module directory.
The optional `--evidence-dir` records backend arguments, exit codes and output.
Treat evidence as sensitive local data.

```powershell
$app = (Resolve-Path target/debug/modn-desktop.exe).Path
$backend = (Resolve-Path target/debug/modn.exe).Path
& $app --backend $backend --state-dir C:\ModnDesktop\state --data-dir C:\ModnDesktop\data
```

The app needs a trusted game module for each game it detects or launches. Load
one with the CLI first. A startup configuration error appears in a native error
dialog, and the app exits 1.

## Screens

The app has one window and three screens. The rail links to Home, and each
screen uses a hash route.

### Home

Home runs detection each time it opens, including at launch. It lists
registered installations and detected game installations that Modn does not
manage. A detected card appears in grayscale and carries the store name.

One click on a detected card registers that installation. Registration writes
manager state only and changes no game file. A failure reason appears on the
card. **Scan** repeats registration listing and detection.

**Add** opens a menu of supported game ids. It then shows the native folder
picker from Rust and registers the chosen folder.

**Remove from Modn** appears only when the installation owns no managed
package. A native confirmation states that Modn only unregisters the
installation and that every game file stays on disk.

### Library

The Library shows one game installation. It lists bucket entries and managed
packages in a grid. Each tile shows the mod name, its bucket and version, and
its state. A native code mod also shows a `Native code` tag. The name opens Mod
details.

**All**, **Installed** and **Available** filter the grid. The search field
filters the grid at once and queries the bucket after a short pause. A selector
appears when the game has several installations.

**Play** launches the selected installation. **Install archive** opens the
native archive picker and installs the chosen ZIP or 7z file. **Recent** lists
the operations of the game, newest first, each with **Undo**. A tile offers
**Install**, **Update** or **Uninstall** for the state it shows.

### Mod details

Mod details shows one mod of one installation. The header shows the name, the
version and bucket, the description and the state.

The main action is the one that fits the state: **Install**, **Update** or
**Uninstall**. Without an installation, or without a bucket entry for an
uninstalled mod, the button stays disabled with a short reason.

**Details** is a collapsed section. It reads the install plan only when it
opens. It lists each file operation as `Write` or `Remove`, the blockers with
their short reasons, the file count and the approval note. It never blocks the
main action.

**History** lists the operations of this mod, newest first, each with **Undo**.
**Copy diagnostics** runs `modn doctor` through the backend and copies the
pretty-printed JSON report.

## One-click operations

Install, update, uninstall, undo and play take one click. Install archive takes
one click and one file choice. Install archive runs the dry run in that same
click. Catalog install, update and uninstall pass `--yes`, and the CLI builds
and checks its plan inside that call. A stale screen cannot force a write.

A blocked plan shows its reason on the button and disables it. A write starts
on the click. Blocked codes include `PLAN_BLOCKED`, `CONFLICT`,
`DEPENDENCY_MISSING`, `ALREADY_INSTALLED`, `DIGEST_MISMATCH`,
`PENDING_TRANSACTION` and `ARCHIVE_INVALID`.

A native injector or an executable mod asks once per mod. Install, update and
archive ask in place, with **Allow** and **Decline**. **Allow** resends the
operation with `--allow-native-injector` or one `--allow-executable <path>` per
uncovered path. Each path must be game-relative and cannot start with `-`.
Undo reports the same refusal as a message instead of an ask.

The CLI stores an approval per game installation, bucket and mod, and only
after the write commits. A later release that adds native code or an executable
path is uncovered again, so the app asks again.

Adoption and owned cleanup are not offered in one click. The argument builders
emit no adoption flag and no `--force`. These CLI commands keep their own
confirmation.

Errors show one short sentence, chosen from the stable code. Raw CLI text is
never shown. An unknown code falls back to a generic sentence that points to
Copy diagnostics.

## Progress, cancel and time limits

The button shows the running phase. A download with known bytes shows a
percentage; other phases show their name. The button is **Cancel** while the
operation runs.

The app writes one `cancel` line on stdin. The CLI stops at its next safe
point, and a cancel before the commit leaves the game unchanged. A cancelled
run shows `Cancelled. Nothing changed.`

After the CLI reports the `commit` event, the button becomes **Finishing...**
and cannot be cancelled. The app then waits for the CLI to finish or roll back.

A write call has an inactivity timeout of 120 seconds before the commit. Any
output resets the timer. On expiry the app sends `cancel`, waits 15 seconds,
then kills the process. After the commit event no inactivity kill applies. A
call that times out returns `BACKEND_TIMEOUT`.

Read-only calls keep their 60-second deadline and a 1 MiB bound per output
stream. The game registration calls (activate, add and remove a game) also use
this deadline; they write manager state only. A call that exceeds either bound
is killed and reported.

The archive check accepts a regular ZIP or 7z file up to 4 GiB. The app hashes
the archive and runs the dry run. It checks that plan and the digest again
before the write.

The window refuses to close while a backend call runs, and a native message
says that Modn is still working. A killed write can leave a pending
transaction. Use the CLI doctor and recovery commands before a retry.

## Undo

Each finished operation shows a toast such as `Install done`. The toast offers
**Undo** for eight seconds. **History** in Mod details and **Recent** in the
Library offer the same action on older operations. The app has no separate
History page.

## Build and checks

Run these commands from the repository root:

```powershell
cargo build -p modn-desktop --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --locked
cargo test -p modn-desktop --all-features --locked
```

The `verification` feature adds a fixture backend and enables the backend
contract test. A normal build omits the fixture backend. The UI tests read the
shipped HTML, CSS and JavaScript files. Windows CI runs the desktop contract
tests with all features.

## Verification

`cargo test -p modn-desktop --all-features --locked` runs the backend contract
tests. They cover argument vectors, refusals, bounded output, the environment
allowlist, progress order, cancel before and after the commit, and the
inactivity timeout.

The [desktop verifier skill](../.agents/skills/verify-modn-desktop/SKILL.md)
checks the running window on Windows. Run it from the repository root:

```powershell
uv run --no-project python .agents/skills/verify-modn-desktop/scripts/verify.py --build
```

## Limits

The app does not offer adoption, forced cleanup, recovery, pin or unpin, bucket
update, module management, Nexus authentication or cache commands. Use the CLI
for each of them.

It has one window, one light theme and no built-in updater. Bundling is
inactive, so the project ships no installer. The release workflow publishes
`modn-desktop.exe` as a verifier artifact.

The app shows managed package IDs and does not infer ownership from scanning. A
game installation with managed packages cannot be removed from Modn. A mod
that is not a bucket entry opens Mod details only after it is installed. A
download shows a percentage only when the CLI reports a byte total.

A backend that does not answer and a forced process termination can leave a
pending transaction. Read the visible failure, then use the CLI doctor and
recovery commands.

## Licensing

Modn Desktop uses Tauri 2 and its official crates, which are licensed under
Apache-2.0 OR MIT. The root `Cargo.lock` records the full dependency graph. See
the [third-party notices](../apps/modn-desktop/THIRD_PARTY_NOTICES.md). A binary
distributor must audit the locked graph and keep the required notices.
