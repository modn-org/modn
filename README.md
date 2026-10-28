# Mods

A Cyberpunk 2077-first, game-agnostic mod manager written in Rust. The first pull request implements **read-only discovery and diagnostics only**. Mod installation, Luau adapters, REDmod deployment, buckets, MCP, and desktop UI will follow in later milestones.

## Development

Requires a current stable Rust toolchain.

```sh
cargo test --workspace
cargo run --bin mod -- game add cyberpunk --path "/path/to/Cyberpunk 2077"
cargo run --bin mod -- game list
cargo run --bin mod -- scan --json
cargo run --bin mod -- doctor --json
```

On Windows, omit `--path` to attempt discovery through Steam libraries, Epic Launcher manifests, and common game directories. Custom GOG/Epic/Steam installs may require an explicit path. Registration writes **only manager-owned state** outside the game directory; `scan` and `doctor` never modify game files.

Use `--state-dir <directory>` for an isolated state database. Multiple registered installations require an explicit `--path` for scanning and diagnostics. No detected mod is considered managed or safe to uninstall.

Current detection recognizes legacy `.archive` files, redscript, tweak files, REDmod directories, CET mod directories, and RED4ext plugin directories. REDmod presence means its executable exists; it is **not** a claim that deployment or game compatibility has been validated.

## Scope

Windows first. Minecraft, installation transactions, the Luau runtime, and a package browser remain future milestones.

All first-party code is MIT licensed.
