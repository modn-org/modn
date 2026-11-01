# modn

A Cyberpunk 2077-first, game-agnostic mod manager written in Rust. The CLI supports game discovery, archive inspection, metadata-only installation previews, and **explicit, ownership-tracked installation of supported legacy .archive, CET, RED4ext, redscript and ArchiveXL-associated ZIP and 7z packages**. The runtime-loaded Lua Cyberpunk discovery and diagnostics module is available; REDmod deployment, other mod types, version-constrained dependency resolution, MCP, and desktop UI remain future milestones.

## Development

Requires a current stable Rust toolchain.

```sh
cargo test --workspace
cargo run --bin modn -- game add cyberpunk-2077 --path "/path/to/Cyberpunk 2077"
cargo run --bin modn -- game list
cargo run --bin modn -- scan --json
cargo run --bin modn -- doctor --json
cargo run --bin modn -- inspect "./downloaded-mod.zip" --json
cargo run --bin modn -- install "./downloaded-mod.zip" --dry-run --path "/path/to/Cyberpunk 2077" --json
```

On Windows, omit `--path` to attempt discovery through Steam libraries, Epic Launcher manifests, and common game directories. Custom GOG/Epic/Steam installs may require an explicit path. Registration writes **only manager-owned state** outside the game directory; `scan` and `doctor` never modify game files.

Use `--state-dir <directory>` for an isolated state database. Multiple registered installations require an explicit `--path` for scanning and diagnostics. Only files explicitly recorded in the manager's SQLite ownership table can be reported as managed. Scanning does not claim or authorize uninstalling detected files. Only Modn-installed archives become newly managed.

Current detection recognizes legacy `.archive` files, ArchiveXL `.xl` metadata, redscript `.reds` files, redscript hint `.toml`, tweak `.yaml`/`.yml` files, REDmod directories, CET mods and RED4ext plugins. REDmod presence means its executable exists; it is **not** a claim that deployment or game compatibility has been validated.

## Recommended first-time setup (MVP)

**Start from a clean, verified game installation.** Use the game client or
platform to verify the game files, and remove manually installed mods using
their original removal instructions before starting with Modn. Keep a backup
of personal saves and settings; Modn does not clean or repair an existing game.

Register the game with `modn game add <game-id> --path "<game>"`, inspect
supported ZIP and 7z packages with `install --dry-run`, and install with explicit
`--yes` after reviewing the plan. There is no default game. Keep subsequent
mod installs and uninstalls under Modn's ownership tracking.

If an installation contains manually installed mods, remove them using their
original uninstall instructions before installing overlapping archive packages.
Modn detects unmanaged content but never claims it; existing files or directories
are not silently overwritten.

The MVP prioritizes conservative ownership checks and refusing ambiguous
filesystem layouts, not support for arbitrary symlink/junction setups or
guaranteed protection against another process changing paths concurrently.
Avoid redirected game/mod directories where practical; use dedicated game
installations and do not run another mod manager against the same files
at the same time.

## First-party Lua game modules

`modn module check <directory> --json` validates a local module without creating manager state or touching a game installation. The selected directory must contain `module.json` and one flat `.lua` entrypoint. [A runnable example](examples/game-modules/sample-game/) is included:

```sh
cargo run --bin modn -- module check examples/game-modules/sample-game --json
```

`module.json`:

```json
{"game_id":"sample-game","version":"0.1.0","api_version":1,"entrypoint":"main.lua"}
```

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

## Runtime-loaded Cyberpunk game policy

Cyberpunk discovery, scan classification, framework signals and doctor diagnostics
now reside in `crates/modn-core/game-modules/cyberpunk-2077/` as
`module.json` and `main.lua`. Rust loads them **at runtime** through a
game-agnostic policy executor, not an embedded `cyberpunk.rs` adapter.

The canonical first-party module sources are committed here and **compiled
into the `modn` executable** at build time. A release binary is portable: it
never reads policy from a `game-modules/` directory beside it or from the
build machine's source tree, so replacing files next to the executable
cannot change the reviewed policy. Editing a bundled source requires a
rebuild. There is no separately installed module ZIP, activation database,
module marketplace or bucket-provided Lua policy.

For development, select a source directory containing `module.json` and
its Lua entrypoint for the current invocation only:

