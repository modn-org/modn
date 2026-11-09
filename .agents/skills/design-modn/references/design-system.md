# Design-system notes

The owner makes every decision in `design/design-system.figcraft`. Propose
these conventions when the owner creates the file or asks for a change.
`scripts/audit.py design/design-system.figcraft` must report 0 raw values.

## Tokens

- Keep tokens as variables, in three collections:
  - `theme`: colours and opacity, with one mode per theme, such as `light` and `dark`;
  - `size`: radius and spacing, with one mode;
  - `type`: `modn/font/family`, and `modn/font/size/<style>` and `modn/font/line/<style>` for each text style, with one mode.
- Name each variable `modn/<group>/<name>`, for example `modn/color/primary` or `modn/radius/m`. `inspect.code` turns slashes into hyphens, so pages export `var(--modn-color-primary, ...)`. That matches the `--modn-*` custom properties that issue #142 makes the theme API.
- Spacing is a 4 px scale from `modn/space/2xs` (2) to `modn/space/4xl` (40). Snap a new gap or padding to the nearest step.
- Give each variable the scopes of its use: colours `ALL_FILLS` and `STROKE_COLOR`; radius `CORNER_RADIUS`; spacing `GAP` and `WIDTH_HEIGHT`; opacity `OPACITY`; fonts `FONT_FAMILY`, `FONT_SIZE` and `LINE_HEIGHT`. Give each colour a description of where it goes.
- `variable.export` with `{"format": "css"}` writes the token stylesheet: the default mode on `:root`, and each other mode on `[data-<collection>="<mode>"]`, such as `[data-theme="dark"]`. Opacity exports as a percent without a unit, so CSS uses `calc(var(--modn-opacity-disabled) / 100)`.

## Text styles

- Name text styles after their type tokens: `display`, `heading-l`, `body-m`, `label-s`.
- Bind each style's `fontFamily`, `fontSize` and `lineHeight` to its `modn/font/*` variables. To change the font, set `modn/font/family` only.

## Components

- Make a component for each repeated control, and variants for its states, such as `state=default`, `state=hover` and `state=disabled`. Write variant properties and values in lowercase.
- Make every text an instance may change a lowercase text property, such as `label`, `value` or `helper`.
- Name each layer after its role, in lowercase: `label`, `field`, `value`, `helper`, `content`, `title`, `body`, `action`. Text content changes; role names stay.
- Give each component set a description that lists its variant properties, their values and its text properties.
- Bind each component fill, stroke, radius, spacing and opacity to a token.
- Prefix a component name with `_` or `.` to keep it out of the published library.

## Publishing

The owner publishes in the app with **Publish library…** and then saves the file.
Each publish increments the library version. Pages see the change through
`library.updates` at the start of the next session.

## Proposing a change

When a request conflicts with the design system, describe the change in the
reply: the variable, style or component, its current value and the proposed
value. Edit `design-system.figcraft` only when the owner asks for it in that
conversation; the owner publishes.
