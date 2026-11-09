---
name: design-modn
description: Iterate on Modn Desktop page designs in FigCraft with the owner. Use when the owner asks to change, create or review a page design under `design/`, or to turn a page design into CSS.
---

# Design Modn pages

The owner decides. The agent edits page designs in the owner's running
FigCraft app, one request at a time, and the owner watches each change land.

## Roles and files

```text
design/
  design-system.figcraft   owner: tokens (variables), styles and components, published as a library
  pages/<page>.figcraft    agent: one canonical design per desktop page
  renders/<page>.png       agent: render of the last saved page
```

- The owner owns `design-system.figcraft` and every library publish. The agent reads it through the library link only, and leaves the file and its publishes to the owner.
- Pages take every token and component from that library.
- The canonical format is `.figcraft`. `library.add` accepts only a `.figcraft` file, and FigCraft calls its `.fig` output experimental. Import `.fig` files only as reference material, kept outside the repository.
- Git cannot diff `.figcraft` files. The render is the reviewable change.

`scripts/fc.py` calls one MCP tool on the app attached through its control port.
Run it from the repository root. Below, `fc.py <tool> '<json>'` means
`python -I .agents/skills/design-modn/scripts/fc.py <tool> '<json>'`.
MCP tools have plain names, such as `get_node`, `save_file` and `screenshot`;
`fc.py --list` prints them. A bare dotted command, such as `library.add`, means
`fc.py run_command '{"command":"library.add","params":{...}}'`.

Look up command parameters with `figcraft-cli commands --filter <command>`.
Read [the FigCraft notes](references/figcraft.md) before the first edit in a session.

## 1. Start the session

1. Attach: run `fc.py inspect_document`. When it fails, ask the owner to close any FigCraft window that has the page open, then start the app with the page:
   `Start-Process "C:\Program Files\ArtCraft\FigCraft\figcraft.exe" -ArgumentList '--control','47811','"<absolute page path>"'`.
2. Open the page in front: `fc.py open_file '{"path":"<absolute page path>"}'`.
3. Relink: run `library.add` with the absolute path of `design/design-system.figcraft` in this worktree. A page stores the library's absolute path, and adding the same library again re-points it. Keep the returned `id`: every `library.*` command below takes it as `library`.
4. Pull: run `library.updates`. When it lists changes, the owner has published them: run `library.acceptUpdates`, then `save_file`, and report what changed.

Done when `library.list` shows that `id` with this worktree's `path` and no `error`, `library.updates` reports `count: 0`, and `inspect_document` names the page file as `path`.

When `design/design-system.figcraft` is absent, stop and ask the owner to create and publish it. [The design-system notes](references/design-system.md) give the conventions to propose.

## 2. Apply one request

The owner gives one instruction, such as "change the colors of the button at browse" or "add a subtle shadow to the install button".

1. **Locate.** Run `node.find` by layer `name` or `text`, then `get_node` on each match. When several layers match, show their names and ask which one.
2. **Classify.** The request **conflicts** with the design system when it:
   - sets a color, size, spacing, radius, effect or font that no published variable or style holds;
   - replaces a bound variable (`boundVariables`) or an applied style with a raw value;
   - changes the structure of a library instance, or detaches it;
   - needs a component the library does not have.

   On a conflict, stop and ask before any edit. Name the layer, the token or component involved and the two options: the owner changes the design system and publishes, or the owner approves a page-only override.
3. **Edit.** Send the whole request as one `batch`, so the owner can undo it in one step. Bind every value that a published variable or style holds, rather than writing the raw value. Bind tokens with `"fill": {"$var": "<variable>"}` or `variable.bind`. Place components with `library.place`, and pick variants with `instance.setProperties`.
4. **Check.** Run `fc.py screenshot '{"ids":[<page frame id>]}' --png <scratch path>` and look at the image. Done when it shows the requested change and every other layer is unchanged.
5. **Save.** Run `save_file`, then `screenshot` the page frame again with `--png design/renders/<page>.png`. Git keeps the history; `file.saveVersion` stores a snapshot inside the file and grows it with each one.
6. **Report.** Name the changed layers, the tokens or components used, each approved override and the render path.

## 3. Owner comments

The owner can also comment on a layer in the app. `fc.py wait_for_changes` returns new comment events, and `run_command` with `comment.list` and `{"filter":"open"}` lists open threads. Handle each open thread as a request in step 2. After the change, run `comment.reply` with what changed, then `comment.resolve`.

## 4. New page

Create the file headless, then open it as in step 1:

```json
[
  {"command": "library.add", "params": {"path": "<absolute design-system path>"}},
  {"command": "node.create", "params": {"type": "frame", "name": "<Page>", "width": 1440, "height": 900}}
]
```

Run `figcraft-cli run new-page.json --out design/pages/<page>.figcraft`, with the script in a scratch folder.

## 5. Hand-off to code

`inspect.code` with `{"id": <frame id>, "language": "html"}` returns the frame's whole layer tree as HTML and CSS; `"css"` returns the CSS of that one layer. A bound variable becomes `var(--<variable name with slashes as hyphens>, <value>)`. The CSS is a reference for the Tauri UI in issue #142, not finished code.

## Commit

Commit each page file with its render. Commit `design-system.figcraft` only when the owner asks.