```sh
modn game add cyberpunk-2077 --src ./my-cyberpunk-module --path "/path/to/game"
modn --game cyberpunk-2077 scan --module-src ./my-cyberpunk-module --json
```

The `--src` alias for `--module-src` is global but never persisted.
`MODN_GAME_MODULES_DIR/<game-id>/` remains an alternative developer override.
An explicit missing, invalid or mismatched source fails without fallback.
Ordinary commands return to the embedded policy; edits to a selected local
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

## CLI: games, mods, buckets and search

Game registration, mod installation and bucket management use separate commands:

```sh
modn game add cyberpunk-2077 --path "/path/to/game"
modn game add cyberpunk-2077 --src "./local-module" --path "/path/to/game"
modn game list
modn game remove cyberpunk-2077

modn bucket list
modn bucket add community "/path/to/catalog"
modn bucket remove community

modn search cet
modn search --game cyberpunk-2077
modn install cet --file "/path/to/CET.zip" --dry-run
modn install redscript --file "/path/to/redscript.zip"
modn install cyberpunk-2077/cet --file "/path/to/CET.zip" --yes
modn install main/cyberpunk-2077/cet --file "/path/to/CET.zip" --yes
modn install "./unlisted-mod.zip" --dry-run
modn install "./unlisted-mod.zip" --yes
modn list --json
modn uninstall <managed-package-id> --yes
```

`modn install` resolves mod IDs only, not games. Narrow a name by
`game/mod` or `bucket/game/mod`. Interactive terminals prompt for colliding
mod IDs; noninteractive invocations return candidates (structured JSON errors
under `--json`). `modn search` matches mod ID or display name and accepts
global `--game`.

Catalog installation needs each archive with the exact format and SHA-256
from its manifest. Modn downloads an `https` source itself. A `manual`
source, such as a Nexus file, is never downloaded: supply it with `--file`,
and without it Modn names the file to get. Standalone
`modn install <archive.zip|archive.7z>` works without any bucket and uses
the existing Lua planner and Rust ownership transaction. `--dry-run` previews
without game writes. A catalog install in an interactive terminal shows what
it will write and asks `Install? [y/N]`; scripts and `--json` must pass
`--yes`. Standalone archive installs always require `--yes`. Each catalog
dependency must be installed with its pinned digest and intact managed
content, or be installed in the same command. Installation re-checks this
while holding the transaction lock. Each catalog installation records its `bucket/game/mod` provenance and
the managed packages that satisfied its dependencies, in the same SQLite
commit as its ownership. `modn uninstall` refuses to remove a recorded
dependency while a dependent managed package remains; uninstall the dependent
first. Standalone archive installs and packages installed before this record
existed carry no dependency links.

`modn install <mod> --dry-run` also reports a read-only dependency plan.
It lists every dependency of the mod, transitively, in install order. The
plan resolves dependencies in the mod's own bucket and game. A cycle or an
unknown dependency ID fails the dry run. Each dependency has a state:
`ready` (managed with the pinned digest and intact), `damaged` (pinned digest
but drifted content), `other_release` (recorded from the same catalog mod
with another digest) or `missing`. Missing dependencies name their artifact
source. Without `--file`, the dry run names the archive to supply, and JSON
`plan` is `null`. JSON adds `dependencies` and `dependencies_ready`. Each
dependency also shows the supplied `archive`, if one matches. The plan
writes nothing.

To install a mod with its missing dependencies, pass `--file` once for each
`manual` archive, in any order. Modn downloads or reuses the `https` ones.
Modn matches each archive to a release by SHA-256 and refuses an archive that
no release in the installation pins. When a `manual` archive is missing, Modn
names it and writes nothing. One confirmation, or `--yes`, approves the whole stack. With `--yes`,
each executable that a stack manifest declares also needs `--allow-executable`.
Modn installs the stack in dependency order under one transaction lock. The
game policy plans each package after the packages before it are installed.
Each package keeps its own ownership record, provenance and dependency links.
If any package fails, Modn uninstalls the packages that this command
installed, newest first. Like `modn uninstall`, rollback removes owned files
but leaves empty parent directories that installation created. Two
dependencies that pin the same archive digest install it once. If the process stops before the stack completes,
other writes are refused until `modn recover --yes` uninstalls the stack's
packages. A damaged dependency or another installed release is never replaced:
the install fails until you restore or uninstall it.

