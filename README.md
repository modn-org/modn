# modn

A Cyberpunk 2077-first, game-agnostic mod manager written in Rust. The CLI supports game discovery, archive inspection, metadata-only installation previews, and **explicit, ownership-tracked installation of supported legacy .archive, CET, RED4ext, redscript and ArchiveXL-associated ZIP packages**. The runtime-loaded Luau Cyberpunk discovery and diagnostics module is available; REDmod deployment, other mod types, buckets, MCP, and desktop UI remain future milestones.

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

Use `--state-dir <directory>` for an isolated state database. Multiple registered installations require an explicit `--path` for scanning and diagnostics. Only files explicitly recorded in the manager's SQLite ownership table can be reported as managed. Scanning does not adopt or authorize uninstalling detected files. Only a separately confirmed adoption grants ownership and later deletion permission.

Current detection recognizes legacy `.archive` files, ArchiveXL `.xl` metadata, redscript `.reds` files, redscript hint `.toml`, tweak `.yaml`/`.yml` files, REDmod directories, CET mods and RED4ext plugins. REDmod presence means its executable exists; it is **not** a claim that deployment or game compatibility has been validated.

## Recommended first-time setup (MVP)

**Start from a clean, verified game installation.** Use the game client or
platform to verify the game files, and remove manually installed mods using
their original removal instructions before starting with Modn. Keep a backup
of personal saves and settings; Modn does not clean or repair an existing game.

Register the game with `modn game add <game-id> --path "<game>"`, inspect
supported ZIP packages with `install --dry-run`, and install with explicit
`--yes` after reviewing the plan. There is no default game. Keep subsequent
mod installs and uninstalls under Modn's ownership tracking.

