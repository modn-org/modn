# FigCraft notes

Observed with FigCraft 0.17.0 on Windows. `figcraft-cli commands --filter <command>`
is the source of truth for parameters; these notes cover what it does not say.
After a FigCraft upgrade, run `python -I .agents/skills/design-modn/scripts/test_audit.py`: `audit.py` reads the converter's JSON, and a changed format makes it miss findings.

## Calls

- `run_command` runs one command as one undo step. `batch` runs several as one undo step and rolls all of them back when one fails.
- In `batch` and in `figcraft-cli run` scripts, `as` is a step-level key beside `command`. A later step reads the result with `{"$ref": "<as>"}`, or one field with `{"$ref": "<as>", "field": "id"}`.
- An invalid property fails the call and lists every valid key. Read that list instead of guessing.
- `get_node` accepts `fields` to trim the output. `gap` and `padding` sit inside `layout`.
- `save_file` refuses when another writer changed the file since the app opened it. Reload with `open_file` and `"reload": true`, then repeat the request.

## Components and fonts

- `instance.create` with a component set's id places its default variant; `instance.setProperties` then picks variants and texts.
- An instance keeps its link to its component after an edit. `get_node` lists the edited properties in `overrides`.
- `font.download` installs a missing Google Fonts family only after the owner turns downloading on in **Text ▸ Missing fonts**.

## Properties

- A new frame gets a white fill. Give containers that must show their parent `"fill": "none"`. `"#00000000"` leaves a paint that `audit.py` reports.
- `fill` takes a color, `{"$var": "<variable>"}`, or `"none"`. Gradients go in `fills`: `{"type": "linearGradient", "stops": ["#hex", "#hex"], "from": [0, 0], "to": [1, 1]}`.
- `strokeSides` takes four booleans in `[top, right, bottom, left]` order.
- A drop shadow: `effect.add` with `{"ids": [<id>], "effect": {"type": "dropShadow", "color": "#00000040", "offset": [0, 2], "radius": 6, "spread": 0}}`. `offsetX` and `offsetY` are refused.
- Text in a horizontal layout wraps when it gets a narrow width. Set `width`, or `sizingH: "hug"`.
- `padding` takes a number or `[top, right, bottom, left]`.
- `style.update` with `value` replaces the whole style. For a text style, pass every field: `{"fontFamily", "fontStyle", "fontSize", "lineHeight": <px>, "letterSpacing": "0%"}`. A left-out field resets to its default, such as 12 px and `auto` line height.
- `variable.bind` accepts `gap`, `crossGap` and each padding side: `paddingTop`, `paddingRight`, `paddingBottom`, `paddingLeft`. `opacity` takes a percent.
- `component.bindProperty` takes the layer as `layer`, not `id`, plus `property` and `field: "characters"` for text.
- A text style takes variables through `style.bind` with `fontFamily`, `fontSize` or `lineHeight`. Change a style's name alone with `style.update` and `name` only.

## Reference files

- `figcraft-cli info <file.fig>` lists frame IDs and what the import approximated or skipped.
- `figcraft-cli export <file.fig> -o <out.png> --node <id>` renders one frame.
- `figcraft-cli convert <in> <out>` overwrites `<out>`.
