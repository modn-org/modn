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

Buckets contain mod-package manifests and source metadata. Only the official
`main` bucket may also contain game modules. A bucket
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
```

A game can have multiple installations, each with independent ownership and
transaction state. Mod IDs are qualified by bucket and game when ambiguous.

## Bucket catalog entities (target)

### Bucket

| Field | Meaning |
| --- | --- |
| `bucket_id` | Registered catalog name, for example `main`. |
| `source` | Verified local `main` copy, registered local directory or future Git source. |
| `revision` | Selected catalog snapshot/revision. |
| `title` | Human-readable catalog name. |

The install script runs `modn bucket update` to fetch the official `main` bucket.
Modn never fetches it implicitly. A missing copy prompts the user to run that command.
The update checks the ZIP against its same-origin release SHA-256 sidecar.
Release signing is future work. The update fetches the latest `modn-org/main`
release, and it refuses a bucket with a module this Modn cannot run.
The local copy works offline and changes only on explicit update. Community buckets
cannot contain game modules. Registering a bucket
never authorizes modification of a game installation.

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

### Mod release (package manifest)

| Field | Meaning |
| --- | --- |
| `bucket_id`, `revision` | Originating catalog snapshot. |
| `game_id`, `mod_id` | Game and logical mod identity. |
| `version` | Exact mod release version. |
| `name` | User-facing name. |
| `archive_format` | ZIP or 7z, where supported. |
| `sha256` | SHA-256 of the original distribution archive. |
| `sources` | Download addresses or manual acquisition instructions. |
| `dependencies` | Documented required/optional/conflicting relations. |
| `compatibility` | Verified known game/framework requirements, if any. |

An eventual fully qualified identity may be
`<bucket>/<game>/<mod>@<version>`. **This is a proposed identity**, not a
stable CLI contract. Within one bucket, logical IDs must not collide
silently. The future resolver must detect cycles, missing references,
ambiguous names, and incompatible version requirements.

### Artifact source and verified artifact

An **ArtifactSource** describes an original download URL or a manual archive
source. A **VerifiedArtifact** is a cache entry whose bytes match the pinned
archive hash. Neither is an ownership record for a game installation.

Use normal HTTPS retrieval and local archive import before designing
provider-specific acquisition systems. A checksum match alone is not a
publisher authenticity guarantee.

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
| Catalog provenance | `state.sqlite / catalog_provenance(package_id, bucket, game_id, mod_id)` |
| Dependency links | `state.sqlite / package_dependencies(package_id, dependency_id)` |
| File ownership | `state.sqlite / managed_files(...)` |
| Directory ownership | `state.sqlite / managed_directories(...)` |
| Pending transaction | `state.sqlite / pending_transactions, pending_files, pending_directories` |
| Game module | `main/games/<game-id>/module.json` and `module.lua` in the local bucket copy |
| Development override | In-memory per-invocation `--module-src` / `--src` |
| Bucket packages | Local `main` copy and explicitly registered local directories |

Manager schema version 2 stores each pin as `pinned INTEGER NOT NULL DEFAULT 0`.
Migration preserves existing packages and leaves them unpinned.
A pin belongs to one managed package and disappears when that package is uninstalled.
Pin changes hold the manager transaction lock and change no ownership or game files.

The existing bucket v1 contract validates current releases at
`games/<game-id>/<package-id>.json` and other releases at
`games/<game-id>/<package-id>@<version>.json`. Only `main` publishes game
modules. A mod update is a pending uninstall journal and a pending
install journal; one commit retires the old managed package and moves its
dependency links to the new one. Git-backed catalog sync remains deferred (#53).

## Invariants for future changes

1. Preserve game ownership and pending-transaction evidence.
2. A catalog update cannot silently install, uninstall or update mods.
3. Game-specific Lua policy remains first-party source in `main`.
4. Development override paths never persist or silently fall back.
5. Package manifests and Lua proposals cannot claim pre-existing files.
6. Failed installation and recovery preserve unmanaged game content.
7. Never add a per-game Rust core adapter.
8. Keep manager state outside game installations.
