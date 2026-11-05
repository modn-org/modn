# Getting started

## Recommended first-time setup (MVP)

**Start from a clean, verified game installation.** Use the game client or
platform to verify the game files, and remove manually installed mods using
their original removal instructions before starting with Modn. Keep a backup
of personal saves and settings; Modn does not clean or repair an existing game.

Register the game with `modn game add <game-id> --path "<game>"`, inspect
supported ZIP and 7z packages with `install --dry-run`, and install with explicit
`--yes` after reviewing the plan. There is no default game. Keep subsequent
mod installs and uninstalls under Modn's ownership tracking.

If an installation contains manually installed mods, remove them using their
original uninstall instructions before installing overlapping archive packages.
Modn detects unmanaged content but never claims it; existing files or directories
are not silently overwritten.

The MVP prioritizes conservative ownership checks and refusing ambiguous
filesystem layouts, not support for arbitrary symlink/junction setups or
guaranteed protection against another process changing paths concurrently.
Avoid redirected game/mod directories where practical; use dedicated game
installations and do not run another mod manager against the same files
at the same time.

## Game discovery and manager state

On Windows, omit `--path` to attempt discovery through Steam libraries, Epic Launcher manifests, and common game directories. Custom GOG/Epic/Steam installs may require an explicit path. Registration writes **only manager-owned state** outside the game directory; `scan` and `doctor` never modify game files.

Use `--state-dir <directory>` for an isolated state database. Multiple registered installations require an explicit `--path` for scanning and diagnostics. Only files explicitly recorded in the manager's SQLite ownership table can be reported as managed. Scanning does not claim or authorize uninstalling detected files. Only Modn-installed archives become newly managed.

The CLI has **no default game**. Use `modn game add <game-id> --path <directory>`
to register an installation with its ordinary Lua module; use `--game <game-id>`
with `--path` before registration. When a path or registered installation has
one unambiguous recorded game identity, later commands may select it without
`--game`. Ambiguous registrations fail closed. There are no Cyberpunk aliases,
fallback markers, or game-specific Rust install branches.

## Clean manager state

Modn does not migrate older ownership databases. Installations must use a
fresh manager state database with a versioned schema and archive-backed
package records. If an earlier version of Modn was used, move its state database aside before starting;
do not delete game files or personal saves to reset manager state.

## Next steps

- Run the setup check in [`inventory.md`](inventory.md) before the first install.
- Install mods with [`installing.md`](installing.md).
- Find each command in [`cli.md`](cli.md).

Commands that open manager state retain the selected game installation for the
whole operation. Archive previews with an explicit path do not create state.