### Download cache

Modn keeps downloaded archives in a cache, after Scoop's download cache. The
cache is `<state-dir>/cache` unless `MODN_CACHE` names another directory.
Each entry is named `<game>#<mod>#<version>#<url hash><extension>`. Modn
downloads only over HTTPS, and redirects must stay on HTTPS. A download
fails if the server sends nothing for 60 seconds. Each attempt writes its own
`<entry>.<random>.download` file, so concurrent downloads never share bytes.
Modn checks the manifest's SHA-256 and a 4 GiB size limit, then atomically
renames the file into place. A failed check removes that attempt's partial
file and prints the URL with the expected and actual digests. Unlike Scoop,
an entry exists only after its digest is verified. Modn verifies an entry
again each time it uses it; an entry that fails is never used, and the next
download replaces it. The cache directory, like the state directory, must be
outside every game installation; Modn checks this before it downloads. A dry
run reads the cache but never downloads. Modn downloads only after you
confirm. A single mod asks `Download? [y/N]` first, because its install
prompt needs the archive. A stack checks that every `manual` archive is
supplied, then asks once for the whole stack.

- `modn install <mod> --no-cache` downloads again instead of using an entry.
- `modn cache show [mod]` lists entries and their total size.
- `modn cache rm <mod>` or `modn cache rm --all` removes entries. A file
  counts as an entry only if its whole name matches the entry format. Other
  files, including partial downloads, are left alone.

A catalog manifest may declare the tool executables its archive installs,
for example redscript's `engine/tools/scc.exe`. A declaration only lets Modn
name them; it never approves them. The interactive prompt lists each declared
executable the archive installs that no `--allow-executable` flag already
approved, and installs them only if you answer `y`. `--yes` never
approves a declared executable: scripts pass `--allow-executable <path>` for
each one, exactly as for standalone archives. An executable the manifest does
not declare stays blocked unless `--allow-executable` names it.
`--dry-run --json` reports `unapproved_declared_executables`.

`modn game remove <game-id>` only unregisters an installation and never
deletes game files. It refuses to orphan managed mods or pending transactions;
use `--path` if multiple installations have the same ID. `modn list`
shows manager-owned packages, not unmanaged content. `modn uninstall`
currently accepts the managed-package ID shown by `modn list`, not the
bucket mod name; friendly-name uninstall remains future work.

The CLI has **no default game**. Use `modn game add <game-id> --path <directory>`
to register an installation with its ordinary Lua module; use `--game <game-id>`
with `--path` before registration. When a path or registered installation has
one unambiguous recorded game identity, later commands may select it without
`--game`. Ambiguous registrations fail closed. There are no Cyberpunk aliases,
fallback markers, or game-specific Rust install branches.

The fictional `tests/fixtures/test-game/` Lua module uses `bin/TestGame.exe`,
`package/plugins/*.plugin` and `addons/*.plugin`, independently of Cyberpunk.
`verify-modn` proves its actual CLI install, scan, ownership, diagnostics and
uninstall behavior as well as Cyberpunk coexistence, with disposable fixtures.
Run `python .agents/skills/verify-modn/scripts/verify.py --build` from the
repository root (use `python3` on Linux). Do not grant sensitive roots to
untrusted modules: confinement is not atomic against external concurrent path
replacement, and the Lua VM is not a formal security boundary.

## Read-only inventory and ownership drift

`modn --game <game-id> scan --json --path <game>` combines on-disk discovery with SQLite ownership records. The `mods` array labels discovered legacy archives as `managed` only when the observed path resolves to a recorded, manager-owned file (including valid Windows case-only aliases); managed entries also include `package_id` and `integrity`. The Cyberpunk scanner emits individual CET/RED4ext files, redscript and tweak files, ArchiveXL metadata, and redscript hints for exact file-level ownership attribution; empty mod directories remain unmanaged. To preserve the one-second Lua execution budget, CET, RED4ext, redscript, hint and tweak traversal shares a bounded 256-entry budget; larger inventories return `incomplete: true` and `SCAN_INCOMPLETE` diagnostics rather than aborting. The ownership ledger still lists all recorded files. Unmanaged content and REDmod folders remain unmanaged even if they appear alongside owned files.

