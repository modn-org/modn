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

The internal `test-instruction-budget` Cargo feature disables the wall-clock check for CLI integration tests. It retains the instruction-hook ceiling. Do not enable it in production builds.

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
no-write guarantees remain. An empty relative path names the authorized root
itself, so `directory.list(root_id, "")` lists the top level of that root; all
other host calls still reject an empty path, and listing never creates or
follows a missing root. Rust retains platform access, CLI path selection,
SQLite ownership, file identity and integrity checks, transactions and recovery.
Lua presence signals do not certify runtime compatibility.

## Trusted third-party modules

A third-party module is executable policy. Register it only when you trust its source:

```sh
modn module register ./module --trust
modn module list --json
modn module remove game-id
```

Registration copies the manifest and Lua entrypoint into a separate registry. It requires a valid local `main` bucket and refuses official game identities. The retained digest and identity are checked when loading or listing. Official modules take priority; development overrides remain temporary and fail closed. Removal deletes the registration only. It does not remove installed packages or game files. Trust does not grant installation approval or provide a sandbox.

### Module releases

A module release identity has two parts: the required `version` in `module.json`, and the SHA-256 digest of the `module.json` bytes followed by the entrypoint bytes. Modn stores each release under `<data>/game-modules/<game-id>/versions/<version>/`. It keeps every retained release and imports a receipt with the identity, digest and source.

The `current` file names the one active release. An update stages the new release in private storage and validates it there. It switches the `current` pointer only after the source bucket publishes. A failed update keeps the previous active release, and a failed registry commit removes every release directory it moved and restores every `current` pointer before returning the error. `module update` reports `imported`, `unchanged`, `not_newer` or `pinned`.

Modn refuses a release when the same version is already retained with a different digest. Remove the module and register it again to resolve the conflict. An unversioned registry entry from an earlier release is refused in the same way: remove it and register the module again.

### Pin, unpin, update and rollback

```sh
modn module pin <game-id>
modn module unpin <game-id>
modn module update <game-id>
modn module rollback <game-id>
modn module rollback <game-id> --to <version>
```

`module pin` keeps the active release and records its version and digest in `pin.json`. While pinned, imports, updates and rollback do not move the active release. `module update` still fetches the source bucket and reports `pinned`. `module unpin` removes the pin and allows updates and rollback again.

`module update` fetches the registered source bucket and switches to a newer retained release. A local registration has no bucket, so update refuses it. Register the new version with `modn module register <directory> --trust` instead.

`module rollback` switches the `current` pointer to the previous retained release, or to the version named by `--to`. It changes only future plans. It never changes installed files, game files or ownership records.

`module list` verifies and lists every retained release, with its digest, source, trust state and `active` or `pinned` marker.

### Trusted module sources

`main` is always a trusted module source. A signed Git catalog becomes a module source only when the user trusts it:

```sh
modn bucket trust-modules <bucket>
modn bucket untrust-modules <bucket>
```

