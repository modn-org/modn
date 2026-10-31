# Modn data model

This document describes the **target** bucket-first domain model and maps it to
Modn's existing implementation. It is a design contract, not a database
migration or a claim that the target is shipped. See [glossary.md](glossary.md)
for preferred terms and [roadmap.md](roadmap.md) for milestone order.

## Domain boundaries

Three kinds of state must remain distinct:

- **Catalog**: available game modules and mod releases, published by buckets.
- **Selection and trust**: community buckets the user trusts, and the active
  module release chosen for each game.
- **Managed state**: specific game installations, committed package ownership
  and interrupted transactions.

A catalog entry is not evidence that a mod is installed. An archive hash
proves matching bytes, not benign behavior. Trusting executable module code
does not by itself grant an ownership claim on existing game files.

Rust manages generic install transactions, ownership and persistence.
Trusted Lua modules provide game-specific behavior. Normal managed installs
continue through the Rust transaction engine even when modules are powerful.

## Relationships

```text
Bucket -> BucketSnapshot -> GameModuleRelease -> Game
                         -> ModRelease -> Game
                                       -> ArtifactSource
                                       -> Dependency -> ModRelease
BucketTrust -> Bucket
ActiveModule -> GameModuleRelease

Game -> GameInstallation -> ManagedPackage -> OwnershipClaim
                         -> PendingTransaction
```

A game identity can have multiple installations. The active module is selected
per game; managed packages and ownership claims belong to **one concrete game
installation**. Mod IDs need a bucket qualifier to avoid ambiguity.

## Bucket catalog entities (target)

### Bucket

| Field | Meaning |
| --- | --- |
| `bucket_id` | Local name, for example `main` or `community`. |
| `source` | Built-in snapshot, local directory or Git repository. |
| `source_identity` | Stable identity of the repository or directory. |
| `revision` | Version/commit of the selected catalog snapshot. |
| `title` | Human-readable bucket name. |
| `is_official` | True only for Modn's curated `main` bucket. |

The official bucket is trusted by default. A third-party bucket cannot become
official by using the same name. Updating catalog contents **does not**
silently install mods or activate new module versions.

### Bucket trust

| Field | Meaning |
| --- | --- |
| `source_identity` | Community bucket source being acknowledged. |
| `trusted` | Whether its executable modules may be activated. |
| `acknowledged_at` | Time of the user's explicit acceptance. |

Adding a community bucket shows a clear one-time warning: it may supply Lua
modules with broad local capabilities. Trust applies to the bucket maintainer,
not to each individual host capability. A changed source identity must not
inherit trust just because it reuses the same bucket name.

Bucket trust does not imply approval of managed installation writes.

### Game and game installation

**Game** has a stable `game_id` such as `cyberpunk-2077`. A **GameInstallation**
has `game_id`, canonical `path`, and registration/discovery `source`.
Game detection and path interpretation belong in the game module.

Do not confuse a game identity with a physical install location. Two
installations of the same game must have independent managed state.

### Game module release

| Field | Meaning |
| --- | --- |
| `bucket_id`, `revision` | Originating bucket snapshot. |
| `game_id` | Game handled by the module. |
| `module_version` | Version of the game integration. |
| `api_version` | Required Modn host contract. |
| `entrypoint` | Lua script, intended `main.lua`. |
| `content_sha256` | Digest of the module release content. |

A module release is **executable game integration code**, not a mod archive.
Both official and community buckets can publish module releases. Only
acknowledged community buckets can supply active third-party modules.

**ActiveModule** identifies a selected release per `game_id`, plus a previous
selection for rollback. Installing or updating catalog metadata does not
change this selection without an explicit action.

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

A transient proposal containing the chosen game installation, active module,
exact bucket snapshot and releases, verified archive digests, dependencies,
destination operations, conflicts and explicit approvals required.

Preview is not installation. A trusted Lua module can propose operations, but
Rust must validate and stage managed writes before recording ownership.

### Managed package

A committed installation record for a specific game installation, linked to
an original archive digest and its ownership claims. Later, record the
originating bucket/release for provenance without replacing the existing
ownership evidence.

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

## Current implementation mapping (2026-10-31)

The current shipped runtime uses **Luau**, not the target Lua 5.4. It has a
separate module store; bucket-managed modules are **not yet implemented**.

| Domain concept | Current data |
| --- | --- |
| Game installation | `state.sqlite / game_installations(game_id, path, source)` |
| Managed package | `state.sqlite / managed_packages(id, game_path, archive_sha256, created_at)` |
| File ownership | `state.sqlite / managed_files(game_path, path_key, relative_path, package_id, sha256, bytes)` |
| Directory ownership | `state.sqlite / managed_directories(game_path, path_key, relative_path, package_id, identity)` |
| Pending transaction | `state.sqlite / pending_transactions, pending_files, pending_directories` |
| Module releases | `game-modules/modules.sqlite / packages(game_id, version, api_version, entrypoint, sha256, manifest_sha256, script_sha256)` |
| Module selection | `game-modules/modules.sqlite / selections(game_id, active, previous, enabled)` |

**Merged PR #23** introduced a metadata-only bucket registry and a bundled snapshot
of three Cyberpunk prerequisite packages. Its initial package manifest uses
`schema_version`, `id`, `game`, `name`, `version`, `archive_format`,
`sha256`, `source`, and optional `dependencies`/`homepage`. It does not
yet provide `GameModuleRelease` in buckets, persisted community trust,
multi-source downloads, versioned dependency solving or a remote official
repository. Do not treat the proposed model above as its implemented schema.

## Invariants for future migrations

1. Preserve ownership and pending-transaction evidence during schema changes.
2. A catalog update cannot silently install, uninstall or update anything.
3. An activated module has a selected version and identifiable bucket source.
4. Community modules require an explicit bucket trust decision.
5. A package manifest or module result cannot claim existing files as owned.
6. Failed installation and recovery must preserve unmanaged game content.
7. Never move game-specific policy into a per-game Rust core adapter.
8. Keep user data and manager-state files outside game installations.
