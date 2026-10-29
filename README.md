# modn

A Cyberpunk 2077-first, game-agnostic mod manager written in Rust. The CLI supports game discovery, archive inspection/planning, and **explicit, ownership-tracked installation of legacy .archive ZIP packages**. The first read-only Luau game-module loader is available; game-specific capabilities and adapters, REDmod deployment, other mod types, buckets, MCP, and desktop UI remain future milestones.

## Development

Requires a current stable Rust toolchain.

```sh
cargo test --workspace
cargo run --bin modn -- game add cyberpunk --path "/path/to/Cyberpunk 2077"
cargo run --bin modn -- game list
cargo run --bin modn -- scan --json
cargo run --bin modn -- doctor --json
cargo run --bin modn -- inspect "./downloaded-mod.zip" --json
cargo run --bin modn -- plan "./downloaded-mod.zip" --path "/path/to/Cyberpunk 2077" --json
```

On Windows, omit `--path` to attempt discovery through Steam libraries, Epic Launcher manifests, and common game directories. Custom GOG/Epic/Steam installs may require an explicit path. Registration writes **only manager-owned state** outside the game directory; `scan` and `doctor` never modify game files.

Use `--state-dir <directory>` for an isolated state database. Multiple registered installations require an explicit `--path` for scanning and diagnostics. Only files explicitly recorded in the manager's SQLite ownership table can be reported as managed. Scanning does not adopt or authorize uninstalling detected files.

Current detection recognizes legacy `.archive` files, redscript, tweak files, REDmod directories, CET mod directories, and RED4ext plugin directories. REDmod presence means its executable exists; it is **not** a claim that deployment or game compatibility has been validated.

## Local Luau game modules (experimental, read-only)

`modn module check <directory> --json` validates a local module without creating manager state or touching a game installation. The selected directory must contain `module.json` and one flat `.luau` entrypoint. [A runnable example](examples/game-modules/sample-game/) is included:

```sh
cargo run --bin modn -- module check examples/game-modules/sample-game --json
```

`module.json`:

```json
{"game_id":"sample-game","version":"0.1.0","api_version":1,"entrypoint":"main.luau"}
```

`main.luau`:

```lua
return { game_id = "sample-game", api_version = mods_host.api_version }
```

The loader negotiates **host API v1** and rejects mismatching manifest/script identities, unsupported versions, malformed input, unsafe filenames, linked files and directories, oversized source files, and binary/non-UTF-8 Luau chunks. The VM is fresh per check, with an 8 MiB memory ceiling, a 1-second execution deadline checked at Luau interrupt points, and a limit of 100,000 interrupts. Manifest size is capped at 16 KiB and script size at 256 KiB. Script failures produce explicit errors.

This is a **restricted, untrusted local module inspection** workflow, not module installation, enablement, or compatibility certification. Luau only receives `mods_host.api_version`; it has no host filesystem, state database, game directory, archive, network, or process API. OS, IO, package loading, dynamic script loading, debug, print and warn globals are unavailable. Untrusted scripts cannot write to CLI stdout or corrupt `--json` responses. `mlua` sandboxing and resource limits reduce exposure but are **not a formal security boundary** against native runtime vulnerabilities, and local scripts must not be treated as automatically trusted. No modules are auto-discovered or executed; the operator selects the path. Later roadmap PRs will introduce narrowly validated read-only capabilities and game policy. Existing Rust Cyberpunk CLI commands and transaction semantics remain unchanged.

## Read-only Luau host capabilities (experimental)

PR #7 adds optional, strictly scoped read-only capabilities to the existing local module check command. **No directory is authorized by default.** The operator can explicitly permit up to eight real directories:

```sh
modn module check ./module --allow-root "/path/to/game" --json
# Repeat --allow-root for more than one candidate installation.
```

The host exposes a version-1 additive API on the existing read-only `mods_host` table. Only the operator-selected roots are enumerated; there is **no automatic drive scanning**, no generic Rust claim of game compatibility and no unrestricted filesystem access. A game-specific Luau module uses file metadata to recognize installations among those roots:

```lua
local roots = mods_host.discovery.roots() -- { { id = 1 }, ... }
local root = roots[1]
if root then
    local metadata = mods_host.fs.metadata(root.id, "game.marker")
    -- metadata: { kind = "file" | "directory", bytes = integer }
    local digest = mods_host.hash.sha256(root.id, "game.marker") -- hex string
    local archive = mods_host.archive.inspect(root.id, "mods/package.zip")
    -- { format, sha256, total_uncompressed_bytes, metadata_only = true,
    --   files = { { entry, bytes }, ... } }
end
return { game_id = "my-game", api_version = mods_host.api_version }
```

