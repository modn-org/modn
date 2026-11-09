# FigCraft notes

Observed with FigCraft 0.17.0 on Windows. `figcraft-cli commands --filter <command>`
is the source of truth for parameters; these notes cover what it does not say.

## Calls

- `run_command` runs one command as one undo step. `batch` runs several as one undo step and rolls all of them back when one fails.
- In `batch` and in `figcraft-cli run` scripts, `as` is a step-level key beside `command`. A later step reads the result with `{"$ref": "<as>"}`, or one field with `{"$ref": "<as>", "field": "id"}`.
- An invalid property fails the call and lists every valid key. Read that list instead of guessing.
- `get_node` accepts `fields` to trim the output. `gap` and `padding` sit inside `layout`.
- `save_file` refuses when another writer changed the file since the app opened it. Reload with `open_file` and `"reload": true`, then repeat the request.

## Library

- Pass the library `id` from `library.add` to `library.place`, `library.applyStyle` and `library.bindVariable`. The library name is the document name at publish time, which is `Untitled` for a file created headless. A forward-slash path does not match the stored backslash path.
- `library.list` shows an `error` when the stored path is missing. `library.add` with the current path repairs it.
- An instance keeps its link to the library after an edit. `get_node` lists the edited properties in `overrides`.

## Properties

- A new frame gets a white fill. Give containers that must show their parent `"fill": "#00000000"`.
- `fill` takes a color, `{"$var": "<variable>"}`, or `"none"`. Gradients go in `fills`: `{"type": "linearGradient", "stops": ["#hex", "#hex"], "from": [0, 0], "to": [1, 1]}`.
- `strokeSides` takes four booleans in `[top, right, bottom, left]` order.
- A drop shadow: `effect.add` with `{"ids": [<id>], "effect": {"type": "dropShadow", "color": "#00000040", "offset": [0, 2], "radius": 6, "spread": 0}}`. `offsetX` and `offsetY` are refused.
- Text in a horizontal layout wraps when it gets a narrow width. Set `width`, or `sizingH: "hug"`.
- `padding` takes a number or `[top, right, bottom, left]`.

## Reference files

- `figcraft-cli info <file.fig>` lists frame IDs and what the import approximated or skipped.
- `figcraft-cli export <file.fig> -o <out.png> --node <id>` renders one frame.
- `figcraft-cli convert <in> <out>` overwrites `<out>`.
