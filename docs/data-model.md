# Modn data model

This document describes the **target** bucket-first domain model and maps it to
Modn's existing implementation. It is a design contract, not a database
migration or a claim that the target is shipped. See [glossary.md](glossary.md)
for preferred terms and [vision.md](vision.md) for product direction and
where planned work is tracked.

## Domain boundaries

Modn separates **first-party executable game support**, **catalog metadata**,
and **managed installation state**.

First-party Lua 5.4 modules live at `games/<game-id>/` of the `modn-org/main`
bucket repository as `module.json` and `module.lua`. Each `module.json`
declares the host API version that it needs. Modn embeds no bucket manifests or modules.
The development-only `--module-src` flag selects a local source for one command;
it does not change any persistent selection or stored installation record.

Buckets contain mod-package manifests and source metadata. The official `main`
bucket and the signed Git catalogs the user trusts for modules may contain game
modules. Modn ignores modules from other buckets, and local folder buckets
cannot be module sources. Trusted third-party modules are imported into a
versioned registry after explicit approval. A bucket
record is not evidence of installed content; an archive hash proves exact
bytes, not harmlessness. Catalog updates cannot modify game files.
Rust owns managed transactions, file and directory ownership, rollback,
recovery and path validation; Lua supplies game-specific policy and proposals.

## Relationships

```text
FirstPartyGameModule -> Game -> GameInstallation
Bucket -> BucketSnapshot -> ModRelease -> Game
                                     -> ArtifactSource
                                     -> Dependency -> ModRelease
GameInstallation -> ManagedPackage -> OwnershipClaim
                 -> PendingTransaction
                 -> ModApproval
```

A game can have multiple installations, each with independent ownership and
transaction state. Mod IDs are qualified by bucket and game when ambiguous.

## Bucket catalog entities (target)

### Bucket

| Field | Meaning |
| --- | --- |
| `bucket_id` | Registered catalog name, for example `main`. |
| `source` | Verified local `main` copy, registered local directory or explicitly synchronized Git source. |
| `revision` | Selected catalog snapshot/revision. |
| `title` | Human-readable catalog name. |
| `module_trust` | Stored signer key ID that accepts this bucket's module code. A trusted signed Git catalog has this record; other buckets do not. |

The install script runs `modn bucket update` to fetch the official `main` bucket.
Modn never fetches it implicitly. A missing copy prompts the user to run that command.
The update verifies a minisign signature over the catalog digest with the signer
key built into Modn. It refuses an unsigned or invalid release and keeps the
installed copy.
The update fetches the latest `modn-org/main`
release, and it refuses a bucket with a module this Modn cannot run.
The local copy works offline and changes only on explicit update. `main` and the
signed Git catalogs the user trusts for modules may contain game modules. Modn
ignores modules from other buckets, and local folder buckets cannot be module
sources. Registering a bucket never authorizes modification of a game installation.
A trusted catalog's `module_trust` record names its signer key ID; a signer key
change ends trust.

### Game and game installation

**Game** has a stable `game_id` such as `cyberpunk-2077`.
A **GameInstallation** has `game_id`, canonical `path`, and registration
or discovery `source`. Its validation and layout are supplied by Modn's
first-party Lua policy. Do not confuse an identity with a physical installation.

### First-party game module

Each supported game has one `module.json` and `module.lua` in `main`.
The manifest declares `game_id`, `version`, `api_version` and
`entrypoint`. Explicit bucket updates can change supported game policies.
A caller can explicitly provide a temporary alternative source for tests;
a missing or mismatched override fails without falling back.

### Game module release

A **GameModuleRelease** is one version of a trusted third-party module. Its
identity is the required `version` plus the SHA-256 digest of `module.json` and
its entrypoint. The registry keeps every retained release and one active release:

```text
<data>/game-modules/<game-id>/
  current                      # active version string
  pin.json                     # optional pin: version and digest
  versions/<version>/
    module.json
    module.lua
    registration.json          # identity, digest and source receipt
```

`current` and `pin.json` are manager metadata. Pin and rollback change only
these pointers, so they affect future plans and never installed files. Import
stages a release in private storage and switches `current` atomically. A failed
import keeps the previous release. A retained version with a different digest is
refused. An unversioned entry from an earlier release is refused and must be
registered again.

### Mod release (package manifest)