`owned_files` reports recorded file paths, including files missing from discovery. Shared-location files use `intact`, `modified`, `missing`, `unsafe`, or `unreadable` and retain SHA-256 integrity checks. Files under an exclusively owned directory report `present` when that real directory exists; Modn does not hash those files on scan or uninstall. Missing, linked, or unreadable owned directory roots report `missing`, `unsafe`, or `unreadable`. The original hashes remain installation evidence, not an uninstall gate for exclusive folders. `doctor` warns on missing, modified, unsafe, or unreadable managed content and reports `UNMANAGED_CONTENT` when unmanaged entries are present.

The `framework_signals` array identifies candidate CET, RED4ext, redscript/compiler, ArchiveXL and TweakXL directories or binaries with their relative paths. Doctor emits `REDSCRIPT_NOT_INSTALLED`, `ARCHIVEXL_NOT_INSTALLED`, or `TWEAKXL_NOT_INSTALLED` when corresponding content is present but a known framework marker is absent. A presence signal **does not verify that a framework is installed, active, or compatible**. `scan` and `doctor` never write to the game or adopt files; the state database remains outside the game installation. A game directory redirect is not followed for ownership verification. No automatic repairs or deletion are performed.

Scan JSON now includes `scan_issues`, distinguishing entry-limit truncation
(`SCAN_INCOMPLETE`) from unsafe/linked (`SCAN_UNSAFE_ENTRY`) and unreadable
(`SCAN_UNREADABLE_ENTRY`) entries. On Windows, a verified file symlink named `ArchiveXL.log` inside
`red4ext/plugins/ArchiveXL/` is recognized as generated log output when the
ArchiveXL DLL is present, and reported informationally as `SCAN_IGNORED_GENERATED_LOG` rather than missing
mod content. Directory symlinks and special objects stay unsafe; Unix
conservatively reports an unknown link type. The link is never followed, and
skipped entries remain visible in diagnostics; scanning never grants ownership.

## Archive inspection and dry-run resolution

`modn inspect <file>` reads ZIP, JAR, or 7z **headers and SHA-256 only**. It never extracts a file into the game. Archive entries with unsafe paths, duplicates, symbolic links detected in metadata, or encrypted ZIP entries are rejected. Windows-style backslash separators in ZIP entry names are normalized for planning while original ZIP names are retained for extraction. Traversal, reserved paths, and normalized case-collisions are still rejected. Unsupported formats (including RAR) are refused. Plan operations are **not** verified file contents, installation authorization, or game compatibility.

`modn --game <game-id> install <file> --dry-run --path <game> --json` resolves archive contents into proposed file operations without writing game files. The runtime-loaded Lua game policy provides a **schema-v1** proposal (`schema_version = 1`, `operations = { { source, destination, kind, package_root }, ... }`, `blockers = { { code, message }, ... }`). Rust independently validates source membership, game-root confinement, target paths, case-insensitive duplicates, file/directory overlap, occupied or linked destinations, prohibited executables, declared byte limits, and source-archive changes. The module cannot choose an arbitrary destination root or access write APIs. To keep the existing 8 MiB/1-second module execution budget predictable, previews with more than 2,048 archive files return a blocked result (TOO_MANY_FILES), not a partial proposal. Replacing the Lua policy alone changes the preview without rebuilding Rust.

An optional outer package folder is trimmed by the game policy. Unknown layouts, incompatible package variants, executable installers, occupied destinations, and missing REDmod tooling block the preview. A `status: "reviewable"` preview means the selected Lua policy proposed safe operations, **not** that archive content is verified or installation approved. The bundled Cyberpunk module permits legacy `.archive` and ArchiveXL `.xl`/`.archive.xl` metadata directly under `archive/pc/mod/`, `.reds` under `r6/scripts/` and its subdirectories, `.toml` under `r6/config/redsUserHints/`, `.yaml`/`.yml` under `r6/tweaks/`, CET mods under `bin/x64/plugins/cyber_engine_tweaks/mods/<name>/`, and RED4ext plugins under `red4ext/plugins/<name>/`, plus direct `r6/input/*.xml` definitions when Input Loader is present or included in the ZIP. It still blocks REDmod deployment and unsupported content. An Input Loader package containing its DLL under `red4ext/plugins/input_loader/` deliberately omits three runtime-generated files from installation: `engine/config/platform/pc/input_loader.ini`, `r6/cache/inputContexts.xml`, and `r6/cache/inputUserMappings.xml`. Dry-run JSON lists them under `skipped_generated_files`. Input Loader creates or regenerates these when the game starts; Modn never claims ownership or deletes them during uninstall. Preserve existing caches and follow upstream instructions for optional manual cleanup. Other game-cache and engine-config paths remain unsupported. The Rust executor accepts independently validated ZIP and 7z file operations from any selected game policy. Changing Lua mappings changes both preview and approved installation without rebuilding Rust. `metadata_only: true` means file contents have not been staged or verified. Untrusted Lua results have an 8 MiB copied-string budget and strict per-field limits.

