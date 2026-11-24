# Game themes

A theme is one CSS contract with optional variants.
It can change layout, sizing, fonts and colors.
It cannot run code, load remote resources or change a destructive confirmation.
Themes need no trust step: selecting a theme is the consent.

## Status

This page is the specification of theme API v1. The Status table says what Modn implements today.

| Component | State |
| --- | --- |
| Theme folder | Implemented in modn-core and the theme commands |
| `theme.json` | Implemented in modn-core and the theme commands |
| Limits and digest | Implemented in modn-core and enforced by the theme commands |
| CSS rule engine | Implemented in modn-core and used by the theme commands |
| Variants | Implemented in modn-core and the theme commands |
| Sources and selection | Implemented in modn-core and the theme commands |
| CLI theme commands | Implemented: `check`, `register`, `list`, `remove`, `select`, `reset` and `resolve` |
| Desktop theme control | Implemented; it needs a `modn` build with the theme commands |
| Desktop hooks | Implemented: hooks and protected regions in the UI |
| Official theme | Staged in `examples/themes/cyberpunk-2077/night-city/`; publication in `modn-org/main` is a follow-up in that repository |

## Theme folder

A bucket holds themes at this path, relative to the bucket root:

```text
themes/<game-id>/<theme-id>/theme.json
themes/<game-id>/<theme-id>/theme.css
themes/<game-id>/<theme-id>/theme.<variant>.css      (one per listed variant)
themes/<game-id>/<theme-id>/assets/...               (PNG, WebP, WOFF2 only)
```

Every bucket kind may contain `themes/`.
A bucket may contain only themes: no mods, modules or games.

`theme.json` and `theme.css` are required.

The theme directory accepts these top-level entries:

- `theme.json`, `theme.css` and the listed `theme.<variant>.css` files.
- The `assets/` directory.
- The optional ignored files are `README.md`, `LICENSE` and `NOTICE.md`.

Text files in a theme use LF line endings: `theme.json`, `*.css`, `NOTICE.md`, `README.md` and `LICENSE`.
A CRLF checkout changes the digest, because the digest covers file bytes.
A bucket that publishes themes should mark `themes/** text eol=lf` in `.gitattributes`.
It should mark its `.png` and `.woff2` assets `binary` there.

Any other top-level entry is the diagnostic `unexpected-file`.

`assets/` holds asset files.
An asset path matches `assets/[a-z0-9._-]+(/[a-z0-9._-]+)*`.
It is lowercase and contains no `..`.
The extension decides the asset type, and the first bytes must confirm it:

| Extension | Type | Magic bytes |
| --- | --- | --- |
| `.png` | PNG | `89 50 4E 47` |
| `.webp` | WebP | `RIFF....WEBP` |
| `.woff2` | WOFF2 | `wOF2` |

A wrong extension or a wrong signature is the diagnostic `asset-type`.

## theme.json

`theme.json` is a UTF-8 JSON object.

| Field | Type | Required | Rule |
| --- | --- | --- | --- |
| `schema_version` | integer | yes | Must be `1`. |
| `id` | string | yes | Theme id. Equals the theme directory name. |
| `game` | string | yes | Game id. Equals the parent directory name. |
| `name` | string | yes | 1-64 characters, no control characters. |
| `theme_api` | integer | yes | Must be `1`. |
| `variants` | array of strings | yes | Zero to 16 variant ids, no duplicates. |
| `icon` | string | no | Path under `assets/` ending in `.png` or `.webp`. |

Unknown fields are rejected with `unknown-manifest-field`.

A theme id, a game id and a variant id use the same grammar:

```text
[a-z0-9]+(-[a-z0-9]+)*
```

An id has at most 64 characters.
It is lowercase ASCII.
Each hyphen separates two non-empty groups of `a-z` and `0-9`.

A complete example:

```json
{"schema_version":1,"id":"night-city","game":"cyberpunk-2077","name":"Night City","theme_api":1,"variants":["neon","noir"],"icon":"assets/icon.png"}
```

## Limits

The validator and the theme loader enforce these limits.
The names below are the Rust constant names.

| Constant | Scope | Limit |
| --- | --- | --- |
| `MAX_THEME_CSS_BYTES` | `theme.css` | 256 KiB (262144 bytes) |
| `MAX_THEME_VARIANT_CSS_BYTES` | One `theme.<variant>.css` file | 128 KiB (131072 bytes) |
| `MAX_THEME_MANIFEST_BYTES` | `theme.json` | 16 KiB (16384 bytes) |
| `MAX_THEME_ASSET_BYTES` | One file under `assets/` | 2 MiB (2097152 bytes) |
| `MAX_THEME_ASSETS_BYTES` | All files under `assets/` together | 8 MiB (8388608 bytes) |
| `MAX_THEME_ASSETS` | Number of files under `assets/` | 64 |
| `MAX_THEME_VARIANTS` | Number of entries in `variants` | 16 |
| `MAX_IGNORED_FILE_BYTES` | One ignored top-level file | 1 MiB (1048576 bytes) |
| `MAX_IGNORED_FILES_BYTES` | All ignored top-level files together | 4 MiB (4194304 bytes) |
| `MAX_THEME_ENTRIES` | Directory entries one inspection visits | 1024 |

The ignored top-level files are `README.md`, `LICENSE` and `NOTICE.md`.
An ignored file above its limit, the ignored files above their total or an inspection above the entry budget is `theme-too-large`.

The digest is BLAKE3 over the theme directory.
It covers each regular file in byte-wise sorted relative-path order: the path bytes, a NUL, the length as 8 little-endian bytes, then the content.
The stored digest is 64 lowercase hex characters with no prefix, as in the trusted game module registry.
A JSON field named `blake3` holds the bare hex digest; the `register` answer spells the same value as `digest` with a `blake3:` prefix.
A registered theme is hashed again on every load.
A mismatch is the diagnostic `digest-mismatch`.

## CSS rules

A theme stylesheet is a sequence of style rules, `@media` rules and `@font-face` rules.
Everything else is rejected.

The allowed top-level rules are:

- Style rules.
- `@media`, which may contain style rules only.
- `@font-face`, whose `src` `url()` points into `assets/` and ends in `.woff2`.

Any other at-rule is `forbidden-at-rule`.
An `@media` query uses only these media features: `width`, `height`, `min-width`, `max-width`, `min-height`, `max-height`, `orientation`, `prefers-color-scheme` and `prefers-reduced-motion`.
It may also use `and`, `not`, `only`, `screen` and `all`, and the level 4 range syntax with numbers and lengths.
Any other media feature, or any other token, is `forbidden-at-rule`.
`@import` has its own rule id, `forbidden-import`.

The property allowlist applies to style rules.
Inside `@font-face`, only the `src` URL is checked.

### Property allowlist

A declaration property must appear in this table.
The match is exact, except where a group names a prefix.

