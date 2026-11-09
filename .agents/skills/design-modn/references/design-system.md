# Design-system notes

The owner makes every decision in `design/design-system.figcraft`. Propose
these conventions when the owner creates the file or asks for a change.

## Tokens

- Keep tokens as variables. Put colours in a collection named `theme`, with one mode per theme, such as `light` and `dark`. Put radius and spacing in a collection named `size`, with one mode.
- Name each variable `modn/<group>/<name>`, for example `modn/color/primary` or `modn/radius/m`. `inspect.code` turns slashes into hyphens, so pages export `var(--modn-color-primary, ...)`. That matches the `--modn-*` custom properties that issue #142 makes the theme API.
- `variable.export` with `{"format": "css"}` writes the token stylesheet: the default mode on `:root`, and each other mode on `[data-<collection>="<mode>"]`, such as `[data-theme="dark"]`.

## Components

- Make a component for each repeated control, and variants for its states, such as `state=default`, `state=hover` and `state=disabled`. Write variant properties and values in lowercase.
- Bind each component fill, stroke, radius and spacing to a token.
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
