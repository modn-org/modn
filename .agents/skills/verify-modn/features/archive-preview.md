# ZIP inspection and read-only preview

## Sub-features

- A1: Inspect fictional package ZIP metadata.
- A2: Resolve `test-game` Luau ZIP mapping.
- A3: Verify preview preserves game bytes and does not initialize manager state.
- A4: Verify a second game's independent ZIP mapping.

## How to get to it (user POV)

`modn inspect <archive> --json`; `modn --game test-game install <archive> --dry-run --path <game> --json`; `--game` is global.

## Driving it with Python subprocess

The driver creates `Demo/package/plugins/example.plugin` in a ZIP. The real CLI produces a `reviewable`, `metadata_only` schema-v1 operation mapping to `addons/example.plugin`, with a `Demo` wrapper. The driver checks the exact mapping, takes independent file snapshots, and asserts no state directory was created. It later previews Cyberpunk's unrelated `archive/pc/mod/managed.archive` operation.

## Gotchas

A dry run never authorizes installation; the transaction engine validates sources and destinations again on approved writes. The two modules are selected by explicit or stored game IDs, not destination layout sniffing.