If an installation already contains manually installed mods, treat
[explicit adoption](#explicit-adoption-of-manually-installed-mods-experimental)
as an **advanced, optional migration** rather than normal setup. A scan
does not grant ownership. Adoption transfers deletion authority for exactly
the approved files; do not adopt content you cannot confidently identify.
Unsupported or ambiguous files remain unmanaged, and conflicting existing
files are not silently overwritten.

The MVP prioritizes conservative ownership checks and refusing ambiguous
filesystem layouts, not support for arbitrary symlink/junction setups or
guaranteed protection against another process changing paths concurrently.
Avoid redirected game/mod directories where practical; use dedicated game
installations and do not run another mod manager against the same files
at the same time.

## Local Luau game modules (experimental)

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

This is **restricted, untrusted local module inspection**, not compatibility certification. Without explicit roots, Luau only receives `mods_host.api_version`; it has no host filesystem, state database, game directory, archive, network, or process API. OS, IO, package loading, dynamic script loading, debug, print and warn globals are unavailable. Untrusted scripts cannot write to CLI stdout or corrupt `--json` responses. `mlua` sandboxing and resource limits reduce exposure but are **not a formal security boundary** against native runtime vulnerabilities. Read-only capabilities and runtime policy selection are documented below.

### Distributing and selecting local module ZIPs

A module package is an **unsigned local ZIP**, containing exactly root
`module.json` and the one flat `.luau` file named by its manifest. Add
`"package_format":1` to the manifest above when distributing a ZIP. The existing
directory check also accepts manifests without this field. Unknown or duplicate
JSON fields are rejected. Game IDs, host API and optional script `version`
declarations must agree with the manifest. Version labels are opaque, case-sensitive
labels (1–48 ASCII letters, digits, `.`, `-`, `+`), not ordered SemVer ranges.
A game/version pair is immutable: publish changed content under a new label.

Use ZIP stored or deflate compression, regular files with no executable bits,
and matching local/central headers in the same entry order. Directories, extra files, links, reparse flags,
encryption, duplicate/case-colliding names, unsafe or Windows-reserved names,
ZIP64, split archives, extra fields, comments, data descriptors, prefixes and
trailing data are refused. The compressed input is limited to 288 KiB, central
metadata to 8 KiB, manifest to 16 KiB and expanded script to 256 KiB. Expanded
bytes and CRCs are checked before running source. For example, Python's ordinary
seekable `zipfile.ZipFile(..., "w", compression=zipfile.ZIP_DEFLATED)` with two
`writestr` calls produces this format.

```sh
modn --state-dir ./manager module inspect ./test-game-1.zip --json
modn --state-dir ./manager module install ./test-game-1.zip --dry-run --json
modn --state-dir ./manager module install ./test-game-1.zip --yes
modn --state-dir ./manager module list --json
modn --state-dir ./manager module enable test-game 1.0.0 --yes
modn --state-dir ./manager module status test-game --json
modn --state-dir ./manager module update ./test-game-2.zip --yes
modn --state-dir ./manager module rollback test-game --yes
modn --state-dir ./manager module disable test-game --yes
modn --state-dir ./manager module enable test-game 1.0.0 --yes
```

All module writes require `--yes`. Installation alone never enables or shadows
a bundled module. `update` requires an already enabled user module and atomically
installs/selects the new version; initial activation requires `enable`. Switching
versions retains the previous selection. Rollback swaps current and previous,
so it is reversible; retrying the same update preserves the rollback target.
Disable preserves both selections and all installed content, and returns policy
resolution to the bundled module where available. It does not disable an explicit
operator override. Updates and rollback never remove old versions.

**Trust:** SHA-256 checks integrity of recorded bytes; it does **not authenticate
a publisher or prove safety**. Package inspection executes validation in
the bounded Luau sandbox with zero filesystem roots. Before a write, the
incoming policy also checks candidate state-directory ancestors using the
existing root-scoped read-only game-recognition check, to reject a store inside
a game. Those grants are temporary. Enabled policies run in
that same sandbox with the existing command-authorized read-only capabilities
and can propose mod installation mappings. Review the source and its origin
before enabling it. There are no remote downloads, signing keys, or additional
host capabilities in this workflow.

The separate store is `<state-dir>/game-modules/`: `modules.sqlite` records
package digests and transactional selections; `versions/<content-sha256>/`
holds exact immutable manifest/source bytes. The content address hashes the
ASCII domain `modn-module-package-v1` plus NUL, the manifest's little-endian
64-bit byte length, then manifest and script bytes. Separate manifest and script
SHA-256 values are also recorded and reported. Module actions never create
`state.sqlite` or write game files. Read-only check, inspect, dry-run, list and
status do not create a store. Use a state directory outside game installations;
existing registration roots are read only to enforce that write boundary.

Writers take an exclusive store lock (a busy store gives an explicit retry error),
publish a complete version by a rename from staging on the same filesystem,
then commit selection through SQLite with FULL synchronous mode. Every policy
read rechecks the exact selected bytes **before execution**; corruption of an
explicit active selection fails closed with no bundled fallback. An interrupted
publication can leave unused `.staging-*` directories or complete unselected
content; neither becomes active. Retry the original command. File contents are
flushed before publication; directory durability is additionally synced on Unix.
Portable Windows directory fsync and protection against concurrent external
filesystem replacement are not claimed.

For repair, inspect `module status <game-id>` and its integrity error. Restore
the original manifest/source bytes from the trusted original ZIP, or explicitly
`module disable <game-id> --yes` / enable another intact installed version.
Rollback requires intact current and previous packages. A reinstall refuses to
overwrite tampered content or reuse a label with different bytes. Stop other
module writers before manually removing abandoned `.staging-*` directories;
leave `versions/` and `modules.sqlite` together, and never edit the selection
database or delete rollback versions. A corrupt selection database itself must
be restored from backup, or the entire module store moved aside and original
ZIPs reinstalled and explicitly enabled. The game ownership database remains
separate.

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

**Security limitations:** Permission to read a root must be granted by the operator; it authorizes metadata and hashing for files beneath it, not module trust. These path checks are not atomic against a *concurrent external attacker modifying the filesystem* between verification and opening; no general-purpose portable no-follow/openat handle confinement is claimed. The bounded native archive parser is also outside the Luau interrupt timer. Do not point an untrusted module at sensitive directories. Game-specific discovery runs only inside approved roots; module-supplied file mappings are untrusted data, and mutation, ownership and transaction APIs are not exposed to Luau. The existing `modn module check <directory>` behavior without `--allow-root` remains valid.

## Runtime-loaded Cyberpunk game policy

Cyberpunk discovery, scan classification, framework signals and doctor diagnostics
now reside in `crates/modn-core/game-modules/cyberpunk-2077/` as
`module.json` and `main.luau`. Rust loads them **at runtime** through a
game-agnostic policy executor, not an embedded `cyberpunk.rs` adapter.

Resolution priority is an explicit `MODN_GAME_MODULES_DIR/<game-id>/` override,
then an explicitly enabled user version under the selected `--state-dir`, then
`game-modules/<game-id>/` beside the executable, then the repository directory
for development builds. Distributions must ship the bundled module folder beside
the executable. `module status` reports the effective origin and selected digests.
The operator override always wins, even over a corrupt user selection, and a
missing or invalid override never falls back. Core library callers scope policy
operations with `game_modules::with_module_store(state_dir, action)`; the CLI
does this before resolving any game policy.
Missing or incompatible modules fail closed. Policy edits are observed on the
next CLI command without recompiling (covered by a CLI integration test).

The shared read-only host adds `mods_host.directory.list(root_id,relative[,max_entries])`
(default and maximum 2,048 entries; an optional per-call bound reports
`incomplete=true` when truncated) and `mods_host.directory.read_text(root_id,relative)`
(UTF-8, max 64 KiB). Existing root containment, link denial, budgets and
no-write guarantees remain. Rust retains platform access, CLI path selection,
SQLite ownership, file identity and integrity checks, transactions and recovery.
Luau presence signals do not certify runtime compatibility.

The CLI has **no default game**. Use `modn game add <game-id> --path <directory>`
to register an installation with its ordinary Luau module; use `--game <game-id>`
with `--path` before registration. When a path or registered installation has
one unambiguous recorded game identity, later commands may select it without
`--game`. Ambiguous registrations fail closed. There are no Cyberpunk aliases,
fallback markers, or game-specific Rust install branches.

The fictional `tests/fixtures/test-game/` Luau module uses `bin/TestGame.exe`,
`package/plugins/*.plugin` and `addons/*.plugin`, independently of Cyberpunk.
`verify-modn` proves its actual CLI install, scan, ownership, diagnostics and
uninstall behavior as well as Cyberpunk coexistence, with disposable fixtures.
Run `python .agents/skills/verify-modn/scripts/verify.py --build` from the
repository root (use `python3` on Linux). Do not grant sensitive roots to
untrusted modules: confinement is not atomic against external concurrent path
replacement, and the Luau VM is not a formal security boundary.

## Read-only inventory and ownership drift

`modn --game <game-id> scan --json --path <game>` combines on-disk discovery with SQLite ownership records. The `mods` array labels discovered legacy archives as `managed` only when the observed path resolves to a recorded, manager-owned file (including valid Windows case-only aliases); managed entries also include `package_id` and `integrity`. The Cyberpunk scanner emits individual CET/RED4ext files, redscript and tweak files, ArchiveXL metadata, and redscript hints for exact file-level ownership attribution; empty mod directories remain unmanaged. To preserve the one-second Luau execution budget, CET, RED4ext, redscript, hint and tweak traversal shares a bounded 256-entry budget; larger inventories return `incomplete: true` and `SCAN_INCOMPLETE` diagnostics rather than aborting. The ownership ledger still lists all recorded files. Unmanaged content and REDmod folders remain unmanaged even if they appear alongside owned files.

`owned_files` reports every recorded file, **including missing files absent from discovery**, with the statuses `intact`, `modified`, `missing`, `unsafe` (linked/redirected path), or `unreadable`. Content is SHA-256 checked when size and path verification succeed. Large archives may take time to hash. `modn --game <game-id> doctor --json --path <game>` adds `OWNED_FILE_MISSING`, `OWNED_FILE_MODIFIED`, `OWNED_FILE_UNSAFE`, and `OWNED_FILE_UNREADABLE` warnings, and reports `UNMANAGED_CONTENT` only when unmanaged entries are present.

The `framework_signals` array identifies candidate CET, RED4ext, redscript/compiler, ArchiveXL and TweakXL directories or binaries with their relative paths. Doctor emits `REDSCRIPT_NOT_INSTALLED`, `ARCHIVEXL_NOT_INSTALLED`, or `TWEAKXL_NOT_INSTALLED` when corresponding content is present but a known framework marker is absent. A presence signal **does not verify that a framework is installed, active, or compatible**. `scan` and `doctor` never write to the game or adopt files; the state database remains outside the game installation. A game directory redirect is not followed for ownership verification. No automatic repairs or deletion are performed.

Scan JSON now includes `scan_issues`, distinguishing entry-limit truncation
(`SCAN_INCOMPLETE`) from unsafe/linked (`SCAN_UNSAFE_ENTRY`) and unreadable
(`SCAN_UNREADABLE_ENTRY`) entries. On Windows, a verified file symlink named `ArchiveXL.log` inside
`red4ext/plugins/ArchiveXL/` is recognized as generated log output when the
ArchiveXL DLL is present, and reported informationally as `SCAN_IGNORED_GENERATED_LOG` rather than missing
mod content. Directory symlinks and special objects stay unsafe; Unix
conservatively reports an unknown link type. The link is never followed, and
**adoption remains blocked** if any entries were skipped, including generated logs.

## Archive inspection and dry-run resolution

`modn inspect <file>` reads ZIP, JAR, or 7z **headers and SHA-256 only**. It never extracts a file into the game. Archive entries with unsafe paths, duplicates, symbolic links detected in metadata, or encrypted ZIP entries are rejected. Unsupported formats (including RAR) are refused. Plan operations are **not** verified file contents, installation authorization, or game compatibility.

`modn --game <game-id> install <file> --dry-run --path <game> --json` resolves archive contents into proposed file operations without writing game files. The runtime-loaded Luau game policy provides a **schema-v1** proposal (`schema_version = 1`, `operations = { { source, destination, kind, package_root }, ... }`, `blockers = { { code, message }, ... }`). Rust independently validates source membership, game-root confinement, target paths, case-insensitive duplicates, file/directory overlap, occupied or linked destinations, prohibited executables, declared byte limits, and source-archive changes. The module cannot choose an arbitrary destination root or access write APIs. To keep the existing 8 MiB/1-second sandbox predictable, previews with more than 2,048 archive files return a blocked result (TOO_MANY_FILES), not a partial proposal. Replacing the Luau policy alone changes the preview without rebuilding Rust.

An optional outer package folder is trimmed by the game policy. Unknown layouts, incompatible package variants, executable installers, occupied destinations, and missing REDmod tooling block the preview. A `status: "reviewable"` preview means the selected Luau policy proposed safe operations, **not** that archive content is verified or installation approved. The bundled Cyberpunk module permits legacy `.archive` and ArchiveXL `.xl`/`.archive.xl` metadata directly under `archive/pc/mod/`, `.reds` under `r6/scripts/` and its subdirectories, `.toml` under `r6/config/redsUserHints/`, `.yaml`/`.yml` under `r6/tweaks/`, CET mods under `bin/x64/plugins/cyber_engine_tweaks/mods/<name>/`, and RED4ext plugins under `red4ext/plugins/<name>/`. It still blocks REDmod deployment and unsupported content. The Rust executor accepts independently validated ZIP file operations from any selected game policy. Changing Luau mappings changes both preview and approved installation without rebuilding Rust. `metadata_only: true` means file contents have not been staged or verified. Untrusted Luau results have an 8 MiB copied-string budget and strict per-field limits.

`modn install <file> --yes` re-resolves the same Luau policy as `--dry-run`, then submits its operations to Rust for fresh source hashes, filesystem conflicts and ownership checks under the exclusive transaction lock. Luau itself has no write binding, and preview results are neither stored nor accepted as approval tokens. `--dry-run --path <game>` does not initialize manager state; previews using a registered game open an existing database read-only. Without `--yes` or `--dry-run`, `install` refuses to run. These options are mutually exclusive. **A reviewable dry run never authorizes installation.**

### Supported Cyberpunk ZIP preview examples

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
`red4ext/plugins`, `r6/scripts`, `r6/tweaks`, `r6/config/redsUserHints` and
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

## Transactional Luau-planned ZIP installation (experimental)

The Cyberpunk Luau policy permits ZIP packages containing recognized legacy archives, ArchiveXL metadata, redscript sources/hints, YAML tweaks, CET mods and RED4ext plugin files, alone or together, with an optional wrapper and recognized documentation. A supported multi-file package is installed and uninstalled under one ownership record. Unknown or unsupported layouts block the entire package; `mods` (REDmod), unrelated framework/bootstrap files and executable installers remain out of scope. Inspect and use `install --dry-run` first. Existing manually installed files remain unmanaged until a selected package is explicitly adopted using the workflow below.

```powershell
modn --game cyberpunk-2077 install "C:\Downloads\HD Reworked.zip" --dry-run --json --path <game>
modn --game cyberpunk-2077 install "C:\Downloads\HD Reworked.zip" --yes --json --path <game>
modn installed --json
modn uninstall <package-id> --yes --json
modn recover --yes --json
```

`install` and `uninstall` require explicit `--yes` approval. The manager stores file hashes and ownership in SQLite in the state directory. It rejects existing destinations, linked paths, unsupported mod layouts, encrypted archives, oversized content, and attempts to uninstall modified files. Installation verifies ZIP content into manager-owned staging before creating game files; destination creation uses exclusive hard links rather than replacing existing files. Backups/witnesses remain until the SQLite ownership update commits. An interrupted transaction blocks further writes until `modn recover --yes` can safely roll it back or finish cleanup. If a witness or owned file was modified, recovery deliberately fails without deleting it; resolve that state manually before continuing.

The transaction executor is game-agnostic: Rust callers may submit an explicitly approved `InstallPlan` to `Store::install_operations_zip` for any real installation root. Under the exclusive store lock it independently re-inspects the ZIP, verifies archive hashes and all source sizes, validates destinations, collisions, executable payloads, ownership conflicts, and size limits before staging. The ownership ledger, rollback, crash recovery, and uninstall operate on generic installation roots. The CLI now resolves installation operations from Luau; no Cyberpunk-specific Rust installation planner or legacy compatibility adapter remains. No new archive extraction formats or REDmod deployment are enabled. CET/RED4ext `.dll` plugin files are permitted within the recognized plugin layout, but neither Modn nor its preview executes them or certifies their compatibility; prohibited installer/script executables remain blocked.

Transactions are **recoverable** rather than one single filesystem-wide atomic visibility event; during a multi-file operation, some files may briefly appear before all are committed. Hard links must be supported by the game volume. This implementation does not perform REDmod deployment, runtime/compatibility verification (only framework marker presence checks), package updates, file adoption, migration, or automatic recovery. Always try it on a disposable game fixture before testing against a real game.

## Explicit adoption of manually installed mods (experimental)

Adoption is an **advanced, optional** opt-in **ownership transfer**, not a reinstall.
For a new setup, prefer the clean-install workflow above. Game-specific Luau
policy proposes bounded, explicit groups of existing files; Rust rejects incomplete
scans, links/aliases/hard links, unknown or ambiguous groups, executables,
ownership conflicts, changed file content, and unsafe destinations. Groups include
only exact listed files, not all files in a directory or generated configurations.

    modn --game cyberpunk-2077 adopt --dry-run --path "<game>" --json
    modn --game cyberpunk-2077 adopt "archive/pc/mod/Existing.archive" --yes --path "<game>" --json
    modn --game cyberpunk-2077 installed --path "<game>" --json

A dry run shows selectors, exact relative paths, byte counts and SHA-256 hashes
without initializing manager state or touching the game. To adopt, specify a
**single exact selector** and --yes; a preview itself provides no authorization.
A confirmed adoption writes only the manager-owned SQLite ledger (transactional
all-or-nothing), never copies, replaces or alters game files. The installed
package records origin: "manual" and archive_sha256: null (previously
installed ZIP packages retain origin: "archive" and their archive digest).

**Important:** confirmation grants Modn permission to **delete those previously
manual files on a later confirmed uninstall**. Uninstall still verifies file
integrity and refuses changed, missing, linked or aliased files. Manual sibling
files and pre-existing directories are not removed. No automatic adoption or
bulk approval exists. Game modules must explicitly support adoption; unsupported
or ambiguous content remains unmanaged. Filesystem checks do not defeat a hostile
concurrent process swapping paths between verification and use.

Databases from the prior release are migrated by Store::open without changing
existing ownership or archive provenance. Adoption writes no game files and uses
a single SQLite transaction; unlike installation and uninstall, it requires no
filesystem journal or recovery of partial game changes. If the process stops
before SQLite commit, no files become owned; after commit, the entire package
does. Run modn recover --yes only for outstanding install/uninstall journals.

## Scope

Windows first. Minecraft, additional transaction types, game-specific Luau integrations and a package browser remain future milestones.

All first-party code is MIT licensed.
