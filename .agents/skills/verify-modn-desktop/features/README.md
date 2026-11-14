# Modn Desktop feature map

Build and run through [SKILL.md](../SKILL.md).
Every journey gets its own fixture and its own app process.
Keep the UI Automation snapshots, backend records and launch record in the
printed evidence directory.
A journey proves only the entry points it exercised.

- [Local archive review](review.md): the native picker, progress, cancel,
  blocked plans and the commit phase.
- [Owned package lifecycle](ownership.md): registration, installation,
  play and the details view.
- [Process lifecycle](process.md): close refusal, backend output containment
  and the unreachable approval path.

`scripts/verify.py` runs the six passing journeys against the real CLI and the
verification test backend.
The earlier GPUI drivers (`lifecycle.py`, `doctor.ps1`) are removed; they
targeted control ids the Tauri window does not have.

The desktop app is Tauri 2 with a WebView2 webview. WebView2 publishes the DOM
accessibility tree through UI Automation, so buttons, links and headings are
found by accessible name. `data-testid` values are not AutomationIds.

Windows 10 hardware, real games and real store clients remain unverified.
