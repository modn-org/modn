# Modn Desktop feature map

Build and run through [SKILL.md](../SKILL.md).
Every journey gets its own fixture and its own app process.
Keep the UI Automation snapshots, backend records and launch record in the
evidence directory. A journey proves only the entry points it exercised.

- [Local archive review](review.md): the native picker, progress, cancel, the
  commit phase and blocked plans (journeys 2, 3, 4, 6 and the 7z picker in 11).
- [Owned package lifecycle](ownership.md): detection, registration, install,
  play, remove, details, undo and the catalog (journeys 1, 5, 9, 10 and 11).
- [Approval policy](approval.md): the native-code and executable ask
  (journey 7).
- [Themes](themes.md): the Theme control, the official fallback, variants,
  local and community themes, the broken-theme diagnostic and the
  digest-mismatch fallback (journey 12).
- [Process lifecycle](process.md): close refusal and CLI-output containment
  (journeys 4 and 8).

`scripts/verify.py` runs the twelve journeys against the real CLI and the
verification test backend. Run `verify.py --list` for the current order and
names.

The desktop app is Tauri 2 with a WebView2 webview.
WebView2 publishes the DOM accessibility tree through UI Automation, so
buttons, links and headings are found by accessible name.
`data-testid` values are not AutomationIds.

Windows 10 hardware, real games and real store clients remain unverified.
