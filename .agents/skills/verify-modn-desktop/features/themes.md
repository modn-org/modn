# Themes

The desktop applies theme API v1 themes from the `main` bucket, the local
registry and a community bucket, and it refuses a theme that breaks the
allowlist. This page maps those checks to the `themes` journey and to the
fixtures in `scripts/theme_fixture.py`.

## What the journey proves

- T1: `modn theme list` shows the official, local and community themes with
  their sources, and shows `fixture-broken` only as a diagnostic naming
  `forbidden-position-fixed`. `modn theme check` of the protected theme exits
  1 naming `protected-region`. The bucket and list commands still exit 0.
- T2: the launched app shows `fixture-official` in the Theme control with the
  source label `main bucket`.
- T3: the `bold` variant is chosen and the button grows. `modn theme resolve`
  reports variant `bold` and origin `selected`.
- T4: the local theme and then the community theme are chosen. The source
  labels read `local registry` and the community bucket id.
- T5: the control lists the diagnostic for the broken theme and does not offer
  it. After one byte of the registered local copy changes, a relaunch shows
  the fallback and `digest-mismatch`.
- T6: `Default theme` restores the base height, and `modn theme resolve`
  reports `theme: null`.
- T7: with a theme selected, `modn theme check` still refuses the protected
  theme with `protected-region`. The install review is not opened (see
  Not proven).

## Geometry method

UI Automation publishes bounding rectangles, not computed styles. Every
fixture theme sets `min-height` on the stable hook `.modn-theme-button` to a
distinct value between 56 px and 88 px, so the button height orders the
themes. The journey compares that order, not absolute pixels, because DPI
scaling changes pixels. `theme_fixture.expected_order` returns the order and
`theme_fixture.taller` compares two rectangles with a one-pixel tolerance.

## Fixtures

All fixtures use the game id `balatro` and theme API 1.
`bucket_fixture.write_main_bucket` writes the `main` bucket.
`bucket_fixture.write_bucket` writes the `community` bucket.

| Theme | Height | Location |
| --- | --- | --- |
| `fixture-official` | 56 px | `main` bucket; variants `bold` at 72 px and `quiet` at 64 px |
| `fixture-local` | 80 px | plain folder, registered by the journey |
| `fixture-community` | 88 px | `community` bucket |
| `fixture-broken` | none | `community` bucket; CSS `.modn-theme-button { position: fixed; }` |
| `fixture-protected` | none | plain folder; CSS `.modn-approval { display: none; }` |

Each theme folder holds `theme.json`, `theme.css` and one
`theme.<variant>.css` per listed variant. The files use LF line endings, so a
digest covers the same bytes as a published bucket.

## Not proven

- No real game is used.
- No Windows 10 hardware is tested.
- Computed CSS is not visible to UI Automation, so the journey reads no
  colour, font or hidden element from the rendered webview.
- Step T7 does not open the install review: reaching it needs more than 40
  lines, so the review's protected regions and controls are not proven at
  runtime. The `protected-region` rule rests on the `modn theme check`
  refusal in step T1.
- Absolute pixel heights are not asserted, only their order.
