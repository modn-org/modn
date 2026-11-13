---
name: design-modn
description: Iterate on Modn Desktop screen designs in FigCraft with the owner. Use when the owner asks to change, create or review a screen design or the design system in `design/design.figcraft`, or to turn a design into CSS.
---

# Design Modn screens

The owner decides. The agent edits screen designs in the owner's running
FigCraft app, one request at a time, and the owner watches each change land.

## Roles and files

```text
design/
  design.figcraft          the one design file
    Foundations            owner: tokens (variables) and text styles, shown as swatches
    Components             owner: components and their variants
    ---                    divider
    <Screen>               agent: one page per desktop screen, one top-level frame per state
  renders/<Page>/<frame>.png     agent: render of each saved top-level frame, per page
```

- The owner owns the design system: the Foundations and Components pages, the variables and the styles. The agent changes them only when the owner asks for it in that conversation.
- Screen pages take every token and component from the design system. Variables and components are local to the file, so a design-system change reaches every screen at once.
- The canonical format is `.figcraft`; FigCraft calls its `.fig` output experimental. Import `.fig` files only as reference material, kept outside the repository.
- Git can neither diff nor merge `.figcraft` files. The renders are the reviewable change, and only one branch at a time may change `design.figcraft`.

`scripts/fc.py` calls one MCP tool on the app attached through its control port.
Run it from the repository root. Below, `fc.py <tool> '<json>'` means
`python -I .agents/skills/design-modn/scripts/fc.py <tool> '<json>'`. Pass
`@<file>` instead of the JSON for a batch over the Windows command-line limit.
MCP tools have plain names, such as `get_node`, `save_file` and `screenshot`;
`fc.py --list` prints them. A bare dotted command, such as `page.select`, means
`fc.py run_command '{"command":"page.select","params":{...}}'`.

Look up command parameters with `figcraft-cli commands --filter <command>`.
Read [the FigCraft notes](references/figcraft.md) before the first edit in a session.

## 1. Start the session

1. Attach: run `fc.py inspect_document`. When it fails, ask the owner to close any FigCraft window that has the file open, then start the app with it:
   `Start-Process "C:\Program Files\ArtCraft\FigCraft\figcraft.exe" -ArgumentList '--control','47811','"<absolute path of design/design.figcraft>"'`.
2. Open the file in front: `fc.py open_file '{"path":"<absolute path of design/design.figcraft>"}'`.
3. Go to the screen: `page.list`, then `page.select` with its `id`.

Done when `inspect_document` names this worktree's `design/design.figcraft` as `path` and the screen page is current.

## 2. Apply one request

The owner gives one instruction, such as "change the colors of the button at browse" or "add a subtle shadow to the install button".

1. **Locate.** Run `node.find` with the screen page as `root` and the layer `name` or `text`, then `get_node` on each match. When several layers match, show their names and ask which one.
2. **Classify.** The request **conflicts** with the design system when it:
   - sets a color, font, type size, radius, spacing, opacity or effect that no variable or style holds. Layout widths and heights have no tokens, so they are no conflict;
   - replaces a bound variable (`boundVariables`) or an applied style with a raw value;
   - changes a layer inside a component instance in a way that no component property exposes. Variant, text and boolean properties are the exposed ways;
   - changes the structure of a component instance, or detaches it;
   - needs a component the Components page does not have;
   - changes a variable, a style, or the Foundations or Components page.

   On a conflict, stop and ask before any edit. Name the layer, the token or component involved and the two options: change the design system, or approve a screen-only override.
3. **Edit.** Send the whole request as one `batch`, so the owner can undo it in one step: `fc.py batch '{"commands": [{"command": "<id>", "params": {...}}, ...]}'`. Bind every value that a variable or style holds, rather than writing the raw value. Bind tokens with `"fill": {"$var": "<variable>"}` or `variable.bind`. Place components with `instance.create`, and pick variants and texts with `instance.setProperties`.
4. **Check.** Run `fc.py screenshot '{"ids":[<frame id>]}' --png <scratch path>` and look at the image. Done when it shows the requested change and every other layer is unchanged.
5. **Save.** Run `save_file`, then export the screen's frames:
   `figcraft-cli export design/design.figcraft -o design/renders/<Screen> --page <Screen>`.
   Then run `python -I .agents/skills/design-modn/scripts/audit.py design/design.figcraft --page <Screen>`. It lists every value that no variable or style holds, and every instance override that no component property made. Done when each listed finding is an override the owner approved.
6. **Report.** Name the changed layers, the tokens or components used, each approved override and the render paths.

## 3. Owner comments

The owner can also comment on a layer in the app. `fc.py wait_for_changes` returns new comment events, and `run_command` with `comment.list` and `{"filter":"open"}` lists open threads. Handle each open thread as a request in step 2. After the change, run `comment.reply` with what changed, then `comment.resolve`.

## 4. New screen

Add the page after the divider and its first frame, as one `batch`:

```json
{"commands": [
  {"command": "page.add", "params": {"name": "<Screen>"}},
  {"command": "node.create", "as": "frame", "params": {"type": "frame", "name": "<screen>", "width": 1440, "height": 900, "props": {"fill": {"$var": "modn/color/surface"}}}}
]}
```

`page.add` appends the page and makes it current. Name each further frame after the state it shows, such as `browse-empty`.

## Frame placement

Top-level frames are a page's direct children, component sets included. Keep
one constant gap between neighbours, in a row and between rows: the gap that
already separates the first two non-overlapping frames of the page, or 200 px
when no two are separate. Align the top-left x of frames in a column and the y
of frames in a row. Run `python -I .agents/skills/design-modn/scripts/overlap.py design/design.figcraft` after moving or adding a top-level frame; it must report 0 overlapping pairs on every page.

## 5. Design-system changes

When the owner asks for one, follow [the design-system notes](references/design-system.md). Then run `audit.py design/design.figcraft --page Foundations` and `--page Components`, and export both pages as in step 2.5, to `design/renders/Foundations` and `design/renders/Components`. Done when both audits report 0 findings and the renders, of these pages and of each screen, show the intended effect.

## 6. Hand-off to code

`inspect.code` with `{"id": <frame id>, "language": "html"}` returns the frame's whole layer tree as HTML and CSS; `"css"` returns the CSS of that one layer. A bound variable becomes `var(--<variable name with slashes as hyphens>, <value>)`. `variable.export` with `{"format": "css"}` writes the token stylesheet. The CSS is a reference for the Tauri UI in issue #142, not finished code.

## Commit

Commit `design/design.figcraft` with the renders of each changed page, Foundations and Components included. Name each design-system change in the commit message.