| Group | Properties |
| --- | --- |
| Layout | `display`, `visibility`, `position`, `top`, `right`, `bottom`, `left`, `inset` and `inset-*`, `z-index`, `flex` and `flex-*`, `grid` and `grid-*`, `gap`, `row-gap`, `column-gap`, `order`, `align-*`, `justify-*`, `place-*`, `overflow*` |
| Box model | `width`, `height`, `min-width`, `min-height`, `max-width`, `max-height`, `margin` and `margin-*`, `padding` and `padding-*`, `box-sizing`, `aspect-ratio` |
| Typography | `font` and `font-*`, `line-height`, `letter-spacing`, `word-spacing`, `text-align`, `text-transform`, `text-decoration` and `text-decoration-*`, `text-overflow`, `text-indent`, `white-space`, `text-shadow` |
| Color and background | `color`, `opacity`, `background`, `background-color`, `background-image`, `background-position`, `background-size`, `background-repeat`, `background-clip` |
| Borders | `border` and `border-*`, `outline` and `outline-*`, `border-radius` and its longhands |
| Shadows | `box-shadow`, `text-shadow` |
| Transitions | `transition` and `transition-*` |

`float` is not allowed.
The following are also not allowed: `animation*`, `transform*`, `filter`, `backdrop-filter`, `pointer-events`, `cursor` and `content`.
A custom property is allowed only when its name starts with `--modn-`.
Any other custom property is `forbidden-custom-property`.

These properties must use a literal value, not `var()`: `position`, `display`, `visibility`, `opacity`, `overflow`, `overflow-x`, `overflow-y`, `z-index`, `top`, `right`, `bottom`, `left`, `inset`, `inset-block`, `inset-inline`, `width`, `height`, `min-width`, `min-height`, `max-width`, `max-height`, `font-size` and `line-height`.
A `var()` in one of them is `forbidden-var`.
`var()` stays allowed in every other property, including colour properties and unrelated `--modn-*` values.

### Rule ids

A refusal names the rule and the file.
It names the line when the validator knows it.

| Id | Rejects | Bad example | Good example |
| --- | --- | --- | --- |
| `forbidden-import` | An `@import` rule. | `@import "extra.css";` | `.modn-card { color: #ffffff; }` |
| `forbidden-at-rule` | Any at-rule other than `@media` and `@font-face`. | `@supports (display: grid) { .modn-card { display: grid; } }` | `@media (max-width: 900px) { .modn-card { display: block; } }` |
| `remote-url` | A `url()` with a scheme, `//`, `data:` or a leading `/`. | `background-image: url("https://example.com/a.png");` | `background-image: url(assets/a.png);` |
| `url-outside-assets` | A `url()` outside `assets/`, with a rejected extension, or with no file. | `background-image: url("../a.png");` | `background-image: url(assets/a.png);` |
| `forbidden-content` | The `content` property. | `.modn-card { content: "new"; }` | `.modn-card { opacity: 0.9; }` |
| `forbidden-position-fixed` | `position: fixed`. | `.modn-rail { position: fixed; }` | `.modn-rail { position: sticky; }` |
| `forbidden-var` | A `var()` in a property that must use a literal value. | `.modn-rail { position: var(--modn-position, fixed); }` | `.modn-card { padding: var(--modn-space-m); }` |
| `forbidden-important` | The `!important` flag. | `.modn-card { color: #ffffff !important; }` | `.modn-card { color: #ffffff; }` |
| `unknown-property` | A property outside the allowlist. | `.modn-card { transform: scale(1.1); }` | `.modn-card { opacity: 0.8; }` |
| `forbidden-custom-property` | A custom property not named `--modn-*`. | `.modn-app { --accent: #ffffff; }` | `.modn-app { --modn-color-primary: #ffffff; }` |
| `token-out-of-range` | A size or opacity token outside its allowed range. | `:root { --modn-font-size-body-s: 0; }` | `:root { --modn-font-size-body-s: 13px; }` |
| `forbidden-selector` | A selector outside the selector grammar. | `div .modn-card { color: #ffffff; }` | `.modn-app .modn-card { color: #ffffff; }` |
| `unknown-hook` | A `.modn-*` class or `data-*` attribute not in the stable tables. | `.modn-missing { color: #ffffff; }` | `.modn-card { color: #ffffff; }` |
| `protected-region` | A non-allowed property on a protected hook, a reachable attribute or a context hook. | `.modn-review { display: none; }` | `.modn-review { color: #ffffff; }` |
| `protected-color-not-opaque` | A translucent colour in a protected region, a translucent `--modn-color-*` token, or a translucent `color` on `:root` or `html`. | `.modn-approval { background-color: rgba(0, 0, 0, 0.5); }` | `.modn-approval { background-color: #000000; }` |
| `css-syntax` | A syntax error. | `.modn-card { color: #ffffff;` | `.modn-card { color: #ffffff; }` |

An allowed `url()` has the canonical form `url(assets/<path>)`.
The pure CSS rule engine checks the path only.
The theme loader checks that the file exists.

An invalid `id`, `game` or variant id string in `theme.json` is `manifest-invalid`.
`inspect_theme_dir` does not compare the directory name with the manifest.
Bucket scanning reports `id-mismatch` in two cases.
The path `themes/<game>/<id>` differs from `game` and `id` in `theme.json`.
A directory name under `themes/` that is not a valid id also reports `id-mismatch`.

### Selector grammar

A selector is allowed only when all of these conditions hold:

1. Every compound selector contains a stable hook.
   A stable hook is a listed `.modn-*` class or a listed `[data-*]` attribute with an exact `=` match.
2. `:root` and `html` are allowed only for rules that declare only `--modn-*` custom properties, `color` and `font-*`.
3. The selector uses no `*`, no bare type selector, no `:has()`, no `:is()` and no `:where()`.
4. The selector uses no pseudo-element, including `::before` and `::after`.
5. The selector uses only the descendant, child, adjacent and sibling combinators.
6. Pseudo-classes are limited to `:hover`, `:focus-visible`, `:active`, `:disabled`, `:first-child` and `:last-child`.
   `:not()` is allowed only with stable hooks.

A violation is `forbidden-selector`.
A `.modn-*` class or `data-*` attribute outside the stable tables is `unknown-hook`.

### Other diagnostics

The theme loader and the registry report these structural diagnostics.
They are not CSS rule ids.

| Id | Meaning |
| --- | --- |
| `manifest-invalid` | `theme.json` is missing, unreadable or malformed, including an invalid `id`, `game` or variant id string. |
| `unknown-manifest-field` | `theme.json` holds a field outside the table. |
| `id-mismatch` | A bucket path `themes/<game>/<id>` differs from `game` and `id` in `theme.json`, or a bucket directory name is not a valid id. |
| `missing-css` | `theme.css` is absent. |
| `missing-variant-file` | A listed variant has no `theme.<variant>.css`. |
| `unexpected-file` | A top-level entry is not allowed, including an unlisted variant CSS file. |
| `asset-type` | An asset extension or magic signature does not match. |
| `asset-too-large` | One asset exceeds 2 MiB. |
| `too-many-assets` | `assets/` holds more than 64 files. |
| `theme-too-large` | A CSS file, an ignored file, the ignored-file total, the entry budget, a registration copy or a bucket scan exceeds its limit. |
| `duplicate-theme-id` | A later duplicate `(game, id)` pair was skipped. |
| `digest-mismatch` | A registered theme no longer matches its digest. |
| `theme-missing` | The selected theme no longer exists. |

## Stable hooks