| Field | Meaning |
| --- | --- |
| `bucket_id`, `revision` | Originating catalog snapshot. |
| `game_id`, `mod_id` | Game and logical mod identity. |
| `version` | Exact mod release version. |
| `name` | User-facing name. |
| `archive_format` | ZIP or 7z, where supported. |
| `sha256` | SHA-256 of the original distribution archive. |
| `sources` | Download addresses, pinned provider selectors or manual acquisition instructions. |
| `dependencies` | Documented required/optional/conflicting relations. |
| `native_injector` | True when the release loads native code into the game process; the caller must opt in. |
| `compatibility` | Verified known game/framework requirements, if any. |

An eventual fully qualified identity may be
`<bucket>/<game>/<mod>@<version>`. **This is a proposed identity**, not a
stable CLI contract. Within one bucket, logical IDs must not collide
silently. The current resolver checks cycles, missing references, ambiguous names and supported version requirements.

### Artifact source and verified artifact

An **ArtifactSource** describes an HTTPS URL, a pinned GitHub release asset, a
pinned Nexus Mods file, a Git commit or a manual archive source. GitHub
selectors contain repository, tag and asset names. A Nexus selector contains
the game domain, mod ID, file ID and file name. Approved acquisition validates
bounded provider metadata before verified retrieval. A Nexus download sends the
API key only to `api.nexusmods.com` and, for a free account, carries the nxm
link grant. The key and grant never enter a cache name, state record or journal.
Ordinary dry runs remain offline and read-only. A **VerifiedArtifact** is a cache entry whose bytes match the pinned
archive hash. Neither is an ownership record for a game installation.

Verified cache reuse can return an existing payload for another package or source alias with the same pinned digest.
Every reuse verifies bytes. It creates no alias file and never deduplicates mutable game files.
A checksum match alone is not a publisher authenticity guarantee.

## Installation and ownership entities

### Installation plan

A transient proposal containing the chosen game installation, first-party module,
exact bucket snapshot and releases, verified archive digests, dependencies,
destination operations, conflicts and explicit approvals required.

Preview is not installation. A trusted Lua module can propose operations, but
Rust must validate and stage managed writes before recording ownership.

### Managed package

A committed installation record for a specific game installation, linked to
an original archive digest and its ownership claims. Catalog installations
also record their originating `bucket/game/mod` and the managed packages that
satisfied their dependencies, committed atomically with ownership. Recorded
dependencies cannot be uninstalled while a dependent remains. Provenance does
not replace the ownership evidence.

### Mod approval

A **ModApproval** records the capabilities of one catalog mod that the user
approved for one game installation. Its identity is `(game_path, mod_id)`:

- `native_injector` records that native code is approved.
- `executables` is the set of normalized game-relative executable payload
  paths that are approved.

An install or update whose release capabilities the approval covers needs no
flag. A release that adds native code or an executable path is refused with
`NATIVE_INJECTOR_NOT_ALLOWED` or `APPROVAL_REQUIRED` until the user passes the
matching flag or confirms the executable paths interactively. A committed
operation then stores or widens the approval. A cancelled, refused, dry-run or
rolled-back operation stores nothing. Uninstall keeps the approval. A release
that drops a capability does not shrink it. Only the CLI backend writes
approvals; the MCP and desktop surfaces inherit the stored decision. There is
no approval revoke command.

### Ownership claim

A recorded file or exclusive directory target owned by one managed package
in one game installation.

- In a newly created per-mod directory, the claim covers the directory and
  its runtime-generated contents; cleanup checks directory identity.
- In a shared game location, Modn claims **individual files**, including
  their paths, size and digest.
- Pre-existing files and directories are not silently adopted or overwritten.
  Unmanaged content stays outside managed uninstall authority.

### Pending transaction and recovery

An incomplete install or uninstall journal identifies the action, game
installation, package and staged file/directory operations. Recovery uses
journal evidence and actual disk state, not a mutable bucket definition.
Pending transactions block conflicting managed writes until resolved.

### Operation history

A durable record of one committed install, update or uninstall. It names the
game installation and per package the mod ID, package IDs and release sides
before and after, each release's source descriptor and archive SHA-256 and
BLAKE3 digests, and every owned file the operation created, replaced or
removed with its content digest before and after. A record written by an undo
also names the operation it reversed. History is evidence for undo, not an
ownership claim: it never grants authority over a file. The record is committed
in the same SQLite transaction as the ownership change it describes, so
rollback, cancellation and recovery leave no disagreeing record.

