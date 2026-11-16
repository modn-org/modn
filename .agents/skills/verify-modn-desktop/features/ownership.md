# Owned package lifecycle

Home shows detected and registered installations. A detected card registers
the game in one click; a registered card opens the game's Library.

## What to check

- O1: a fake Steam library makes Home show `Add Balatro at <path> to Modn`
  (journey 1).
- O2: one click changes that card to `Open Balatro at <path>` and the CLI
  `game list` reports the installation (journey 1).
- O3: detection and registration leave the fixture game folder byte-identical
  (journey 1).
- O4: a local archive install writes the archive bytes and the CLI lists the
  package; toast undo restores the earlier bytes (journey 5).
- O5: a catalog install, uninstall and history undo agree with the CLI list
  and history (journeys 5 and 11).
- O6: `Play balatro` launches the fixture executable (journey 9).
- O7: Mod details shows the file plan and operation history (journeys 5 and
  9), and `Copy diagnostics` confirms with `Diagnostics copied.` (journey 9).
  That toast needs a focused document, so the journey holds the window in the
  foreground for the click and fails with the holder name when another app
  keeps it.
- O8: `Remove from Modn` opens a native Yes/No dialog; `No` keeps the
  registration, `Yes` unregisters it, and the game folder stays unchanged
  (journey 10).

## Fixture

The real-CLI journeys register a fixture game with
`modn game add <id> --path <folder>`. Detection and remove also write
`steam/steamapps/libraryfolders.vdf` plus `appmanifest_2379780.acf`, and put
`Balatro.exe` under `steam/steamapps/common/Balatro`.

For play, the fixture game folder holds a byte copy of the signed Windows
binary `driverquery.exe`. That console program shows no window under the
hidden console of `modn play`, and the journey fails when a visible window of
the fixture image appears. The journey finds the process id by image path and
stops only that process id, so no real game ever starts and the store URI
branch is never taken.

`MODN_STORE_LIBRARIES` names a JSON file (`{"steam_roots": [ … ]}`); the core
reads that variable as a path, not as inline JSON.

`MODN_APPDATA` sets the content root for installed mods. The CLI reads it only
when built with `modn-core/test-instruction-budget`, so the verifier builds
that CLI and does not use a release `modn.exe`.

## Gotchas

Home falls back to a capitalized id, so the test backend's `fixture` game shows
as `Open Fixture at …`.
The `--module-dir` option is only passed when the fixture has modules; the
real-CLI journeys read the generated main bucket instead.
Mod details links are hyperlinks, not buttons.