Paths in these tables are relative to `apps/modn-desktop/ui/`; files outside that directory have a `../` prefix.
The UI has 21 shell classes, 17 home classes, 34 library classes, 15 details classes, 12 install-review classes, 7 approval classes, 4 toast classes, 2 message classes, 18 theme-control classes, 12 `data-*` attributes and 43 `--modn-*` custom properties.

A selector may use only hooks listed on this page. An unlisted `.modn-*` class is the rule `unknown-hook`.
An unlisted `data-*` attribute has the same rule.

### Classes

| Class | Group | Defined in | Used in | Purpose |
| --- | --- | --- | --- | --- |
| `.modn-actions` | Shell | `css/base.css` | `js/details.js`, `js/home.js`, `js/library.js` | Header action button row |
| `.modn-app` | Shell | `css/base.css` | `index.html` | Two-column application frame |
| `.modn-button` | Shell | `css/base.css` | `js/details.js`, `js/home.js`, `js/library.js`, `js/op-button.js`, `js/home/card.js`, `js/home/dom.js`, `js/library/recent.js` | Base action button |
| `.modn-button--danger` | Shell | `css/base.css` | — | Destructive action button |
| `.modn-button--quiet` | Shell | `css/base.css` | `js/home.js`, `js/home/card.js`, `js/library/recent.js` | Low-emphasis action button |
| `.modn-button--secondary` | Shell | `css/base.css` | `js/details.js`, `js/home.js` | Secondary action button |
| `.modn-card` | Shell | `css/base.css` | `js/home.js`, `js/library.js`, `js/details/history.js`, `js/library/tile.js` | Raised content card |
| `.modn-card--empty` | Shell | `css/base.css` | `js/home.js`, `js/library.js` | Card that fills an empty state |
| `.modn-empty` | Shell | `css/base.css` | — | Empty-state text block |
| `.modn-grid` | Shell | `css/base.css` | `js/home.js`, `js/library.js` | Responsive card grid |
| `.modn-link` | Shell | `css/base.css` | `js/details.js`, `js/library.js` | Inline text link |
| `.modn-list` | Shell | `css/base.css` | `js/library.js` | Vertical list container |
| `.modn-main` | Shell | `css/base.css` | `index.html` | Main view region |
| `.modn-muted` | Shell | `css/base.css` | `js/details.js`, `js/home.js`, `js/library.js`, `js/details/history.js`, `js/home/card.js`, `js/library/recent.js`, `js/library/tile.js` | Muted secondary text |
| `.modn-rail` | Shell | `css/base.css` | `index.html` | Left navigation rail |
| `.modn-rail__link` | Shell | `css/base.css` | `index.html` | Rail home link |
| `.modn-row` | Shell | `css/base.css` | `js/library.js`, `js/details/history.js` | Horizontal content row |
| `.modn-row__text` | Shell | `css/base.css` | `js/details.js`, `js/home.js`, `js/library.js`, `js/details/history.js` | Text block inside a row |
| `.modn-tag` | Shell | `css/base.css` | `js/library/tile.js` | Small metadata tag |
| `.modn-view` | Shell | `css/base.css` | `index.html`, `js/app.js`, `js/details.js`, `js/home.js`, `js/library.js` | Routed screen container |
| `.modn-view__header` | Shell | `css/base.css` | `js/details.js`, `js/home.js`, `js/library.js` | Screen title and actions header |
| `.modn-game-card` | Home | `css/home.css` | `js/home/card.js` | Detected game card |
| `.modn-game-card__badge` | Home | `css/home.css` | `js/home/card.js` | Detected state badge |
| `.modn-game-card__body` | Home | `css/home.css` | `js/home/card.js` | Card text body |
| `.modn-game-card__error` | Home | `css/home.css` | `js/home/card.js` | Card error message |
| `.modn-game-card__hint` | Home | `css/home.css` | `js/home/card.js` | Card action hint |
| `.modn-game-card__initials` | Home | `css/home.css` | `js/home/card.js` | Fallback game initials |
| `.modn-game-card__media` | Home | `css/home.css` | `js/home/card.js` | Card artwork area |
| `.modn-game-card__meta` | Home | `css/home.css` | `js/home/card.js` | Installation metadata line |
| `.modn-game-card__path` | Home | `css/home.css` | `js/home/card.js` | Installation path line |
| `.modn-game-card__remove` | Home | `css/home.css` | `js/home/card.js` | Remove game button |
| `.modn-game-card__surface` | Home | `css/home.css` | `js/home/card.js` | Card hover surface layer |
| `.modn-game-card__title` | Home | `css/home.css` | `js/home/card.js` | Game title line |
| `.modn-home__actions` | Home | `css/home.css` | `js/home.js` | Home header action row |
| `.modn-home__empty` | Home | `css/home.css` | `js/home.js` | Home empty-state panel |
| `.modn-home__grid` | Home | `css/home.css` | `js/home.js` | Detected game card grid |
| `.modn-menu` | Home | `css/home.css` | `js/home/menu.js` | Card overflow menu |
| `.modn-menu__item` | Home | `css/home.css` | `js/home/menu.js` | Overflow menu item |
| `.modn-input` | Library | `css/library.css` | `js/library.js` | Text input field |
| `.modn-library` | Library | `css/library.css` | `js/library.js` | Library screen container |
| `.modn-library__actions` | Library | `css/library.css` | `js/library.js` | Library action row |
| `.modn-library__ask` | Library | `css/library.css` | `js/library.js` | Library approval ask host |
| `.modn-library__empty` | Library | `css/library.css` | `js/library.js` | Library empty-state text |
| `.modn-library__grid` | Library | `css/library.css` | `js/library.js` | Mod tile grid |
| `.modn-library__hero` | Library | `css/library.css` | `js/library.js` | Game hero header |
| `.modn-library__installation` | Library | `css/library.css` | `js/library.js` | Installation summary block |
| `.modn-library__label` | Library | `css/library.css` | `js/library.js` | Summary field label |
| `.modn-library__search` | Library | `css/library.css` | `js/library.js` | Search input wrapper |
| `.modn-library__title` | Library | `css/library.css` | `js/library.js` | Game title heading |
| `.modn-library__toolbar` | Library | `css/library.css` | `js/library.js` | Tabs and search toolbar |
| `.modn-op` | Library | `css/library.css` | `js/op-button.js` | Operation button group |
| `.modn-op__bar` | Library | `css/library.css` | `js/op-button.js` | Download progress fill |
| `.modn-op__button` | Library | `css/library.css` | `js/op-button.js` | Operation action button |
| `.modn-op__progress` | Library | `css/library.css` | `js/op-button.js` | Download progress track |
| `.modn-recent` | Library | `css/library.css` | `js/library/recent.js` | Recent operations list |
| `.modn-recent__entry` | Library | `css/library.css` | `js/library/recent.js` | Recent operation row |
| `.modn-recent__menu` | Library | `css/library.css` | `js/library/recent.js` | Recent operation menu |
| `.modn-recent__text` | Library | `css/library.css` | `js/library/recent.js` | Recent operation text |
| `.modn-select` | Library | `css/library.css` | `js/library.js` | Dropdown select field |
| `.modn-tab` | Library | `css/library.css` | `js/library.js` | Single tab button |
| `.modn-tab--active` | Library | `css/library.css` | `js/library.js` | Selected tab button |
| `.modn-tabs` | Library | `css/library.css` | `js/library.js` | Tab button row |
| `.modn-tile` | Library | `css/library.css` | `js/library/tile.js` | Mod tile card |
| `.modn-tile__actions` | Library | `css/library.css` | `js/library/tile.js` | Mod action button row |
| `.modn-tile__art` | Library | `css/library.css` | `js/library/tile.js` | Mod artwork area |
| `.modn-tile__ask` | Library | `css/library.css` | `js/library/tile.js` | Tile approval ask host |
| `.modn-tile__body` | Library | `css/library.css` | `js/library/tile.js` | Mod tile text body |
| `.modn-tile__initials` | Library | `css/library.css` | `js/library/tile.js` | Fallback mod initials |
| `.modn-tile__name` | Library | `css/library.css` | `js/library/tile.js` | Mod name button |
| `.modn-tile__state` | Library | `css/library.css` | `js/library/tile.js` | Mod state badge |
| `.modn-tile__tags` | Library | `css/library.css` | `js/library/tile.js` | Mod tag row |
| `.modn-tile__warning` | Library | `css/library.css` | `js/library/tile.js` | Mod warning badge |
| `.modn-details` | Details | `css/details.css` | `js/details.js` | Mod details screen container |
| `.modn-details__actions` | Details | `css/details.css` | `js/details.js` | Details action row |
| `.modn-details__ask` | Details | `css/details.css` | `js/details.js` | Details approval ask host |
| `.modn-details__back` | Details | `css/details.css` | `js/details.js` | Back to library button |
| `.modn-details__description` | Details | `css/details.css` | `js/details.js` | Mod description text |
| `.modn-details__footer` | Details | `css/details.css` | `js/details.js` | Details footer block |
| `.modn-details__header` | Details | `css/details.css` | `js/details.js` | Details header block |
| `.modn-details__heading` | Details | `css/details.css` | `js/details/history.js` | Section heading text |
| `.modn-details__meta` | Details | `css/details.css` | `js/details.js` | Mod metadata line |
| `.modn-details__state` | Details | `css/details.css` | `js/details.js` | Details state badge |
| `.modn-details__title` | Details | `css/details.css` | `js/details.js` | Mod title heading |
| `.modn-details__titles` | Details | `css/details.css` | `js/details.js` | Title and metadata stack |
| `.modn-history` | Details | `css/details.css` | `js/details/history.js` | Undo history list |
| `.modn-history__entry` | Details | `css/details.css` | `js/details/history.js` | Single history entry |
| `.modn-history__label` | Details | `css/details.css` | `js/details/history.js` | History entry label |
| `.modn-review` | Install review | `css/base.css` | `js/details/plan.js` | Install review frame |
| `.modn-review__summary` | Install review | `css/details.css` | `js/details/plan.js` | File plan summary toggle |
| `.modn-review__body` | Install review | `css/details.css` | `js/details/plan.js` | File plan body |
| `.modn-review__approval` | Install review | `css/details.css` | `js/details/plan.js` | Plan approval note |
| `.modn-review__muted` | Install review | `css/details.css` | `js/details/plan.js` | Muted review text |
| `.modn-review__status` | Install review | `css/details.css` | `js/details/plan.js` | Review status message |
| `.modn-review__status--error` | Install review | `css/details.css` | `js/details/plan.js` | Review error status message |
| `.modn-review__list` | Install review | `css/details.css` | `js/details/plan.js` | File operation list |
| `.modn-review__row` | Install review | `css/details.css` | `js/details/plan.js` | Single file operation row |
| `.modn-review__path` | Install review | `css/details.css` | `js/details/plan.js` | Operation destination path |
| `.modn-review__tag` | Install review | `css/details.css` | `js/details/plan.js` | Write or remove tag |
| `.modn-review__blocker` | Install review | `css/details.css` | `js/details/plan.js` | Plan blocker message |
| `.modn-approval` | Approval | `css/library.css` | `js/library/ask.js` | Inline approval ask panel |
| `.modn-approval__question` | Approval | `css/library.css` | `js/library/ask.js` | Approval question line |
| `.modn-approval__list` | Approval | `css/library.css` | `js/library/ask.js` | Executable path list |
| `.modn-approval__path` | Approval | `css/library.css` | `js/library/ask.js` | Single executable path |
| `.modn-approval__actions` | Approval | `css/library.css` | `js/library/ask.js` | Allow and decline row |
| `.modn-approval__button` | Approval | `css/library.css` | `js/library/ask.js` | Allow button |
| `.modn-approval__button--quiet` | Approval | `css/library.css` | `js/library/ask.js` | Decline button |
| `.modn-toast` | Toast | `css/base.css` | `js/toast.js` | Single toast message |
| `.modn-toast__action` | Toast | `css/base.css` | `js/op-button.js`, `js/toast.js` | Undo action inside toast |
| `.modn-toast__message` | Toast | `css/base.css` | `js/toast.js` | Toast text area |
| `.modn-toasts` | Toast | `css/base.css` | `index.html`, `js/op-button.js`, `js/toast.js` | Toast stack container |
| `.modn-status` | Messages | `css/base.css` | `index.html`, `js/app.js`, `js/details.js`, `js/library.js`, `js/details/history.js`, `js/home/card.js`, `js/home/dom.js`, `js/library/recent.js` | Status and progress message |
| `.modn-status--error` | Messages | `css/base.css` | `js/app.js`, `js/details.js`, `js/library.js`, `js/details/history.js`, `js/home/card.js`, `js/home/dom.js`, `js/library/recent.js` | Error state status message |
| `.modn-theme-actions` | Theme control | `css/theme.css` | `js/theme/panel.js` | Theme panel action row |
| `.modn-theme-button` | Theme control | `css/theme.css` | `js/theme/panel.js` | Theme panel toggle button |
| `.modn-theme-control` | Theme control | `css/theme.css` | `js/theme/panel.js` | Theme control root |
| `.modn-theme-current` | Theme control | `css/theme.css` | `js/theme/panel.js` | Active theme block |
| `.modn-theme-diagnostic` | Theme control | `css/theme.css` | `js/theme/panel.js` | Single theme diagnostic line |
| `.modn-theme-diagnostics` | Theme control | `css/theme.css` | `js/theme/panel.js` | Theme diagnostic list |
| `.modn-theme-item` | Theme control | `css/theme.css` | `js/theme/panel.js` | Theme list item |
| `.modn-theme-list` | Theme control | `css/theme.css` | `js/theme/panel.js` | Theme choice list |
| `.modn-theme-mark` | Theme control | `css/theme.css` | `js/theme/panel.js` | Chosen theme mark |
| `.modn-theme-name` | Theme control | `css/theme.css` | `js/theme/panel.js` | Theme name |
| `.modn-theme-panel` | Theme control | `css/theme.css` | `js/theme/panel.js` | Theme control panel |
| `.modn-theme-pick` | Theme control | `css/theme.css` | `js/theme/panel.js` | Theme choice button |
| `.modn-theme-pick--chosen` | Theme control | `css/theme.css` | `js/theme/panel.js` | Selected theme choice button |
| `.modn-theme-select` | Theme control | `css/theme.css` | `js/theme/panel.js` | Variant dropdown |
| `.modn-theme-shell` | Theme control | `css/theme.css` | `index.html` | Theme shell header |
| `.modn-theme-source` | Theme control | `css/theme.css` | `js/theme/panel.js` | Theme source label |
| `.modn-theme-status` | Theme control | `css/theme.css` | `js/theme/panel.js` | Theme status line |
| `.modn-theme-variant` | Theme control | `css/theme.css` | `js/theme/panel.js` | Variant field |

