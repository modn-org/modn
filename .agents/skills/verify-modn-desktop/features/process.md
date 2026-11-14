# Process lifecycle

The window stays responsive while the backend runs, refuses to close during
the commit phase, and never shows raw backend output.

## What to check

- P1: the commit phase disables the operation button.
- P2: closing during the commit shows the native `Modn is still working…`
  dialog and the window stays open.
- P3: stdout noise (`invalid fixture response`) and stderr text
  (`fixture refusal`, `fixture stderr noise…`) never appear in the window tree.
- P4: a blocked plan shows the mapped message, not the backend's message.

## How to drive it

Start the app with the verification backend and a disposable state directory
holding `test-backend.marker`.
For P1 and P2, install `hang-after-commit.zip`: the backend emits through
`commit`, ignores cancel for 1.5 seconds and then records the write.
Send `WM_CLOSE` to the owned window with `PostMessageW`, read the extra
top-level window, assert its text, press its `OK` button and wait for the
write.

For P3, set the `mode` file to `invalid-json`, click `Scan this PC for
supported games`, then set it to `failure` and scan again. Each backend call
re-reads the mode file.
The tree is captured after each scan and searched for the sentinel strings.

## Unreachable paths

The native-code ask, the one-click uninstall and both undo paths cannot be
reached in this app revision:

- `apps/modn-desktop/src/app/commands/args.rs` `install_mod_commit` has no
  `--file` argument, so a catalog mod with an offline `manual` source cannot
  install. Every other source kind needs the network.
- `apps/modn-desktop/tests/fixtures/backend.rs` has no scenario that emits
  `APPROVAL_REQUIRED` or `NATIVE_INJECTOR_NOT_ALLOWED`.
- A locally installed archive is not a catalog mod, so no Library tile or Mod
  details action offers `Uninstall` for it.

Add one of those hooks before extending the verifier with journey 7.