`modn install <file> --yes` re-resolves the same Lua policy as `--dry-run`, then submits its operations to Rust for fresh source hashes, filesystem conflicts and ownership checks under the exclusive transaction lock. Lua itself has no write binding, and preview results are neither stored nor accepted as approval tokens. `--dry-run --path <game>` does not initialize manager state; previews using a registered game open an existing database read-only. Without `--yes` or `--dry-run`, `install` refuses to run. These options are mutually exclusive. **A reviewable dry run never authorizes installation.**

### Supported Cyberpunk archive preview examples

```powershell
modn --game cyberpunk-2077 install "<mod-package.zip>" --dry-run --path "<game-root>" --json
modn --game cyberpunk-2077 install "<mod-package.zip>" --yes --path "<game-root>" --json
```

The first command is read-only; the second writes only after explicit approval and validation.
Both CET and RED4ext are **game policy**, not hardcoded Rust installers. A CET mod
can contain multiple Lua/config files, while RED4ext plugins can include DLLs
and nested assets. These are not claims that the required framework is present
or that a particular mod is compatible with the installed game version.
Recognized directory prefixes use canonical casing (`bin/x64/plugins/cyber_engine_tweaks/mods`,
`red4ext/plugins`, `r6/scripts`, `r6/input`, `r6/tweaks`, `r6/config/redsUserHints` and
`archive/pc/mod`); the per-mod filenames retain their original spelling.
Mismatched prefix casing, unrelated file types, REDmod, installers or unsafe ZIP
names block the **entire** package. The presence of `engine/tools/scc.exe`,
`red4ext/plugins/ArchiveXL/ArchiveXL.dll` or
`red4ext/plugins/TweakXL/TweakXL.dll` is required respectively for redscript
sources/hints, ArchiveXL metadata or TweakXL YAML tweaks. Missing markers return
specific blocked-preview codes; a marker does **not** establish a working
framework, game-version compatibility or per-mod dependency completeness.

For an optional, **read-only** local benchmark of downloaded ZIP archives,
build `modn` then run `python .agents/skills/verify-modn/scripts/benchmark_cp77.py "<archives-folder>"`
(`python3` on Linux). It previews eligible local ZIPs only against a disposable
game marker and synthetic redscript/ArchiveXL/TweakXL framework markers; it never extracts, executes, installs, copies or alters archives,
and does not create manager state. The source archives stay outside the repo;
benchmark results are terminal output only. Use `--max-archive-mib` to change
the default 64 MiB per-file cap. The benchmark also skips ZIPs advertising over
2,048 entries, over 2 MiB of central-directory metadata, or ZIP64 directory
records before loading entry metadata.

## Transactional Lua-planned ZIP and 7z installation (experimental)

The Cyberpunk Lua policy permits ZIP and 7z packages containing recognized legacy archives, ArchiveXL metadata, redscript sources/hints, YAML tweaks, CET mods and RED4ext plugin files, alone or together, with an optional wrapper and recognized documentation. A supported multi-file package is installed and uninstalled under one ownership record. Unknown or unsupported layouts block the entire package; `mods` (REDmod), unknown framework/bootstrap files and executable installers remain out of scope. Inspect and use `install --dry-run` first. Existing manually installed files remain unmanaged; remove them before installing a conflicting package through Modn.

```powershell
modn --game cyberpunk-2077 install "C:\Downloads\HD Reworked.zip" --dry-run --json --path <game>
modn --game cyberpunk-2077 install "C:\Downloads\HD Reworked.zip" --yes --json --path <game>
modn list --json
modn uninstall <package-id> --yes --json
modn recover --yes --json
```