The Theme control hooks are listed in the Classes table above under the group `Theme control`.

### Data attributes

| Attribute | Defined in | Used in | Purpose |
| --- | --- | --- | --- |
| `[data-theme]` | `index.html` | `css/tokens.css` | Select light or dark tokens |
| `[data-testid]` | `index.html`, `js/details.js`, `js/details/history.js`, `js/details/plan.js`, `js/home.js`, `js/home/card.js`, `js/home/dom.js`, `js/home/menu.js`, `js/library.js`, `js/library/ask.js`, `js/library/recent.js`, `js/library/tile.js`, `js/op-button.js`, `js/toast.js` | `../tests/ui_details.rs`, `../tests/ui_library.rs` | Test and verifier lookup key |
| `[data-state]` | `js/home/card.js`, `js/library/tile.js`, `js/op-button.js` | `css/home.css`, `css/library.css`, `js/library/tile.js` | Style operation and card state |
| `[data-busy]` | `js/home/card.js` | `css/home.css`, `js/home/card.js` | Mark a card busy |
| `[data-operation]` | `js/details/plan.js` | `css/details.css` | Distinguish write and remove rows |
| `[data-game-id]` | `js/details.js`, `js/home/card.js` | — | Game identity on screen elements |
| `[data-path]` | `js/home/card.js` | — | Installation path on card elements |
| `[data-mod-id]` | `js/details.js`, `js/library/tile.js` | — | Mod identity on screen elements |
| `[data-operation-id]` | `js/details/history.js` | — | Operation identity on history rows |
| `[data-installed]` | `js/library/tile.js` | — | Installed state on a mod tile |
| `[data-code]` | `js/details/plan.js` | — | Blocker code on a plan row |
| `[data-modn-protected]` | `js/details/plan.js`, `js/library/ask.js` | `../tests/js/protected-regions.test.mjs`, `../tests/ui_hooks.rs` | Mark a protected region root |

