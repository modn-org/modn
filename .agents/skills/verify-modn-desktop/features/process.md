# Process lifecycle

The window stays responsive while the backend runs, refuses to close during
the commit phase, and never shows raw backend output.

## What to check

- P1: the commit phase disables the operation button (journey 4).
- P2: closing during the commit shows the native
  `Modn is still working. Wait for the operation to finish.` dialog and the
  window stays open (journey 4).
- P3: stdout noise (`invalid fixture response`) and stderr text
  (`fixture refusal`) never appear in the window tree (journey 8).
- P4: a blocked plan shows the mapped message, not the backend's message
  (journey 6).
- P5: a finished write shows its toast (journeys 2 and 4).

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

## Cleanup

Every journey registers its cleanup before the first UI action.
Cleanup answers only `OK` or `Cancel` on a close dialog. It never answers a
destructive Yes/No confirmation on the user's behalf.
