# Windows desktop MVP

Modn Desktop uses GPUI Kit 0.9.0 and its included Component layer.
It is a root Cargo workspace member and uses the root Cargo.lock.
The manifest restricts GPUI Kit and GPUI pre to Windows.
On other platforms, the executable reports that Windows is required and exits 1.
Linux core and CLI checks do not compile GPUI or require GPU system libraries.

## Build and prerequisites

Use Windows 10 or newer with an interactive desktop session.
Install stable Rust 1.92 or newer, VS 2022 C++ Build Tools, Windows SDK, and CMake.
Native verification also requires 7-Zip on PATH.
The verified host uses Rust 1.99 and Windows SDK 10.0.26100.0.
No nightly toolchain is required.

Run these commands from the repository root:

```powershell
cargo build -p modn-cli --locked
pwsh -NoProfile -File .agents/skills/verify-modn-desktop/scripts/doctor.ps1 -Backend target/debug/modn.exe
cargo build --workspace --all-features --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --locked
cargo test -p modn-desktop --all-features --locked
```

The `verification` feature adds a disposable test backend executable.
Normal builds omit that executable.
The timeout contract test takes about 60 seconds.

## Configure and launch

Choose a fixed, trusted `modn.exe` and an explicit manager state directory.
Use the same manager state for CLI registration and desktop operations.
Pre-register each installation with `modn --state-dir <state> game add <game-id> --path <installation>`.
Install or register its trusted Lua game module through the CLI first.
The desktop app does not register games or approve executable Lua policy.

Example launch, after registration:

```powershell
$app = (Resolve-Path target/debug/modn-desktop.exe).Path
$backend = (Resolve-Path target/debug/modn.exe).Path
& $app --backend $backend --state-dir C:\ModnDesktop\state --data-dir C:\ModnDesktop\data
```

All configuration paths must be absolute.
`--backend` must name an existing executable.
`--state-dir` selects manager state; it has no default.
`--data-dir` optionally sets the backend's `LOCALAPPDATA` for the official `main` bucket.
The module registry and the other bucket registrations, with their module trust records, follow `--state-dir`.
Omit it to use the CLI's standard data location.
`--module-dir` optionally selects an explicit development module directory.
`--evidence-dir` optionally records backend arguments, exit codes, and output.
Treat evidence as sensitive local data.
Startup configuration failures appear in an error window.

The app clears inherited backend environment variables.
It passes `SystemRoot` and the explicitly configured data and module locations.
It never forwards arbitrary command arguments.
It never invokes a shell or searches PATH for its backend.

## Operate

1. Select a registered installation in the sidebar.
2. Choose a local ZIP or 7z with **Browse...**, or enter its absolute path.
3. Select **Inspect and preview**.
4. Review the proposed source files, destinations, sizes, and blockers.
5. Select **Install reviewed archive...**, then **Confirm install**.
6. Select an owned package, then **Uninstall selected package...** and **Confirm uninstall**.

**Cancel review** clears the pending review and confirmation before a write starts.
Changing the archive or game installation clears that authority too.
A blocked, empty, or malformed plan cannot enable installation.
The app rechecks the source hash, current registration, and installation plan before installation.
Uninstall checks the current owned-package listing and passes no force flags.
The CLI retains authority over transactions, ownership, path validation, rollback, and recovery.
Manual files remain outside uninstall authority.

Backend work runs outside the UI thread.
Controls remain disabled during an operation.
Closing a busy window shows a message and keeps the window open.
An active installation cannot be cancelled safely.
Each CLI process has a 60-second deadline and a 1 MiB output limit per stream.
Archive hashing accepts regular files up to 4 GiB.
The app kills and waits for a backend that exceeds a limit.
A stopped write can leave a pending transaction.
Read the visible failure, then use the CLI's doctor and recovery commands before retrying.
If a write succeeds but inventory refresh fails, the app states that the write completed.
It clears the stale package selection and asks for diagnostics and refresh.
An OS crash or forced process termination cannot guarantee graceful shutdown.

## Verification

The [verification skill](../.agents/skills/verify-modn-desktop/SKILL.md) defines the native proof.
Its driver creates fresh game, state, data, module, and archive directories.
It chooses a test backend explicitly and never uses the user's default manager state.
It records native accessibility actions and cropped app screenshots.
Independent CLI reads and file hashes check the resulting ownership and bytes.
Cleanup closes only the recorded app window and removes only the unique fixture root.
Evidence remains outside that root.

## Limits and integration

This MVP manages local archives and pre-registered installations.
Catalog browsing, downloads, updates, pins, recovery controls, and executable approval remain CLI operations.
It performs no automatic network requests.
It does not deploy REDmod or certify game compatibility.
The UI shows managed package IDs; it does not infer ownership from scanning.
The desktop verifier uses a synthetic Lua game module, ZIP archives, and a genuine 7z fixture.
It selects ZIP and 7z fixtures through the native picker and checks the selected paths.
It also checks native dialog cancellation with Windows control IDs.
Both archive formats exercise real CLI preview, confirmed installation, ownership, and strict uninstall.
Modified owned bytes cause refusal; manual bytes remain intact.
Windows 10 hardware validation remains unclaimed.
Real game installations have not been used for verification.

Root Windows gates cover the app and existing backend contracts.
The Windows CI job also runs the desktop contracts with all features.
Linux dependency resolution excludes GPUI; native desktop behavior requires Windows.
The root dev profile disables debug information to retain the existing GPUI build configuration.
An explicit CARGO_TARGET_DIR changes all binary paths shown above.
Use that directory for the app, CLI, test backend, doctor, and native drivers.
The official bucket uses its separate repository and the existing pinned submodule.

## Framework and licensing

The manifest pins `gpui-kit = "=0.9.0"`.
The manifest also pins `gpui-pre = "=0.3.11"`.
The app uses `application().with_assets(assets::Assets).run`, `init`, `Render`, Kit buttons, and Kit `open_window`.
`open_window` supplies the Root component.
There is no alternate UI framework.

GPUI Kit and its included layers use Apache-2.0.
Bundled Lucide icons use ISC.
The app loads installed Segoe UI and Consolas fonts without redistributing them.
See [third-party notices](../apps/modn-desktop/THIRD_PARTY_NOTICES.md) and its license files.
A binary distributor must audit transitive dependency notices before distribution.

Framework references:

- https://github.com/longbridge/gpui-kit/tree/v0.9.0
- https://gpui-kit.com/docs/installation/
- https://gpui-kit.com/docs/getting-started/
- https://gpui-kit.com/docs/task/
