# Approval policy

A native injector or an executable mod needs one explicit approval per mod.
This file maps the policy of issue #152 to journey 7.

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

## Fixture

Journey 7 registers two games. `balatro` is a content-root game and carries
the native-code and plain-pack releases. `tools-game` uses the permissions
model and carries the executable release with `tools/helper.exe`.

The executable helper is a byte copy of a signed Windows binary. The archive
fixtures come from `catalog_fixture.add_catalog_release`, which writes a
contract v2 release and pins the ZIP by its BLAKE3 digest.

## Content-root limit

The CLI preview ignores explicit executable approvals for a content-root game
(`apps/modn-cli/src/catalog_install.rs:328-331`, `install.rs:70-72`). The
executable ask therefore runs on the `tools-game` fixture.
This is a CLI limit, not an app limit. Do not move that check to `balatro`.

## How to inspect the store

`approval_support.approval_rows` opens the live `state.sqlite` read-only and
reads `native_injector` and `executables` from `mod_approvals`.
The journey checks the same rows after every allow, uninstall and reinstall.
