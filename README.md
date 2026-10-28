# Mods

A Cyberpunk 2077-first, game-agnostic mod manager written in Rust. The CLI supports read-only game discovery, archive inspection, and installation planning. Actual installation, Luau adapters, REDmod deployment, buckets, MCP, and desktop UI will follow in later milestones.

## Development

Requires a current stable Rust toolchain.

```sh
cargo test --workspace
cargo run --bin mod -- game add cyberpunk --path "/path/to/Cyberpunk 2077"
cargo run --bin mod -- game list
cargo run --bin mod -- scan --json
cargo run --bin mod -- doctor --json
cargo run --bin mod -- inspect "./downloaded-mod.zip" --json
cargo run --bin mod -- plan "./downloaded-mod.zip" --path "/path/to/Cyberpunk 2077" --json
```

On Windows, omit `--path` to attempt discovery through Steam libraries, Epic Launcher manifests, and common game directories. Custom GOG/Epic/Steam installs may require an explicit path. Registration writes **only manager-owned state** outside the game directory; `scan` and `doctor` never modify game files.

Use `--state-dir <directory>` for an isolated state database. Multiple registered installations require an explicit `--path` for scanning and diagnostics. No detected mod is considered managed or safe to uninstall.

Current detection recognizes legacy `.archive` files, redscript, tweak files, REDmod directories, CET mod directories, and RED4ext plugin directories. REDmod presence means its executable exists; it is **not** a claim that deployment or game compatibility has been validated.

## Archive inspection and planning

`mod inspect <file>` reads ZIP, JAR, or 7z **headers and SHA-256 only**. It never extracts a file into the game. Archive entries with unsafe paths, duplicates, symbolic links detected in metadata, or encrypted ZIP entries are rejected. Unsupported formats (including RAR) are refused. Plan operations are **not** verified file contents, installation authorization, or game compatibility.

`mod plan <file> --path <game>` maps recognized layouts (legacy archives, REDmod packages with `info.json`, redscript/tweaks, CET and RED4ext plugin directories) to game-relative destinations. An optional single outer package folder is trimmed. Unknown layouts, multiple incompatible variants, executable installers, occupied destinations, and missing REDmod tooling cause `status: "blocked"`. Other plans return `status: "reviewable"` (not "safe to install"). Each response identifies `metadata_only: true`; no install or extraction commands exist yet. **Even reviewable plans require revalidation, content integrity checking, version/compatibility checks, and approval in a future transaction engine.**

## Scope

Windows first. Minecraft, installation transactions, the Luau runtime, and a package browser remain future milestones.

All first-party code is MIT licensed.