The Cyberpunk Lua policy prefers an **exclusive mod folder** for new CET, RED4ext, and mod-specific redscript subdirectories when their shared parent already exists. Modn owns the entire newly created folder: uninstall quarantines and deletes it, including changed files and runtime-generated content. Do not place unrelated personal content in these folders. Pre-existing mod directories cannot be claimed wholesale; loose archives, tweaks and XML mappings use conservative file-level ownership. An exclusive claim never overwrites a pre-existing folder. Modn records the operating system's directory-object identity in its SQLite ledger (on Linux this requires a supported filesystem birth timestamp, not just a reusable inode). A transaction prepares each new folder at a same-volume staging name, persists that identity, and then activates it by rename. Recovery checks the directory object rather than a file marker, so interrupted cleanup remains restartable without hashing runtime contents.

`uninstall` and standalone archive installs require explicit `--yes` approval; catalog installs accept `--yes` or an interactive confirmation. The manager stores file hashes and ownership in SQLite in the state directory. It rejects existing destinations, linked paths, unsupported mod layouts, encrypted archives, oversized content, and attempts to uninstall modified files. Installation verifies archive content into manager-owned staging before creating game files; destination creation uses exclusive hard links rather than replacing existing files. Backups/witnesses remain until the SQLite ownership update commits. An interrupted transaction blocks further writes until `modn recover --yes` can safely roll it back or finish cleanup. If a witness or owned file was modified, recovery deliberately fails without deleting it; resolve that state manually before continuing.

The transaction executor is game-agnostic: Rust callers may submit an explicitly approved `InstallPlan` to `Store::install_operations_zip` for any real installation root. Under the exclusive store lock it independently re-inspects the archive, verifies archive hashes and all source sizes, validates destinations, collisions, executable payloads, ownership conflicts, and size limits before staging. The ownership ledger, rollback, crash recovery, and uninstall operate on generic installation roots. The CLI now resolves installation operations from Lua; no Cyberpunk-specific Rust installation planner or legacy compatibility adapter remains. ZIP and 7z extraction are supported; REDmod deployment remains unsupported. CET/RED4ext `.dll` plugin files are permitted within the recognized plugin layout, but neither Modn nor its preview executes them or certifies their compatibility; prohibited executable payloads remain blocked unless explicitly approved as an exact tool path by the caller.

Transactions are **recoverable** rather than one single filesystem-wide atomic visibility event; during a multi-file operation, some files may briefly appear before all are committed. Hard links must be supported by the game volume. This implementation does not perform REDmod deployment, runtime/compatibility verification (only framework marker presence checks), package updates, file adoption, migration, or automatic recovery. Always try it on a disposable game fixture before testing against a real game.

### Cyberpunk framework ZIP installation

CET (Cyber Engine Tweaks), RED4ext and redscript compiler distributions can
now be installed and removed through the ordinary ZIP lifecycle. The Cyberpunk
Lua policy recognizes each supported distribution through paired
bootstrap markers and mandatory ZIP members in the same wrapper. Use `install --dry-run` to
verify the whole archive, then `install --yes` to record **one package** per
framework ZIP. Partial and mixed framework distributions remain blocked.

Framework payloads use **file-level ownership** even inside CET and RED4ext
directories: other mods, personal configuration and generated runtime files
remain separate. Uninstall verifies hashes of owned framework files and
leaves unowned logs and settings untouched; modified bootstrap files block
uninstall. Rust denies executable payloads by default, including the redscript compiler.
Installing redscript requires the operator to approve `engine/tools/scc.exe`:
at the catalog install prompt, or by adding
`--allow-executable engine/tools/scc.exe` to **both** the review preview and
confirmed installation. This is a trusted, explicit approval of one exact
game-relative tool executable, independent of Lua operation labels.
Without that specific approval, executables remain prohibited. Explicit tool-executable permission does not certify the executable as trustworthy.
Modn does not execute framework binaries or certify runtime compatibility
or game-version support. Only catalog-installed dependents block a framework
uninstall; standalone archives that need a framework are not tracked.

### Common Cyberpunk prerequisite package layouts

The following is a **file-layout installation** compatibility list, not a
claim that Modn validates game-version compatibility or mod dependencies.
All eleven original local distributions were installed and uninstalled together
in an isolated game fixture:

