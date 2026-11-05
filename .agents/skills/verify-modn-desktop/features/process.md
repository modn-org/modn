# Process lifecycle

Users can observe progress while the fixed CLI backend runs outside the UI thread.

## Sub-features

- P1: remain responsive during backend work.
- P2: refuse busy close and disable active-operation cancellation.
- P3: bound backend runtime and output, then reap the child.
- P4: show backend refusal and invalid-JSON failures.
- P5: distinguish completed writes from a failed inventory refresh.

## How to get to it (user POV)

Select an installation to load its owned packages.
Attempt to close while the operation runs.
Read the visible message and wait for completion before closing.

## Driving it with agent-desktop

Run `lifecycle.py` with the exact app and test-backend paths in [SKILL.md](../SKILL.md).
The explicit fixture backend delays package listing for twelve seconds.
The driver checks responsive accessibility, disabled cancellation, and busy close refusal.
It waits for completion and checks the backend PID is gone.
It then simulates a successful write followed by inventory failure.
The visible status must state that the write completed and clear the stale package selection.
It closes the owned window afterward.
Run the Cargo all-feature tests for the actual 60-second deadline, output overflow, and failure cases.

## Gotchas

The slow backend proves process handling; it does not prove an actual install transaction.
The real-CLI native journey supplies installation and ownership evidence separately.
Dropping a GPUI task cannot cancel a running child or undo an installation.
Forced OS termination remains outside graceful-shutdown guarantees.