Lua modules may invoke powerful game-specific integrations. Do not represent
those external effects as a committed managed Rust transaction unless they
actually used Modn's managed-install path.

## Current implementation mapping

The host runs Lua 5.4 through `mlua`. First-party Cyberpunk source is
`games/cyberpunk-2077/` in `modn-org/main`. This repository pins a bucket commit
as the `buckets/main` submodule for its tests. The synthetic module
fixture exercises game-agnostic support.

| Domain concept | Current data |
| --- | --- |
| Game installation | `state.sqlite / game_installations(game_id, path, source)` |
| Managed package | `state.sqlite / managed_packages(id, game_path, archive_sha256, created_at, pinned)` |
| Mod approval | `state.sqlite / mod_approvals(game_path, mod_id, native_injector, executables)` |
| Catalog provenance | `state.sqlite / catalog_provenance(package_id, bucket, game_id, mod_id)` |
| Dependency links | `state.sqlite / package_dependencies(package_id, dependency_id)` |
| Installation root | `state.sqlite / installation_roots(game_path, root, path, path_key)` |
| File ownership | `state.sqlite / managed_files(game_path, root, path_key, ...)` |
| Directory ownership | `state.sqlite / managed_directories(game_path, root, path_key, ...)` |
| Created folder | `state.sqlite / created_directories(game_path, root, path_key, ...)` |
| Pending transaction | `state.sqlite / pending_transactions, pending_files, pending_directories` |
| Package release metadata | `state.sqlite / managed_package_releases(package_id, mod_id, version, source, archive_blake3)` |
| Operation history | `state.sqlite / history_operations, history_packages, history_files` |
| Official game module | `main/games/<game-id>/module.json` and `module.lua` in the local bucket copy |
| Retained module release | `<data-directory>/game-modules/<game-id>/versions/<version>/`, with a verified `registration.json` receipt |
| Active or pinned module release | `<data-directory>/game-modules/<game-id>/current` and `pin.json` |
| Bucket module trust | `buckets/registrations/<name>.json / module_trust.signer_key_id` |
| Development override | In-memory per-invocation `--module-src` / `--src` |
| Bucket packages | Local `main` copy, registered local directories and explicitly synced Git snapshots |

Manager schema version 2 stores each pin as `pinned INTEGER NOT NULL DEFAULT 0`.
Migration preserves existing packages and leaves them unpinned.
A pin belongs to one managed package and disappears when that package is uninstalled.
Pin changes hold the manager transaction lock and change no ownership or game files.

### Installation roots (schema version 4)

Manager schema version 4 records the roots of each installation.
Schema version 3 introduced the root model with the `game` root.
Version 4 adds the `content` root and records the root label on pending journal rows.
An installation is identified by its canonical `game_path`.
A root is identified by its label within that installation and by its canonical `path`.
The labels are `game` and, for an API 2 module that declares `content_root`, `content`.
The game root's path is the installation path.
The content root is resolved from the declared appdata base and recorded when an installation first writes content there.

- Every owned file, owned directory and created folder names its root.
  Its ownership key is `(game_path, root, path_key)`.
- `path_key` in `installation_roots` is the ownership key of the canonical root path.
  It is unique, so two installations cannot record one canonical root.
  On Windows the key folds case, so case spellings of one folder collide.
- Modn records the game root when it registers a game and when an install commits ownership.
  It records a content root before staging files below it.
- Pending journal rows name the root of each file, directory and created folder.
  A created folder may instead name an appdata base, so recovery removes only folders the interrupted transaction created.
- Transactions and recovery resolve the root through its record.
  If the recorded path is missing, is a link or junction,
  or now canonicalizes to a different path, the operation refuses and writes nothing.
- An installation without a recorded root has committed no ownership.
  It resolves its game root as before, from the canonical game path.

A writable open migrates version 1, 2 or 3 state in the same transaction that opens it.
The migration assigns every owned path to the `game` root, records one game root per installation, and adds root labels to pending rows.
If any step fails, for example because two recorded installations fold to one root,
SQLite rolls the transaction back and the old state stays unchanged and readable.
A second open of migrated state changes nothing.
A read-only open of version 1 or 2 state presents the same rows through temporary views and writes nothing.

Operation history and the package release metadata are additive tables in the
same schema version 4. A writable open creates them when missing, so a state
database from an earlier build opens unchanged. Packages recorded before these
tables existed have no release metadata; their uninstall history names the
archive SHA-256 without a version or source. See [history.md](history.md).