### Custom properties

All 43 `--modn-*` properties are defined in `css/tokens.css`.
The "used in" column lists the consumers in the component stylesheets.
Every `--modn-color-*` token a theme sets must be opaque, because protected regions read it.

These tokens accept only a literal value inside a range:

- `--modn-font-size-*` takes a length of at least 10px, or at least 0.625rem or 0.625em.
- `--modn-font-line-*` takes a unitless number of at least 1, or a length of at least 10px.
- `--modn-space-*` takes a length from 0 to 64px, or from 0 to 4rem.
- `--modn-opacity-disabled` takes a number from 0.4 to 1.

A `var()`, a keyword or an unparsable value is `token-out-of-range`.

| Property | Defined in | Used in | Purpose |
| --- | --- | --- | --- |
| `--modn-color-border` | `css/tokens.css` | `css/base.css`, `css/details.css`, `css/home.css`, `css/library.css` | Border and divider colour |
| `--modn-color-danger` | `css/tokens.css` | `css/base.css`, `css/details.css` | Danger and error colour |
| `--modn-color-on-primary` | `css/tokens.css` | `css/base.css` | Text on primary fill |
| `--modn-color-primary` | `css/tokens.css` | `css/base.css`, `css/details.css`, `css/home.css`, `css/library.css` | Primary accent colour |
| `--modn-color-primary-hover` | `css/tokens.css` | `css/base.css` | Primary accent hover colour |
| `--modn-color-success` | `css/tokens.css` | — | Success state colour |
| `--modn-color-surface` | `css/tokens.css` | `css/base.css`, `css/details.css`, `css/home.css`, `css/library.css` | Base surface colour |
| `--modn-color-surface-raised` | `css/tokens.css` | `css/base.css`, `css/details.css`, `css/home.css`, `css/library.css` | Raised surface colour |
| `--modn-color-text` | `css/tokens.css` | `css/base.css`, `css/details.css`, `css/home.css`, `css/library.css` | Primary text colour |
| `--modn-color-text-muted` | `css/tokens.css` | `css/base.css`, `css/details.css`, `css/home.css`, `css/library.css` | Secondary text colour |
| `--modn-font-body` | `css/tokens.css` | `css/base.css` | Body font stack |
| `--modn-font-heading` | `css/tokens.css` | `css/base.css`, `css/details.css`, `css/home.css`, `css/library.css` | Heading font stack |
| `--modn-font-line-body-l` | `css/tokens.css` | — | Body line height, large |
| `--modn-font-line-body-m` | `css/tokens.css` | `css/base.css`, `css/library.css` | Body line height, medium |
| `--modn-font-line-body-s` | `css/tokens.css` | `css/base.css`, `css/details.css`, `css/home.css`, `css/library.css` | Body line height, small |
| `--modn-font-line-display` | `css/tokens.css` | `css/home.css` | Display line height |
| `--modn-font-line-heading-l` | `css/tokens.css` | `css/base.css`, `css/details.css`, `css/library.css` | Heading line height, large |
| `--modn-font-line-heading-m` | `css/tokens.css` | `css/base.css`, `css/details.css`, `css/home.css`, `css/library.css` | Heading line height, medium |
| `--modn-font-line-heading-s` | `css/tokens.css` | `css/base.css`, `css/details.css`, `css/library.css` | Heading line height, small |
| `--modn-font-line-label-m` | `css/tokens.css` | `css/base.css`, `css/details.css`, `css/home.css`, `css/library.css` | Label line height, medium |
| `--modn-font-line-label-s` | `css/tokens.css` | `css/base.css`, `css/details.css`, `css/home.css`, `css/library.css` | Label line height, small |
| `--modn-font-size-body-l` | `css/tokens.css` | — | Body size, large |
| `--modn-font-size-body-m` | `css/tokens.css` | `css/base.css`, `css/library.css` | Body size, medium |
| `--modn-font-size-body-s` | `css/tokens.css` | `css/base.css`, `css/details.css`, `css/home.css`, `css/library.css` | Body size, small |
| `--modn-font-size-display` | `css/tokens.css` | `css/home.css` | Display font size |
| `--modn-font-size-heading-l` | `css/tokens.css` | `css/base.css`, `css/details.css`, `css/library.css` | Heading size, large |
| `--modn-font-size-heading-m` | `css/tokens.css` | `css/base.css`, `css/details.css`, `css/home.css`, `css/library.css` | Heading size, medium |
| `--modn-font-size-heading-s` | `css/tokens.css` | `css/base.css`, `css/details.css`, `css/library.css` | Heading size, small |
| `--modn-font-size-label-m` | `css/tokens.css` | `css/base.css`, `css/details.css`, `css/home.css`, `css/library.css` | Label size, medium |
| `--modn-font-size-label-s` | `css/tokens.css` | `css/base.css`, `css/details.css`, `css/home.css`, `css/library.css` | Label size, small |
| `--modn-opacity-disabled` | `css/tokens.css` | `css/base.css`, `css/home.css` | Disabled control opacity |
| `--modn-radius-l` | `css/tokens.css` | `css/base.css`, `css/details.css`, `css/home.css`, `css/library.css` | Large corner radius |
| `--modn-radius-m` | `css/tokens.css` | `css/base.css`, `css/home.css`, `css/library.css` | Medium corner radius |
| `--modn-radius-s` | `css/tokens.css` | `css/base.css`, `css/details.css`, `css/home.css`, `css/library.css` | Small corner radius |
| `--modn-space-2xl` | `css/tokens.css` | `css/base.css`, `css/details.css`, `css/library.css` | Spacing step, extra extra large |
| `--modn-space-2xs` | `css/tokens.css` | `css/base.css`, `css/details.css`, `css/home.css`, `css/library.css` | Spacing step, extra extra small |
| `--modn-space-3xl` | `css/tokens.css` | `css/base.css`, `css/home.css`, `css/library.css` | Spacing step, 3xl |
| `--modn-space-4xl` | `css/tokens.css` | `css/base.css`, `css/details.css`, `css/home.css`, `css/library.css` | Spacing step, 4xl |
| `--modn-space-l` | `css/tokens.css` | `css/base.css`, `css/home.css`, `css/library.css` | Spacing step, large |
| `--modn-space-m` | `css/tokens.css` | `css/base.css`, `css/details.css`, `css/home.css`, `css/library.css` | Spacing step, medium |
| `--modn-space-s` | `css/tokens.css` | `css/base.css`, `css/details.css`, `css/home.css`, `css/library.css` | Spacing step, small |
| `--modn-space-xl` | `css/tokens.css` | `css/base.css`, `css/details.css`, `css/library.css` | Spacing step, extra large |
| `--modn-space-xs` | `css/tokens.css` | `css/base.css`, `css/details.css`, `css/home.css`, `css/library.css` | Spacing step, extra small |