`bucket trust-modules` prints the pinned signer key ID and refuses without `--yes`. Use `--fingerprint <KEY_ID>` to require an exact signer. Trust covers the whole bucket, so one acknowledgement covers every game in it. A signer key change ends trust. See [buckets.md](buckets.md#module-sources-and-trust).

A catalog sync imports modules only from trusted buckets. It ignores the `games/`
folder of an untrusted bucket at sync: Modn neither reads nor validates it, and
it reports no ignored modules. Modn refuses those modules when a module is used.
It also ignores a catalog module for a game that `main` provides, because
official modules take priority. `modn doctor` reports that case as
`MODULE_HIDDEN_BY_OFFICIAL`.

`main` and the signed Git catalogs the user trusts for modules may hold modules
only at `games/<game-id>/module.json` and `module.lua`. Those folders may hold
nothing else. A package manifest or any other entry under `games/` is refused at
sync. Package manifests live in `mods/<game-id>/`, so a mod whose ID is `module`
is a normal package. See [buckets.md](buckets.md#module-sources-and-trust).

Modn refuses a module whose bucket is no longer trusted. The refusal names the bucket and the recovery command. Run `modn bucket trust-modules <bucket>` to trust the bucket again, or `modn module remove <game-id>` to remove the module. `modn doctor` reports the same condition as `MODULE_SOURCE_UNTRUSTED`.

## Store identities and detection

A module can declare its store identities as data in `module.json`. Lua never
receives them and gains no filesystem or process API from them.

```json
{"game_id":"cyberpunk-2077","version":"0.2.0","api_version":1,"entrypoint":"module.lua",
 "stores":{"steam":"1091500","gog":"1423049311","epic":"Ginger"}}
```

`stores` is optional. Each key is optional and names one store:

| Key | Identity | Shape |
| --- | --- | --- |
| `steam` | Steam app ID | 1 to 10 decimal digits |
| `gog` | GOG product ID | 1 to 20 decimal digits |
| `epic` | Epic Games Launcher `AppName` | 1 to 64 ASCII letters, digits, `-`, `_` or `.` |

An unknown key or a malformed identity makes the module invalid. The error
starts with `MODULE_STORE_ID_INVALID`. Any API version can declare `stores`.
A change to `stores` is a module release, so increment `version`.
Modn 0.5.0 and earlier refuse the field as an unknown manifest field; Modn 0.6.0
reads it. The matching `main` bucket release ships only after Modn 0.6.0 is
published.

Rust reads the store libraries on Windows. This code knows store formats, not
games:
- **Steam**: the Steam folder from the registry (`SteamPath`, `InstallPath`)
  and `%ProgramFiles(x86)%\Steam`. `steamapps/libraryfolders.vdf` lists more
  libraries. Each `steamapps/appmanifest_<app id>.acf` names its folder below
  `steamapps/common` with `installdir`.
- **GOG**: the `HKLM\SOFTWARE\WOW6432Node\GOG.com\Games` registry keys and
  their `gameID` and `path` values.
- **Epic**: the `.item` manifests in
  `%ProgramData%\Epic\EpicGamesLauncher\Data\Manifests`. Add-on content and
  incomplete installations are skipped.

A library only proposes a folder. Modn then runs the module's `validate`
policy with that folder as the only readable root, the same check as
`game add --path`. A folder that the module does not confirm is reported as
refused, with the reason. A registered module whose release cannot be read is
reported as skipped, and detection still inspects the other games. Detection
writes nothing; see
[`modn game detect`](cli.md#game-detection).

## Launch targets

A module can declare how to start its game as data in `module.json`. Rust
validates every target, and `modn play` starts it. Lua never receives launch
targets and gains no process API.

```json
{"game_id":"cyberpunk-2077","version":"0.3.0","api_version":1,"entrypoint":"module.lua",
 "launch":{"steam":"steam://rungameid/1091500","executable":"bin/x64/Cyberpunk2077.exe"}}
```

`launch` is optional. Each key is optional, but a present `launch` object must
declare at least one target. Any host API version can declare `launch`:

| Key | Shape |
| --- | --- |
| `steam` | `steam://rungameid/<app id>`; the app ID must equal `stores.steam` |
| `gog` | A `goggalaxy:` URI; `stores.gog` must be declared |
| `epic` | A `com.epicgames.launcher:` URI; `stores.epic` must be declared |
| `executable` | A game-relative `.exe` path |

An unknown key, an unsupported scheme, a store URI without its matching store
identity, or a malformed value makes the module invalid. The error starts with
`MODULE_LAUNCH_TARGET_INVALID`. An executable path must be relative and
normalized. Absolute paths, `..` components, backslashes, reserved Windows
names and non-`.exe` files are refused.

At launch, Rust canonicalizes the executable below the selected installation.
A missing file, a non-file, or a path that resolves outside the installation is
refused with `LAUNCH_TARGET_UNSAFE`. A module with no target that applies to
the selected installation is refused with `LAUNCH_TARGET_UNAVAILABLE`.

`modn play` prefers a store URI when [`modn game detect`](cli.md#game-detection)
shows that the selected folder came from the same store. Otherwise it starts
the executable with the game folder as its working directory. A store URI opens
through the Windows shell open handler. Play holds the transaction lock from the
pending-transaction check until the process starts or the URI opens, so no
install or uninstall can change the selected files in between. A module change
to `launch` is a module release, so increment `version`. Modn 0.5.0 and earlier
refuse the field as an unknown manifest field; Modn 0.6.0 reads it. The matching
`main` bucket release ships only after Modn 0.6.0 is published.

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
## Host API v2 and content roots

API 2 modules accept the API 1 manifest fields plus an optional `content_root`.
The host accepts API 1 and API 2 modules. An API 1 module with `content_root` is rejected.

```json
{"game_id":"balatro","version":"0.2.0","api_version":2,"entrypoint":"module.lua",
 "content_root":{"base":"appdata","path":"Balatro/Mods"}}
```

`base` must be `appdata`, the Windows Roaming AppData known folder.
`path` must be relative and safe: no absolute path, `..`, empty, reserved or stream component.
Rust resolves the base, rejects links or junctions on the path, a content root that overlaps the game root, a drive root and a canonical root already recorded by another installation.
A missing content folder is created only inside a managed transaction below the validated base, and rollback removes only folders Modn created.

A schema-v1 plan operation may add `"root": "content"`; an absent or `"game"` root keeps the game installation.
An operation with `"root": "content"` is blocked when the module did not declare `content_root`.
Exclusive `owned_directory` claims are limited to the game root in this release.
Modn records content files with a root-qualified ownership key, so plan, install, dry-run, update, uninstall, adoption and recovery use the same root model.
`validate` still receives only the game root. For an API 2 module with `content_root`, `scan` also authorizes the content root: the game root is numeric ID 1 and the content root is ID 2. A scan entry from the content root sets `root = "content"`; a game-root entry omits `root`. Rust labels and reports both roots, while Lua never receives an absolute root path. `doctor` uses the same authorized roots and reports content-root health from Rust.