Older Modn builds accept only schema versions 1 and 2.
They refuse version 3, 4 or 5 state with `unsupported manager state schema; start with a fresh state directory`.
They do not misread it, but that message suggests a fresh state directory.
Do not follow that advice for state that holds ownership; install a current build instead.

### Approvals (schema version 5)

Manager schema version 5 adds the `mod_approvals` table. The table is keyed by
`(game_path, mod_id)` and stores a native-injector flag and a JSON array of
normalized executable payload paths. A writable open creates it in the same
transaction as the rest of the schema and leaves schema version 4 state
readable. A read-only open of version 4 state sees an empty approval table and
writes nothing. Approvals are independent of ownership, managed packages and
pending transactions; uninstall and recovery do not change them.

The existing bucket v1 contract validates current releases at
`games/<game-id>/<package-id>.json` and other releases at
`games/<game-id>/<package-id>@<version>.json`. `main` and trusted signed Git
catalogs may publish game modules. A mod update is a pending uninstall journal and a pending
install journal; one commit retires the old managed package and moves its
dependency links to the new one. Explicit Git-backed community catalog sync is available; see [sources.md](sources.md).

## Invariants for future changes

1. Preserve game ownership and pending-transaction evidence.
2. A catalog update cannot silently install, uninstall or update mods.
3. First-party Lua policy remains in `main`; trusted third-party policy comes from trusted signed catalogs or local `--trust` registration and uses a separate versioned registry.
4. Development override paths never persist or silently fall back.
5. Normal install cannot claim pre-existing files. Explicit adoption requires acknowledgement and exact archive-byte verification.
6. Failed installation and recovery preserve unmanaged game content.
7. Never add a per-game Rust core adapter.
8. Keep manager state outside game installations.

## Explicit lifecycle and acquisition extensions

[sources.md](sources.md) specifies Git catalog revisions, digest-based cache reuse and pinned GitHub release acquisition.
[lifecycle.md](lifecycle.md) specifies acknowledged file adoption and approved cleanup of modified owned files.
Adoption commits individual ownership without writing game bytes. Cleanup retains journals, backups and recovery checks.

### Trusted module registration and trust

`module register <source> --trust` imports a validated third-party module release.
The manager profile stores its identity, API version, exact content digest
and source. The profile is the folder that holds `state.sqlite`, so `--state-dir`
selects it; without `--state-dir` it is the runtime data directory. The registry is
independent of development source overrides. A catalog sync imports a trusted bucket's releases into the same
registry. Official game IDs retain priority and cannot be imported from a trusted
bucket or registered locally. Changed bytes fail verification.

A bucket is a module source when it is `main`, or when a signed Git catalog has a
`module_trust` record whose signer key ID matches its pinned signer. A signer key
change ends trust. Modules from other buckets are ignored at sync and refused at
use. Doctor reports `MODULE_HIDDEN_BY_OFFICIAL` for a catalog module that `main`
hides, and `MODULE_SOURCE_UNTRUSTED` for a retained module whose bucket is no
longer trusted. Unversioned entries from earlier releases are refused; remove and
register the module again. Removal leaves game files and ownership intact.


## Companion and desktop boundaries

The local MCP companion fixes its CLI backend, manager state, and approved filesystem roots at startup.
Server `--allow-writes` and per-request `approved: true` authorize ordinary installation or uninstall.
It exposes neither adoption nor forced cleanup.
The GPUI Kit 0.9.0 desktop belongs to the root workspace, with Windows-only GPUI dependencies.
It reviews local ZIP and 7z archives and confirms ordinary installation or strict uninstall through the CLI.
Neither surface derives ownership from scan results or grants executable approval.
See [MCP scopes](mcp.md) and [desktop configuration](desktop.md).


## Explicit lifecycle commands

`adopt <archive> --acknowledge --yes` records matching manual files in the existing managed-package and file-ownership tables.
It records archive provenance without catalog identity or dependency links. No schema field distinguishes adoption from installation.
The read-only preview reports pending byte, content, and ownership verification.
`uninstall <selector> --force --yes` resolves one managed package and removes only its recorded files.
It preserves unrecorded descendants and retains exclusive directories. Normal uninstall remains strict.
The cleanup journal records current bytes for rollback and recovery.
See [lifecycle contracts](lifecycle.md) and [MCP startup](mcp.md).