## Protected regions

A protected region is a subtree that a theme may recolor and restyle only with colours, font weight and font style.
Two regions exist:

- `data-modn-protected="install-review"` marks the plan and review section of the install flow.
  The markup is in `apps/modn-desktop/ui/js/details/plan.js`.
  The style rules are in `apps/modn-desktop/ui/css/details.css` lines 56-139 and `apps/modn-desktop/ui/css/base.css` lines 133-141.
- `data-modn-protected="approval"` marks the inline approval ask.
  The markup is in `apps/modn-desktop/ui/js/library/ask.js:26-69`.
  The style rules are in `apps/modn-desktop/ui/css/library.css:218-293`.

The protected class prefixes are `.modn-review*` and `.modn-approval*`.

The safety properties of both regions are literals in `apps/modn-desktop/ui/css/protected.css`, inside `@layer protected`.
The layer order puts `protected` last, so a theme cannot override them.

### Protected hooks

| Class | Region | Purpose |
| --- | --- | --- |
| `.modn-review` | Install review | Install review frame |
| `.modn-review__summary` | Install review | File plan summary toggle |
| `.modn-review__body` | Install review | File plan body |
| `.modn-review__approval` | Install review | Plan approval note |
| `.modn-review__muted` | Install review | Muted review text |
| `.modn-review__status` | Install review | Review status message |
| `.modn-review__status--error` | Install review | Review error status message |
| `.modn-review__list` | Install review | File operation list |
| `.modn-review__row` | Install review | Single file operation row |
| `.modn-review__path` | Install review | Operation destination path |
| `.modn-review__tag` | Install review | Write or remove tag |
| `.modn-review__blocker` | Install review | Plan blocker message |
| `.modn-approval` | Approval | Inline approval ask panel |
| `.modn-approval__question` | Approval | Approval question line |
| `.modn-approval__list` | Approval | Executable path list |
| `.modn-approval__path` | Approval | Single executable path |
| `.modn-approval__actions` | Approval | Allow and decline row |
| `.modn-approval__button` | Approval | Allow button |
| `.modn-approval__button--quiet` | Approval | Decline button |

The protected hooks are reserved for their regions.
No other screen may use them.

Inside a protected region, a rule may declare only these properties:

- `color`
- `background-color`
- `border-color`
- `outline-color`
- `font-weight`
- `font-style`

Any other property on a protected hook is the rule `protected-region`.
Protected text uses the fonts and sizes the application controls.
A theme may change the colour, weight and style of protected text, not its font family.

A colour value inside a protected region must be opaque.
`transparent`, `currentcolor` with alpha, and `rgba()` or `hsla()` values with alpha below 1 are the rule `protected-color-not-opaque`.
The colour tokens `--modn-color-*` and `color` on `:root` or `html` must be opaque too, because protected regions read them.

### Reachable attributes

These stable data attributes sit on elements inside a protected region.
A selector that names one of them, with any value and any operator, is limited like a protected hook.

| Attribute | Source | Elements |
| --- | --- | --- |
| `data-code` | `apps/modn-desktop/ui/js/details/plan.js` | Review blocker reason |
| `data-operation` | `apps/modn-desktop/ui/js/details/plan.js` | Install review row |
| `data-testid` | `apps/modn-desktop/ui/js/details/plan.js`, `apps/modn-desktop/ui/js/library/ask.js` | Review lines, empty note and approval buttons |

`[data-operation="remove"] { display: none }` and `[data-testid="ask-decline"] { display: none }` are the rule `protected-region`.
`[data-operation="remove"] { color: #eeeeee }` stays allowed.

### Context hooks

These stable classes hold a protected region, surround it, or share its host.
A CSS effect on one of them reaches the region.

| Class | Source | Relation |
| --- | --- | --- |
| `.modn-app` | `apps/modn-desktop/ui/index.html` | Ancestor of both regions |
| `.modn-details__ask` | `apps/modn-desktop/ui/js/details.js` | Approval host |
| `.modn-library__ask` | `apps/modn-desktop/ui/js/library.js` | Approval host |
| `.modn-recent` | `apps/modn-desktop/ui/js/library/recent.js` | Ancestor of the recent approval |
| `.modn-recent__entry` | `apps/modn-desktop/ui/js/library/recent.js` | Sibling of the recent approval host |
| `.modn-recent__menu` | `apps/modn-desktop/ui/js/library/recent.js` | Ancestor of the recent approval |
| `.modn-recent__text` | `apps/modn-desktop/ui/js/library/recent.js` | Sibling hook beside the recent approval |

On these hooks a rule may declare only `color`, `background-color`, `border-color`, `outline-color`, `font-family`, `font-weight` and `font-style`.
It may also declare `--modn-*` custom properties.
Any other declaration is the rule `protected-region`.
So an ancestor or sibling hook may change colours and fonts only.
`.modn-app { opacity: 0 }`, `.modn-recent__menu { overflow: hidden }` and `.modn-recent__entry { position: absolute }` are refused.

The UI invariant is: every element inside a `[data-modn-protected]` subtree carries only protected hook classes.
It carries no shared widget hook class.
Therefore no selector made of shared hooks can reach it.
Attribute selectors and context hooks can reach it, and the two lists above limit them.

Destructive confirmations are native dialogs.
The remove-game confirmation is the only one today.
A theme cannot change it.
Do not add an HTML confirmation.

## Variants

