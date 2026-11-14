# Owned package lifecycle

Home shows detected and registered installations. A detected card registers
the game in one click; a registered card opens the game's Library.

## What to check

- O1: a fake Steam library makes Home show `Add Balatro at <path> to Modn`.
- O2: one click changes that card to `Open Balatro at <path>` and the CLI
  `game list` reports the installation.
- O3: detection and registration leave the fixture game folder byte-identical.
- O4: `Play balatro` launches the fixture executable, which writes
  `play-marker.txt` beside itself.
- O5: Mod details shows the file plan and operation history, and
  `Copy diagnostics` confirms with a toast.

## Fixture

The journey copies `buckets/main` into the disposable data directory, writes
`steam/steamapps/libraryfolders.vdf` plus `appmanifest_2379780.acf`, and puts
`Balatro.exe` under `steam/steamapps/common/Balatro`.
For play, the fixture game folder holds a real PE compiled by `rustc` at the
module's declared executable path.
That stub writes one marker file and exits, so no real game ever starts and the
store URI branch is never taken.

`MODN_STORE_LIBRARIES` names a JSON file (`{"steam_roots": [ … ]}`); the core
reads that variable as a path, not as inline JSON.

## Gotchas

Home falls back to a capitalized id, so the test backend's `fixture` game shows
as `Open Fixture at …`.
The `--module-dir` option is only passed when the fixture has modules; the
real-CLI journeys read the copied main bucket instead.
Mod details links are hyperlinks, not buttons.
