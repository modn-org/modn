# Design-system notes

The owner makes every decision in `design/design-system.figcraft`. Propose
these conventions when the owner creates the file or asks for a change.

## Tokens

- Keep tokens as variables in one collection named `modn`.
- Name each variable `modn/<group>/<name>`, for example `modn/color/accent` or `modn/radius/control`. `inspect.code` turns slashes into hyphens, so pages export `var(--modn-color-accent, ...)`. That matches the `--modn-*` custom properties that issue #142 makes the theme API.
- Use modes for theme variants, such as `dark` and `light`.

## Components

- Make a component for each repeated control, and variants for its states, such as `state=default`, `state=hover` and `state=disabled`.
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