A variant is an extra stylesheet: `theme.<variant>.css`.
It applies after `theme.css`.
Each variant listed in `theme.json` needs its file.
A missing file is `missing-variant-file`.

A variant id uses the id grammar.
The maximum is 16 variants.

An unlisted `theme.<x>.css` is `unexpected-file`.

## Sources and selection

A theme has one of three sources:

| Kind | Source |
| --- | --- |
| `official` | The `main` bucket. |
| `local` | The local theme registry. |
| `bucket` | Any other registered bucket kind. |

On a duplicate `(game, id)` pair, precedence is `official`, then `local`, then other buckets in bucket id order.
A later duplicate is skipped with `duplicate-theme-id`.
So a `(game, theme-id)` pair identifies a theme.

The theme registry is a separate directory under the Modn data directory.
It holds a copy of each registered theme and one record beside it.
The record holds the schema version, game, id, digest and registered-at time.
A registered theme is checked against its digest on every load.
A mismatch is skipped with `digest-mismatch`.

A replacement keeps the previous registration at `registry/.backup/<game>/<id>` until the new copy and its record are in place.
Recovery runs before register, list, load and remove.
It restores a backup when the live registration is missing, and deletes it when the live registration verifies.
When the live registration fails verification, recovery keeps the backup and reports a diagnostic.
A failed restore keeps the backup and names it in the error.

Registration inspects a bounded copy outside the registry.
The copy holds at most 16 MiB and 2048 entries.
Registration then copies the inspected copy into staging, rechecks its digest and installs it.
A refused registration leaves no state: the data directory is untouched.

Selection belongs to Modn state, not to the desktop.
Modn keeps one record per game: a theme choice or the explicit none choice.
The none choice is stored as `"theme": null` and keeps the default theme even when the game has an official theme.
`modn theme reset` clears one game with `--game`, or every record, so the official theme applies again.

The resolve order for a game is:

1. The selected theme, when it is valid.
2. The official theme from `main` for that game: the lexicographically first theme id, with no variant.
3. The default theme: no theme.

An explicit none selection returns `theme: null` and `fallback: null` and skips the official theme.

A selected theme that fails validation, digest or loading yields a fallback with a diagnostic, then the next step of the order.
The fallback reasons are `theme-invalid`, `theme-missing` and `digest-mismatch`.

A broken theme is never fatal for `bucket update`, `bucket sync`, `install`, `update` or `list`.

## CLI contract

The desktop spawns every theme command with the global prefix `--json --state-dir <dir>`.
Every command accepts `--json`.

A diagnostic object has this shape:

```text
{"theme":"<game>/<id>"|null,"path":"<file relative to the theme directory>"|null,"line":<n>|null,"rule":"<rule-id>","message":"<text>"}
```

A realistic example:

```json
{"theme":"cyberpunk-2077/night-city","path":"theme.css","line":12,"rule":"unknown-property","message":"property \"transform\" is not allowed"}
```

`theme` is `<game>/<id>` or `null`.
`path` is a file path relative to the theme directory or `null`.
`line` is a line number or `null`.

A theme entry has this shape:

```text
{"id":"<theme id>","game":"<game id>","name":"<name>","theme_api":1,"variants":["<variant>"...],"icon":null|"assets/<file>","source":<source>,"selected":true|false}
```

For example:

```json
{"id":"night-city","game":"cyberpunk-2077","name":"Night City","theme_api":1,"variants":["neon","noir"],"icon":"assets/icon.png","source":{"kind":"official","bucket":"main","label":"main bucket"},"selected":true}
```

A source has this shape:

```text
{"kind":"official"|"bucket"|"local","bucket":"main"|"<bucket id>"|null,"label":"main bucket"|"<bucket id>"|"local registry"}
```

For example:

```json
{"kind":"official","bucket":"main","label":"main bucket"}
```

`kind` is `official`, `bucket` or `local`.
`bucket` is `main`, a bucket id or `null`.
`label` is `main bucket`, a bucket id or `local registry`.

### check

`modn theme check <directory> [--json]` validates a theme directory.
It writes `{"ok":true|false,"theme":<theme entry>|null,"diagnostics":[...]}`.
The exit status is 0 when `ok` is true, and non-zero otherwise.

```json
{"ok":true,"theme":{"id":"night-city","game":"cyberpunk-2077","name":"Night City","theme_api":1,"variants":["neon","noir"],"icon":"assets/icon.png","source":{"kind":"local","bucket":null,"label":"local registry"},"selected":false},"diagnostics":[]}
```

A failed check returns `theme: null` and its diagnostics:

```json
{"ok":false,"theme":null,"diagnostics":[{"theme":"cyberpunk-2077/night-city","path":"theme.css","line":12,"rule":"unknown-property","message":"property \"transform\" is not allowed"}]}
```

### register

`modn theme register <directory>` validates, copies and stores the digest.
It has no `--trust` flag.
It writes `{"registered":<theme entry>,"digest":"blake3:<hex>"}`.
A theme that fails validation is refused with its diagnostics, and nothing is copied.
A refusal prints this envelope on stderr and exits with status 1 in `--json` mode:

```json
{"error":"THEME_INVALID","message":"<text>","diagnostics":[<diagnostic object>...]}
```

```json
{"registered":{"id":"night-city","game":"cyberpunk-2077","name":"Night City","theme_api":1,"variants":["neon","noir"],"icon":"assets/icon.png","source":{"kind":"local","bucket":null,"label":"local registry"},"selected":false},"digest":"blake3:9f2c4e6a8b0d1f3a5c7e9b1d3f5a7c9e0b2d4f6a8c0e2b4d6f8a0c2e4b6d8f0a"}
```

### list

`modn theme list [--game <game-id>]` lists registered and bucket themes.
It writes `{"themes":[<theme entry>...],"diagnostics":[...]}`.
A none selection marks no theme selected.

```json
{"themes":[{"id":"night-city","game":"cyberpunk-2077","name":"Night City","theme_api":1,"variants":["neon","noir"],"icon":"assets/icon.png","source":{"kind":"official","bucket":"main","label":"main bucket"},"selected":true}],"diagnostics":[]}
```

### remove

`modn theme remove <theme-id> [--game <game-id>]` removes one registration.
It writes `{"removed":{"id":"..","game":".."}}`.
`--game` is required when the id exists for several games.

```json
{"removed":{"id":"night-city","game":"cyberpunk-2077"}}
```

### select

`modn theme select <game-id> <theme-id> [--variant <variant>]` stores a theme selection.
`modn theme select <game-id> --none` stores the explicit none choice.
`--none` conflicts with a theme id and with `--variant`; a bare `select <game-id>` is a usage error.
It writes `{"selected":{"game":"..","theme":".."|null,"variant":null|".."}}`; the none answer has `theme: null` and `variant: null`.

```json
{"selected":{"game":"cyberpunk-2077","theme":"night-city","variant":"neon"}}
```

### reset

`modn theme reset [--game <game-id>]` clears one game or every game.
It clears a theme selection and the none choice, so the official theme applies again.
It writes `{"reset":["<game-id>"...]}`.

```json
{"reset":["cyberpunk-2077"]}
```

### resolve

`modn theme resolve --game <game-id>` returns the theme that the desktop must apply.
It writes this shape:

