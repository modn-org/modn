# Approval policy

A native injector or an executable mod needs one explicit approval per mod.
This file maps the policy of issue #152 to journeys 7 and 12.

## What to check

- A1: a native-code release asks with `This change needs your approval.`,
  `Allow native code through the native injector.`, `Allow` and `Decline`.
- A2: the first ask writes no manager state, no package and no file.
- A3: `Decline` closes the ask and still writes nothing.
- A4: `Allow` installs the archive and stores one `mod_approvals` row in
  `state.sqlite` after the write commits.
- A5: the stored approval survives uninstall and suppresses the next ask for
  the same capabilities.
- A6: an executable release asks with `Allow these executables:` and lists
  each game-relative path; its approval is stored and remembered.
- A7: a later release that adds native code asks again; a later release with
  covered capabilities does not ask.
- A8: adoption and forced cleanup are not offered by the app, so no journey
  exercises them.
- A9: on a content-root game, an executable release asks the same question,
  lists its content-relative path, `Decline` writes nothing, `Allow` installs
  into the content root, and the stored approval suppresses the next ask
  (journey 12).

## Fixture

Journey 7 registers two games. `balatro` is a content-root game and carries
the native-code and plain-pack releases. `tools-game` uses the permissions
model and carries the executable release with `tools/helper.exe`.

Journey 12 registers only `balatro`. Its `root-exe-pack` release declares
`RootPack/tools/helper.exe`, the content-relative destination of the archive
entry. The Balatro fixture plans both archive files below the `RootPack`
folder, so the install lands in the content root `appdata/Balatro/Mods`, and
the core accepts the declaration because the destination is a `.exe` below a
`tools` folder. The journey checks the installed bytes there.

The executable helper is a byte copy of a signed Windows binary. The archive
fixtures come from `catalog_fixture.add_catalog_release`, which writes a
contract v2 release and pins the ZIP by its BLAKE3 digest.

## Content-root coverage

Journey 7 keeps the native-code case and the executable case on `tools-game`.
Journey 12 covers the executable case on the content-root game, through the
desktop window and the manager-state approval the app stores.

## How to inspect the store

`approval_support.approval_rows` opens the live `state.sqlite` read-only and
reads `native_injector` and `executables` from `mod_approvals`.
The journey checks the same rows after every allow, uninstall and reinstall.
