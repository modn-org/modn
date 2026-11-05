# Development

Requires a current stable Rust toolchain. The tests use the real main bucket,
pinned as the `buckets/main` submodule of `modn-org/main`.

```sh
git submodule update --init
cargo test --workspace
# Milestone 3 gate with the real archives, in a disposable fake game directory:
MODN_GATE_ARCHIVES=~/Downloads/CP77 cargo test -p modn-cli --test milestone_gate -- --ignored
cargo run --bin modn -- game add cyberpunk-2077 --path "/path/to/Cyberpunk 2077"
cargo run --bin modn -- game list
cargo run --bin modn -- scan --json
cargo run --bin modn -- doctor --json
cargo run --bin modn -- inspect "./downloaded-mod.zip" --json
cargo run --bin modn -- install "./downloaded-mod.zip" --dry-run --path "/path/to/Cyberpunk 2077" --json
```

## Project memory in a pull request

Use project-memory version 0.2.0 or later for the CLI and MCP server.
Install the approved upstream branch with this command:

```sh
uv tool install --force git+ssh://git@github.com/sooskca/project-memory@memory-event-files
```

The upstream branch supplies version 0.2.0 until its release reaches the default branch.
Restart an existing MCP server after you update its tool.
Run `project-memory --root . status` from the repository root to check the store.

The tracked `memory/events/` directory selects one file per new event.
Use `project-memory --root . add` to record a decision or finding.
Use `project-memory --root . supersede <id> --reason "<reason>"` to revise a record.
Commit each generated JSON file with the change it describes.
Keep event files immutable after you commit them.
Never edit or append to `memory/events.jsonl` in a pull request.
The tool reads that legacy log first, then event files in filename order.

Run these checks before you commit a memory change:

```sh
project-memory --root . context "<task>"
project-memory --root . search "<word from the record>"
project-memory --root . rebuild
```

## Verification

The fictional `tests/fixtures/test-game/` Lua module uses `bin/TestGame.exe`,
`package/plugins/*.plugin` and `addons/*.plugin`, independently of Cyberpunk.
`verify-modn` proves its actual CLI install, scan, ownership, diagnostics and
uninstall behavior as well as Cyberpunk coexistence, with disposable fixtures.
Run `python .agents/skills/verify-modn/scripts/verify.py --build` from the
repository root (use `python3` on Linux). Do not grant sensitive roots to
untrusted modules: confinement is not atomic against external concurrent path
replacement, and the Lua VM is not a formal security boundary.

## Cyberpunk archive benchmark

For an optional, **read-only** local benchmark of downloaded ZIP archives,
build `modn` then run `python .agents/skills/verify-modn/scripts/benchmark_cp77.py "<archives-folder>"`
(`python3` on Linux). It previews eligible local ZIPs only against a disposable
game marker and synthetic redscript/ArchiveXL/TweakXL framework markers; it never extracts, executes, installs, copies or alters archives,
and does not create manager state. The source archives stay outside the repo;
benchmark results are terminal output only. Use `--max-archive-mib` to change
the default 64 MiB per-file cap. The benchmark also skips ZIPs advertising over
2,048 entries, over 2 MiB of central-directory metadata, or ZIP64 directory
records before loading entry metadata.