All paths passed to Lua capabilities are relative to a short-lived **numeric root ID**, not absolute paths. Rust rejects absolute paths, traversal, invalid components, links/junctions and paths escaping the canonical authorized root. Lua receives only immutable metadata, hashes and validated archive entry descriptions, never host file handles, file contents, or writable APIs. Missing files, unsafe paths and exhausted resource budgets fail explicitly. Each invocation has at most 128 host calls, a 16 MiB aggregate file-byte allowance, an 8 MiB per-file limit, a 256-entry archive result limit and the existing Luau memory/interrupt budgets. Luau inspection supports **ZIP and JAR only**; 7z encoded headers can expand without a native parser allocation budget and are explicitly refused until safely bounded. The existing standalone CLI inspector is unchanged. All supported Luau archive inspection is metadata-only; no extraction or installation occurs.

**Security limitations:** Permission to read a root must be granted by the operator; it authorizes metadata and hashing for files beneath it, not module trust. These path checks are not atomic against a *concurrent external attacker modifying the filesystem* between verification and opening; no general-purpose portable no-follow/openat handle confinement is claimed. The bounded native archive parser is also outside the Luau interrupt timer. Do not point an untrusted module at sensitive directories. Game-specific discovery outside operator-approved roots, plans, mutation, ownership and transaction APIs are not included. The existing `modn module check <directory>` behavior without `--allow-root` remains valid.

## Read-only inventory and ownership drift

`modn scan --json --path <game>` combines on-disk discovery with SQLite ownership records. The `mods` array labels discovered legacy archives as `managed` only when the observed path resolves to a recorded, manager-owned file (including valid Windows case-only aliases); managed entries also include `package_id` and `integrity`. Unmanaged CET/RED4ext directories, scripts, tweaks and REDmod folders remain unmanaged even if they appear alongside owned files.

`owned_files` reports every recorded file, **including missing files absent from discovery**, with the statuses `intact`, `modified`, `missing`, `unsafe` (linked/redirected path), or `unreadable`. Content is SHA-256 checked when size and path verification succeed. Large archives may take time to hash. `modn doctor --json --path <game>` adds `OWNED_FILE_MISSING`, `OWNED_FILE_MODIFIED`, `OWNED_FILE_UNSAFE`, and `OWNED_FILE_UNREADABLE` warnings, and reports `UNMANAGED_CONTENT` only when unmanaged entries are present.

The `framework_signals` array identifies candidate CET, RED4ext, redscript, and ArchiveXL directories or binaries with their relative paths. A presence signal **does not verify that a framework is installed, active, or compatible**. `scan` and `doctor` never write to the game or adopt files; the state database remains outside the game installation. A game directory redirect is not followed for ownership verification. No automatic repairs or deletion are performed.

## Archive inspection and planning

`modn inspect <file>` reads ZIP, JAR, or 7z **headers and SHA-256 only**. It never extracts a file into the game. Archive entries with unsafe paths, duplicates, symbolic links detected in metadata, or encrypted ZIP entries are rejected. Unsupported formats (including RAR) are refused. Plan operations are **not** verified file contents, installation authorization, or game compatibility.

`modn plan <file> --path <game>` maps recognized layouts (legacy archives, REDmod packages with `info.json`, redscript/tweaks, CET and RED4ext plugin directories) to game-relative destinations. An optional single outer package folder is trimmed. Unknown layouts, multiple incompatible variants, executable installers, occupied destinations, and missing REDmod tooling cause `status: "blocked"`. Other plans return `status: "reviewable"` (not "safe to install"). Each response identifies `metadata_only: true`; the install command separately revalidates and extracts ZIP legacy archives. **Even reviewable plans require revalidation, content integrity checking, version/compatibility checks, and explicit approval for installation.**

## Transactional legacy archive installation (experimental)

**Only** ZIP archives consisting of recognized `archive/pc/mod/*.archive` payloads (with optional outer wrapper/documentation) are currently installable. Inspect and plan first. **Do not uninstall existing framework mods or adopt manually installed files**; those remain unmanaged.

```powershell
modn plan "C:\Downloads\HD Reworked.zip" --json
modn install "C:\Downloads\HD Reworked.zip" --yes --json
modn installed --json
modn uninstall <package-id> --yes --json
modn recover --yes --json
```

`install` and `uninstall` require explicit `--yes` approval. The manager stores file hashes and ownership in SQLite in the state directory. It rejects existing destinations, linked paths, unsupported mod layouts, encrypted archives, oversized content, and attempts to uninstall modified files. Installation verifies ZIP content into manager-owned staging before creating game files; destination creation uses exclusive hard links rather than replacing existing files. Backups/witnesses remain until the SQLite ownership update commits. An interrupted transaction blocks further writes until `modn recover --yes` can safely roll it back or finish cleanup. If a witness or owned file was modified, recovery deliberately fails without deleting it; resolve that state manually before continuing.

Transactions are **recoverable** rather than one single filesystem-wide atomic visibility event; during a multi-file operation, some files may briefly appear before all are committed. Hard links must be supported by the game volume. This first implementation does not perform REDmod deployment, framework dependency checks, package updates, file adoption, migration, or automatic recovery. Always try it on a disposable game fixture before testing against a real game.

## Scope

Windows first. Minecraft, additional transaction types, game-specific Luau integrations and a package browser remain future milestones.

All first-party code is MIT licensed.