| Package | Recognized installation layout | Archive exercise |
| --- | --- | --- |
| ArchiveXL | RED4ext plugin and redscript hint | Original ZIP |
| Browser Extension | `r6/scripts/BrowserExtension/*.reds` | Original solid 7z |
| Codeware | RED4ext plugin | Original ZIP |
| Cyber Engine Tweaks | CET framework bootstrap and assets | Original ZIP |
| Equipment-EX | ArchiveXL metadata, redscript and redscript hint | Original ZIP (five payload files; LICENSE skipped) |
| Mod Settings | RED4ext plugin | Original ZIP |
| No Intro Videos | redscript source | Original ZIP |
| RED4ext | Framework bootstrap | Original ZIP |
| TweakXL | RED4ext plugin | Original ZIP |
| Virtual Atelier | ArchiveXL metadata, archives, redscript and hint | Original ZIP |
| redscript | Compiler and framework configuration | Original ZIP |

The redscript framework ZIP additionally requires explicit approval of
`engine/tools/scc.exe`, either at the catalog install prompt or with
`--allow-executable engine/tools/scc.exe` on both preview and installation. Third-party mod ZIPs/7z files cannot include unapproved
executable payloads. The game-agnostic transaction engine streams 7z
members directly into private staging with the same archive hash, file-size,
ownership and crash recovery checks as ZIP; it never extracts unselected
content into the game. Lua's read-only archive inspection capability still excludes 7z.

## Clean manager state

Modn does not migrate older ownership databases. Installations must use a
fresh manager state database with a versioned schema and archive-backed
package records. If an earlier version of Modn was used, move its state database aside before starting;
do not delete game files or personal saves to reset manager state.

## Scope

Windows first. Minecraft, additional transaction types, game-specific Lua integrations and a package browser remain future milestones.

All first-party code is MIT licensed.


## Local buckets (metadata contract v1)

Modn includes a read-only `main` bucket snapshot of the eleven Cyberpunk
prerequisite packages: RED4ext, redscript, Cyber Engine Tweaks, ArchiveXL,
TweakXL, Codeware, Mod Settings, Equipment-EX, Browser Extension, No Intro
Videos and Virtual Atelier. Each SHA-256 was checked against the original
distribution. GitHub-hosted packages record their release URL as an `https`
source. Nexus-only packages are `manual` sources naming the original file.
Modn does not mirror these archives. It downloads an `https` source from its
original URL when you install it. The snapshot does not assert compatibility with every game
version. See [provenance](buckets/main/PROVENANCE.md) for versions, dependency
evidence and compatibility notes.

```powershell
modn bucket list --json
modn search --game cyberpunk-2077 --json
modn bucket show main/cyberpunk-2077/red4ext --json
modn bucket add community C:\path\to\bucket
modn bucket show community/test-game/example --json
modn bucket remove community
```

A local bucket contains `bucket.json` (schema_version: 1, name, title)
and `games/<game-id>/<package-id>.json` files. Each package record
has schema_version, id, game, name, version, archive_format (`zip`/`7z`),
sha256, source (`manual` with filename, or `https` with URL),
optional dependencies (same-game IDs), dependency_versions, conflicts,
executables and homepage. Unknown fields and
unsupported schemas are rejected. See `buckets/main` for examples.
Registrations point to explicitly selected directories. Package manifests
are revalidated on every read. Buckets contain mod-package metadata only;
`modules/` directories are rejected. See [bucket contract v1](buckets/README.md).
Game modules are first-party source shipped with Modn, outside buckets.

Bucket registration, listing, searching and manifest inspection are
**metadata-only** and never modify game files. A catalog entry can be
installed from a supplied, SHA-256-verified ZIP/7z with an explicit approval;
standalone local archives use the same managed transaction engine. A checksum
proves the bytes match a catalog record, not publisher trust.

A manifest may require a dependency version with `dependency_versions` and
declare `conflicts` with other mods of its bucket and game. Before any
download or write, Modn refuses a plan in these cases:
- a dependency's release in the bucket misses a requirement
- two releases in the installation conflict
- a release conflicts with an installed managed package, in either direction

A dry run lists these conflicts in `conflicts`. The Rust core checks
conflicts again under the transaction lock, and records each package's
conflicts in the same commit as its ownership. A bucket publishes one release
per mod, so Modn checks requirements rather than choosing between versions.
Installed packages carry no version, so an `other_release` dependency is
still not replaced or reinterpreted.