```text
{"game":"..","theme":null|{"id":"..","name":"..","source":<source>,"variant":null|"..","origin":"selected"|"official","css":"<validated theme.css text>","variant_css":null|"<validated variant css>","assets_dir":"<absolute path of the theme directory>"},"fallback":null|{"reason":"theme-invalid"|"theme-missing"|"digest-mismatch","message":".."},"diagnostics":[...]}
```

`theme: null` means the default theme, including after an explicit none selection.
Then `fallback` is null and the official theme is not used.
`css` and `variant_css` are canonical CSS text with every `url()` written as `url(assets/<path>)`.
`assets_dir` is the absolute path of the theme directory.

```json
{"game":"cyberpunk-2077","theme":{"id":"night-city","name":"Night City","source":{"kind":"official","bucket":"main","label":"main bucket"},"variant":"neon","origin":"selected","css":".modn-app{--modn-color-primary:#00e5ff}.modn-card{background-color:#101014}","variant_css":".modn-card{border-color:#00e5ff}","assets_dir":"C:\\Users\\J\\AppData\\Local\\modn\\themes\\cyberpunk-2077\\night-city"},"fallback":null,"diagnostics":[]}
```

## Desktop behavior

This section describes theme API v1.
The Status table records what exists today.

### Command spawn model

The desktop links no `modn-core`.
Every theme operation spawns the configured `modn` binary through the existing `Backend`.
The spawn sets the fixed prefix `--json --state-dir <dir>`, clears the environment and hides the console.
Calls run on a blocking thread, so the webview thread stays free.

### Tauri commands

Five commands wrap the CLI contract:

| Command | Action |
| --- | --- |
| `list_themes` | Runs `theme list --game <id>` for the active game and returns the answer. |
| `resolve_theme` | Runs `theme resolve --game <id>`, stores `assets_dir` and adds `asset_base`. |
| `select_theme` | Runs `theme select <game> <theme> [--variant <variant>]`, or `theme select <game> --none` when `theme` is null. |
| `reset_theme` | Runs `theme reset [--game <game>]`. |
| `import_theme` | Opens a native folder picker, then runs `theme register <dir>`. |

Each command needs five registrations: the handler, `generate_handler!`, `COMMANDS` in `build.rs`, `capabilities/default.json` and the wrapper in `ui/js/api.js`.

`resolve_theme` answers with the CLI answer plus `asset_base`.
It removes `assets_dir` at every depth and stores the theme root for the protocol.
An answer with a null theme stores no root.
When two `resolve_theme` calls overlap, the older one answers exactly `{"stale":true,"game":"<game>"}`.
That answer changes nothing in Rust, and the controller applies nothing.

### modn-theme protocol

The app registers a URI scheme protocol named `modn-theme` with `register_uri_scheme_protocol`.
The protocol serves only files under `<assets_dir>/assets/**`.
It serves only the `.png`, `.webp` and `.woff2` extensions.
It rejects `..`, backslashes, absolute paths and any path that canonicalises outside the assets directory; each rejection is a 404.
It sets Content-Type by extension and `Access-Control-Allow-Origin: *`, because fonts are fetched in CORS mode.

On Windows the page URL is `http://modn-theme.localhost/`.
Other platforms use the `modn-theme://localhost/` form.
`resolve_theme` returns that value as `asset_base`.
The request-to-file resolution is a plain function with unit tests.
It is not behind `cfg(windows)`.

### CSP change

Add `http://modn-theme.localhost` to `img-src` and `font-src` only.
`style-src` stays `'self'`.

```text
default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data: http://modn-theme.localhost; font-src 'self' http://modn-theme.localhost; connect-src ipc: http://ipc.localhost; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'
```

The desktop UI test repeats this string in `apps/modn-desktop/tests/ui_assets.rs`.
Update both copies.

### CSS application

`resolve_theme` returns the canonical CSS through IPC.
The loader rewrites each `url(assets/` prefix to `url(<asset_base>assets/`.
It creates a `CSSStyleSheet`, calls `replaceSync(css)` and adds the sheet to `document.adoptedStyleSheets`.
CSSOM is not blocked by `style-src 'self'`.
The loader wraps each theme stylesheet, base and variant, in `@layer theme`.

The base layer order is `@layer app, theme, protected;`, declared once in `apps/modn-desktop/ui/css/tokens.css`.
The sheet order is: app sheets (the `link` elements), then `theme.css`, then the variant CSS.
The variant sheet is omitted when the theme has no selected variant.

### Fallback

A load error removes the theme sheets and shows the default theme.
A load error is a command error, invalid CSS or a `replaceSync` throw.
A failed resolve removes the previous game's sheets.
A late answer for a game the user left applies nothing.
The app shows a diagnostic toast that names the rule.
A notice shows once per game, theme, variant and text; the controller remembers at most 50 in one session.

### Refusal diagnostics

The desktop keeps diagnostics only for a `THEME_INVALID` refusal.
It ignores the `diagnostics` key of every other error code.
It never forwards raw CLI text to the webview.
It sanitises each entry and keeps at most 50.
An entry has exactly the keys `theme`, `path`, `line`, `rule` and `message`.
`rule` is 1 to 64 characters from `[a-z0-9-]`; an entry with a missing or invalid `rule` or `message` is dropped.
`message` is cut at 300 characters and `theme` at 130.
ASCII control characters are removed.
`path` becomes `null` when it is absolute, contains `..` or is longer than 200 characters.

When a theme resolves, a diagnostic about another theme of the same game does not become a toast.
The panel shows it with the theme list diagnostics.

### Default theme flag

The start flag `--default-theme` makes `resolve_theme` return a synthetic answer without spawning the CLI.
That answer is `{"game":"<game>","theme":null,"fallback":null,"disabled":"default-theme-flag","diagnostics":[],"asset_base":"http://modn-theme.localhost/"}`.
The controller applies no theme when `disabled` is set.

### Theme control

A Theme control is reachable from the game screen.
It shows the active theme name and its source label.
It lists the themes for the active game and offers a variant selector.
It offers a "Use default theme" button, which stores the explicit none choice.
The game then keeps the default look, even when it has an official theme.
It offers an Import button, which opens a folder picker.

The control uses `.modn-theme-*` hooks.
Theme API v1 reserves that prefix, and the [stable hooks](#stable-hooks) list every class the control uses.

## Authoring checklist

1. Create `themes/<game-id>/<theme-id>/` in the bucket, or in a directory for a local theme.
2. Write `theme.json` with `schema_version` 1, `theme_api` 1, the matching directory ids and a name.
3. List each variant in `variants`, or use an empty array.
4. Write `theme.css` with rules that use only the listed hooks and allowlisted properties.
5. Add `theme.<variant>.css` for each listed variant.
6. Put PNG, WebP and WOFF2 files under `assets/` and reference them as `url(assets/<path>)`.
7. Keep every file and the asset count within the limits.
8. Do not target layout, spacing or visibility inside a protected region.
9. Run `modn theme check <dir> --json` and fix every diagnostic.
10. Register a local theme with `modn theme register <dir>`.
11. Select it with `modn theme select <game-id> <theme-id> [--variant <variant>]`.
12. Publish a bucket theme under `themes/`; a consumer needs no trust step.
