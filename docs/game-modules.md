# Game modules

## Check a local module

`modn module check <directory> --json` validates a local module without creating manager state or touching a game installation. The selected directory must contain `module.json` and one flat `.lua` entrypoint. [A runnable example](../examples/game-modules/sample-game/) is included:

```sh
cargo run --bin modn -- module check examples/game-modules/sample-game --json
```

`module.json`:

```json
{"game_id":"sample-game","version":"0.1.0","api_version":1,"entrypoint":"main.lua"}
```

An optional `min_modn_version` sets the minimum Modn release that can load the module.
Use a complete semantic version string, such as `"min_modn_version":"0.3.0"`, without operators or wildcards.
Modn compares it with the core crate version set at build time, before executing Lua.
Semantic version precedence applies, including prereleases; build metadata does not affect precedence.
An absent field adds no minimum release requirement. The `api_version` check still applies.
Malformed values return an input error.
A newer requirement returns, for example, `module cyberpunk-2077 needs Modn 0.3.0, you have 0.2.0`.

Released Modn 0.2.0 rejects this field as an unknown manifest field.
It never silently loads a module that declares the requirement.
Its error names `min_modn_version`, but cannot display the needed release.
Keep `api_version` at 1 when the module still uses host API v1.
Version requirements can cover new host validation behavior without changing that API revision.

`main.lua`:

```lua
return { game_id = "sample-game", api_version = mods_host.api_version }
```

The loader negotiates **host API v1** and rejects mismatching manifest/script identities, unsupported versions, malformed input, unsafe filenames, linked files and directories, oversized source files, and binary/non-UTF-8 Lua chunks. The VM is fresh per check, with an 8 MiB memory ceiling, a 1-second execution deadline checked every 1,000 VM instructions, and a ceiling of 100,000 instruction-hook callbacks. Manifest size is capped at 16 KiB and script size at 256 KiB. Script failures produce explicit errors.

This is **explicitly selected local module execution**, not compatibility certification or code trust verification. Without explicit roots, Lua only receives `mods_host.api_version`; it has no host filesystem, state database, game directory, archive, network, or process API. OS, IO, package loading, dynamic script loading, debug, print and warn globals are unavailable. The default host exposes no filesystem writes or process API, and suppresses script printing so `--json` stays valid. These execution limits are **not a sandbox or security guarantee** for intentionally trusted modules. Read-only capabilities and runtime policy selection are documented below.

## Read-only Lua host capabilities (experimental)

PR #7 adds optional, strictly scoped read-only capabilities to the existing local module check command. **No directory is authorized by default.** The operator can explicitly permit up to eight real directories:

```sh
modn module check ./module --allow-root "/path/to/game" --json
# Repeat --allow-root for more than one candidate installation.
```

The host exposes a version-1 additive API on the existing `mods_host` API (a proxy rejecting normal assignments). Only the operator-selected roots are enumerated; there is **no automatic drive scanning**, no generic Rust claim of game compatibility and no unrestricted filesystem access. A game-specific Lua module uses file metadata to recognize installations among those roots:

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

All paths passed to Lua capabilities are relative to a short-lived **numeric root ID**, not absolute paths. Rust rejects absolute paths, traversal, invalid components, links/junctions and paths escaping the canonical authorized root. Lua receives metadata snapshots, hashes and validated archive entry descriptions as ordinary mutable Lua tables, never host file handles, file contents, or writable APIs. Missing files, unsafe paths and exhausted resource budgets fail explicitly. Each invocation has at most 128 host calls, a 16 MiB aggregate file-byte allowance, an 8 MiB per-file limit, a 256-entry archive result limit and the existing Lua memory/instruction-hook budgets. Lua inspection supports **ZIP and JAR only**; 7z encoded headers can expand without a native parser allocation budget and are explicitly refused until safely bounded. The existing standalone CLI inspector is unchanged. All supported Lua archive inspection is metadata-only; no extraction or installation occurs.

