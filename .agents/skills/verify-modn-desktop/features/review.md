# Local archive review

The Library header installs a local ZIP through the native picker, shows
progress on the same button, and turns that button into Cancel.

## What to check

- R1: `Install a local archive in <game>` opens the Windows file dialog.
- R2: while the backend reports progress the button reads `Cancel …` and stays
  enabled.
- R3: before the commit, Cancel ends with `Cancelled. Nothing changed.` and
  writes nothing.
- R4: after the commit the button is disabled until the write finishes.
- R5: a blocked plan shows its short reason on the button and opens no dialog.

## How to drive it

1. Start the app with `--backend <modn-desktop-test-backend.exe>`, a
   disposable `--state-dir` containing `test-backend.marker`, and a `mode`
   file.
2. Open the `Open Fixture at …` Home card.
3. Click `Install a local archive in fixture`.
4. Fill the native dialog and confirm it.

The file dialog is a common item dialog. Its file name field and Open button
do not publish AutomationIds, so `uia.set_file_dialog` uses the documented
Win32 dialog control ids 1148 and 1 on the dialog window owned by the launched
process, then posts `BM_CLICK` to Open.

The test backend selects its progress scenario from the archive path:
`slow.zip`, `hang-before-commit.zip`, `hang-after-commit.zip` and `refuse.zip`.
The `mode` file therefore stays `complete`, so the package listing does not
sleep.

## Gotchas

The percentage in `Cancel · 50 %` is drawn from the button text, while
`aria-label` stays `Cancel Install archive`. UI Automation reports the
accessible name, so the journey asserts the Cancel affordance, not the
percentage. The same applies to `Finishing...`.
The eight-second toast lifetime limits how long `Undo` is available.
