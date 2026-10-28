# Mods

A Cyberpunk 2077-first, game-agnostic mod manager written in Rust. The CLI supports game discovery, archive inspection/planning, and **explicit, ownership-tracked installation of legacy .archive ZIP packages**. Luau adapters, REDmod deployment, other mod types, buckets, MCP, and desktop UI remain future milestones.

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

Use `--state-dir <directory>` for an isolated state database. Multiple registered installations require an explicit `--path` for scanning and diagnostics. Only files explicitly recorded in the manager's SQLite ownership table can be reported as managed. Scanning does not adopt or authorize uninstalling detected files.

Current detection recognizes legacy `.archive` files, redscript, tweak files, REDmod directories, CET mod directories, and RED4ext plugin directories. REDmod presence means its executable exists; it is **not** a claim that deployment or game compatibility has been validated.

## Read-only inventory and ownership drift

`mod scan --json --path <game>` combines on-disk discovery with SQLite ownership records. The `mods` array labels discovered legacy archives as `managed` only when their exact relative path belongs to an installed package; managed entries also include `package_id` and `integrity`. Unmanaged CET/RED4ext directories, scripts, tweaks and REDmod folders remain unmanaged even if they appear alongside owned files.

`owned_files` reports every recorded file, **including missing files absent from discovery**, with the statuses `intact`, `modified`, `missing`, `unsafe` (linked/redirected path), or `unreadable`. Content is SHA-256 checked when size and path verification succeed. Large archives may take time to hash. `mod doctor --json --path <game>` adds `OWNED_FILE_MISSING`, `OWNED_FILE_MODIFIED`, `OWNED_FILE_UNSAFE`, and `OWNED_FILE_UNREADABLE` warnings, and reports `UNMANAGED_CONTENT` only when unmanaged entries are present.

The `framework_signals` array identifies candidate CET, RED4ext, redscript, and ArchiveXL directories or binaries with their relative paths. A presence signal **does not verify that a framework is installed, active, or compatible**. `scan` and `doctor` never write to the game or adopt files; the state database remains outside the game installation. A game directory redirect is not followed for ownership verification. No automatic repairs or deletion are performed.

## Archive inspection and planning

`mod inspect <file>` reads ZIP, JAR, or 7z **headers and SHA-256 only**. It never extracts a file into the game. Archive entries with unsafe paths, duplicates, symbolic links detected in metadata, or encrypted ZIP entries are rejected. Unsupported formats (including RAR) are refused. Plan operations are **not** verified file contents, installation authorization, or game compatibility.

`mod plan <file> --path <game>` maps recognized layouts (legacy archives, REDmod packages with `info.json`, redscript/tweaks, CET and RED4ext plugin directories) to game-relative destinations. An optional single outer package folder is trimmed. Unknown layouts, multiple incompatible variants, executable installers, occupied destinations, and missing REDmod tooling cause `status: "blocked"`. Other plans return `status: "reviewable"` (not "safe to install"). Each response identifies `metadata_only: true`; the install command separately revalidates and extracts ZIP legacy archives. **Even reviewable plans require revalidation, content integrity checking, version/compatibility checks, and explicit approval for installation.**

## Transactional legacy archive installation (experimental)

**Only** ZIP archives consisting of recognized `archive/pc/mod/*.archive` payloads (with optional outer wrapper/documentation) are currently installable. Inspect and plan first. **Do not uninstall existing framework mods or adopt manually installed files**; those remain unmanaged.

```powershell
mod plan "C:\Downloads\HD Reworked.zip" --json
mod install "C:\Downloads\HD Reworked.zip" --yes --json
mod installed --json
mod uninstall <package-id> --yes --json
mod recover --yes --json
```

`install` and `uninstall` require explicit `--yes` approval. The manager stores file hashes and ownership in SQLite in the state directory. It rejects existing destinations, linked paths, unsupported mod layouts, encrypted archives, oversized content, and attempts to uninstall modified files. Installation verifies ZIP content into manager-owned staging before creating game files; destination creation uses exclusive hard links rather than replacing existing files. Backups/witnesses remain until the SQLite ownership update commits. An interrupted transaction blocks further writes until `mod recover --yes` can safely roll it back or finish cleanup. If a witness or owned file was modified, recovery deliberately fails without deleting it; resolve that state manually before continuing.

Transactions are **recoverable** rather than one single filesystem-wide atomic visibility event; during a multi-file operation, some files may briefly appear before all are committed. Hard links must be supported by the game volume. This first implementation does not perform REDmod deployment, framework dependency checks, package updates, file adoption, migration, or automatic recovery. Always try it on a disposable game fixture before testing against a real game.

## Scope

Windows first. Minecraft, additional transaction types, the Luau runtime, and a package browser remain future milestones.

All first-party code is MIT licensed.