**Security limitations:** Permission to read a root must be granted by the operator; it authorizes metadata and hashing for files beneath it, not module trust. These path checks are not atomic against a *concurrent external attacker modifying the filesystem* between verification and opening; no general-purpose portable no-follow/openat handle confinement is claimed. The bounded native archive parser also executes outside the Lua instruction-hook timer. Do not point an untrusted module at sensitive directories. Game-specific discovery runs only inside approved roots; module-supplied file mappings are untrusted data, and mutation, ownership and transaction APIs are not exposed to Lua. The existing `modn module check <directory>` behavior without `--allow-root` remains valid.

## Runtime-loaded game policy

Cyberpunk discovery, scan classification, framework signals and doctor diagnostics
now reside in `games/cyberpunk-2077/` of the
[`modn-org/main`](https://github.com/modn-org/main) bucket repository as
`module.json` and `module.lua`. Rust loads them **at runtime** through a
game-agnostic policy executor, not an embedded `cyberpunk.rs` adapter.

For development, select a source directory containing `module.json` and
its Lua entrypoint for the current invocation only:

```sh
modn game add cyberpunk-2077 --src ./my-cyberpunk-module --path "/path/to/game"
modn --game cyberpunk-2077 scan --module-src ./my-cyberpunk-module --json
```

The `--src` alias for `--module-src` is global but never persisted.
`MODN_GAME_MODULES_DIR/<game-id>/` remains an alternative developer override.
An explicit missing, invalid or mismatched source fails without fallback.
Ordinary commands return to the local `main` policy; edits to a selected local
source are observed without rebuilding Modn. A persistent
`MODN_GAME_MODULES_DIR` applies to every command in that environment, so
unset it outside development. Neither override grants write
access to the game or changes Rust installation transactions.

The shared read-only host adds `mods_host.directory.list(root_id,relative[,max_entries])`
(default and maximum 2,048 entries; an optional per-call bound reports
`incomplete=true` when truncated) and `mods_host.directory.read_text(root_id,relative)`
(UTF-8, max 64 KiB). Existing root containment, link denial, budgets and
no-write guarantees remain. Rust retains platform access, CLI path selection,
SQLite ownership, file identity and integrity checks, transactions and recovery.
Lua presence signals do not certify runtime compatibility.

## Trusted third-party modules

A third-party module is executable policy. Register it only when you trust its source:

```sh
modn module register ./module --trust
modn module list --json
modn module remove game-id
```

Registration copies the manifest and Lua entrypoint into a separate registry. It requires a valid local `main` bucket and refuses official game identities. The retained digest and identity are checked when loading or listing. Official modules take priority; development overrides remain temporary and fail closed. Removal deletes the registration only. It does not remove installed packages or game files. Community buckets cannot contain modules. Trust does not grant installation approval or provide a sandbox.

## Skyrim example

Source: `examples/game-modules/skyrim-special-edition/`.
Register that directory with explicit trust, or select it through the existing source override.
The example uses the official module's `pcall` pattern for metadata checks.
Unsupported ancestor directories return false during installation validation.
Missing executable markers or Data directories still reject installation registration.
The CLI lifecycle test registers an unchanged copy of this example.
Select game ID `skyrim-special-edition` and an explicit disposable or user-selected game path.
The module requires a regular `SkyrimSE.exe` file and a real `Data` directory.
It does not infer game identity from a folder name. It supplies no automatic discovery paths.

Archives must use exact `Data/` prefixes. Wrapped archives and bare plugin files are rejected.
Supported files are top-level `.esp`, `.esm`, `.esl`, and `.bsa` files under `Data`.
Loose files are limited to `.nif` under `Data/meshes/` and `.dds` under `Data/textures/`.
Filename extension checks ignore case. Directory names retain their supplied spelling.
Root `readme.txt` and `license.txt` are ignored. Other unsupported files block the plan.
All supported files use individual ownership in shared locations.
Rust retains destination validation, overwrite protection, rollback, recovery, and uninstall authority.

Scan reports candidate Data content. It does not distinguish official files from manually installed content.
Doctor reports scan limits and managed-file drift. It always reminds the user to manage load order.
No SKSE, DLLs, Papyrus scripts, FOMOD installers, save migration, automatic activation, or load-order editing is supported.
The module does not validate plugin dependencies, game versions, BSA contents, or in-game behavior.


Synthetic fixtures cover the managed lifecycle. They do not establish real-game compatibility.
The example is documentation and test input; Modn does not embed it.
