# Local archive review

Users select a registered installation and inspect an archive before approving writes.

## Sub-features

- R1: select a registered installation and enter an archive path.
- R2: inspect and preview proposed files without game or manager-state writes.
- R3: invalidate reviews after source, installation, or registration changes.
- R4: block invalid plans and cancel pending confirmation.

## How to get to it (user POV)

Select an installation button in the sidebar.
Enter an absolute archive path or select Browse... for the native file dialog.
Select Inspect and preview, then Install reviewed archive....
Cancel review clears the pending write authority.

## Driving it with agent-desktop

Run `python .agents/skills/verify-modn-desktop/scripts/verify.py --backend target/debug/modn.exe`.
The script finds the test-game button and the archive field by accessible names.
It checks the report destination and unchanged file/state hashes.
It changes selection and source, removes registration, and verifies refusal before writes.
It cancels the picker after a preview and checks the retained path, invalidated authority, and unchanged bytes.
It captures the review window and resulting accessibility trees.

## Gotchas

The driver exercises native ZIP and 7z selection, typed paths, and native file-dialog cancellation.
Resolve fresh filename and Open controls after each dialog state change.
Native dialog control IDs avoid locale-specific labels.
Programmatic field changes need fresh snapshots before checking disabled buttons.
A review reads metadata and does not certify compatibility.
