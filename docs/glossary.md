# Modn

Modn manages game modifications through curated buckets and game-specific
modules. This glossary names the domain concepts used in the CLI, manifests,
Rust core and runtime Lua integrations.

## Language

### Buckets and trust

**Bucket**:
A versioned catalog of mod package manifests under a named source. The official
bucket also contains first-party game modules.
_Avoid_: Mod repository, provider, registry (when referring to a bucket)

**Bucket contract v2**:
The bucket layout and record schema that Modn reads: `bucket.json` with
`schema_version: 2`, package manifests under `mods/<game-id>/`, and game
modules under `games/<game-id>/`.
_Avoid_: Legacy bucket, mixed `games/` folder

**Official bucket**:
The Modn-maintained `main` catalog and first-party game modules. Modn uses a
local copy installed by `modn bucket update`. The install script runs that command.
`main` is always a trusted module source. Its source and releases are in the
`modn-org/main` repository.
_Avoid_: Built-in module, automatically safe mods

**Community bucket**:
A mod-package catalog maintained by a third party and explicitly registered
by the user.
_Avoid_: Untrusted bucket (after trust is granted), unofficial plugin

**Bucket registration**:
The local record that makes a named bucket available for resolution.
_Avoid_: Bucket installation, mod installation

**Manager roots**:
The one value that holds the shared data directory, the manager profile selected
by `--state-dir`, the archive cache and the development module source. A command
resolves it once. The official `main` copy and every other manager-owned location
follow the profile, so `--state-dir` is self-contained. Without `--state-dir` the
profile is the shared data directory.
_Avoid_: Ambient root, per-call directory, thread-local state

**Bucket snapshot**:
A specific set of bucket definitions and manifests at a known revision.
_Avoid_: Live catalog (when referring to a pinned version)

### Game support

**Game**:
A game identity, such as `cyberpunk-2077`, recognized by a game module.
_Avoid_: Game installation

**Game installation**:
One concrete directory containing an installed copy of a game.
_Avoid_: Game, game ID

**Store identity**:
The identity of a game in one store, such as Steam app `1091500` for Cyberpunk
2077. A game module declares it as data in `module.json`.
_Avoid_: Store ID mapping, discovery path

**Store library**:
The record of installed games that a store keeps, such as the Steam
`libraryfolders.vdf` and app manifests. Rust reads it; a game module confirms
each folder it proposes.
_Avoid_: Game library, scan root

**Launch target**:
The data in a game module that starts one installation: a store URI for a store
that detection matched, or a game-relative `.exe`. Rust validates the target;
Lua has no process API.
_Avoid_: Game command line, arbitrary executable

**Game module**:
A versioned Lua integration that defines game-specific discovery,
installation policy, diagnostics and tool behavior.
_Avoid_: Mod, plugin, adapter, Rust game handler

**Trusted third-party game module**:
A user-approved executable Lua policy. It comes from a trusted signed Git or
ZIP catalog or a local `module register --trust` source and is stored as a versioned
release in a separate local registry.
_Avoid_: Community bucket module, automatically safe module

**First-party game module**:
A Lua 5.4 integration owned by Modn and distributed in the official bucket.
_Avoid_: Independently installed game plugin

**Module release**:
A versioned game-module release: the required `version` plus the BLAKE3 digest
of `module.json` and its entrypoint. The registry retains releases and activates
one; pin and rollback change which one future plans use.
_Avoid_: Catalog revision, unversioned snapshot

**Trusted module source**:
A bucket that may supply executable game modules: the official `main` bucket, or
a signed Git or ZIP catalog the user trusts with `modn bucket trust-modules`.
Trust covers the whole bucket. A signed rotation moves it to the successor key;
a revocation or `untrust-modules` ends it.
_Avoid_: Any registered bucket, unsigned catalog

**Development module override**:
A temporary caller-selected Lua module source for one command, never persisted.
_Avoid_: Installed module, active third-party module

**Trusted module registration**:
An explicitly accepted module release of third-party Lua policy in the runtime
data directory. Its version and retained digest detect changed bytes.
Registration grants no game-write approval.
_Avoid_: Sandboxed module, bucket module, development override

**Git catalog revision**:
The explicit source and full commit published with a validated community catalog snapshot.
Only an explicit `bucket sync` fetches this source.
A ZIP catalog revision records the ZIP source and digest instead of a commit.
Both kinds record the pinned signer key ID, the pinned revocation key ID and the
previous or revoked signer key IDs.
_Avoid_: Automatically refreshed catalog

**Mirror**:
Another host of the same signed catalog, listed in `mirrors.json`. Sync tries
the registered primary first and then the stored mirrors in order. A failure to
open, download or unpack a source moves to the next mirror, including a broken
archive. A trust failure during signature verification stops the sync.
_Avoid_: Another catalog, fallback archive source

**Catalog relocation**:
A signed `relocation.json` record that names a new primary host for the same
catalog. After verification, Modn updates the registration in one atomic write,
and the next sync contacts the new primary first.
_Avoid_: Redirect, unsigned URL change

**Key rotation**:
A signed `rotation.json` statement from the pinned signing key that names its
successor. Modn pins the successor, records the old key ID in
`previous_signers`, and moves module trust to the successor.
_Avoid_: Replacing `catalog.pub` without a statement

**Revocation key**:
An offline minisign key, pinned at first sync from `revocation.pub`, that signs
`revocation.json` to revoke the signing key and name its replacement. It never
signs a catalog.
_Avoid_: Signing key, backup password

**Catalog signature**:
A minisign (Ed25519) signature over the signed catalog statement.
Modn verifies it before it uses a distributed catalog.
_Avoid_: Archive checksum, transport security

**Signed catalog statement**:
The canonical text that a catalog signature covers: format
`modn-catalog-signature/v1`, the catalog name and the catalog digest.
_Avoid_: Package manifest, release notes

**Catalog digest**:
The BLAKE3 digest of a distributed catalog's files, shown as `blake3:<hex>`.
It excludes the root `catalog.sig` and `catalog.pub` files.
_Avoid_: Archive checksum, commit ID

**Signer key**:
The minisign public key whose signatures Modn accepts for one catalog.
`main` uses the key built into Modn. A Git or ZIP catalog pins its signer key at
first sync and changes it only through a signed rotation or revocation.
_Avoid_: Certificate, user account

**Key ID (fingerprint)**:
The 16 uppercase hexadecimal digits that name a minisign key, for example
`0E250AD52A3BA2AA`. Modn shows the key ID as the signer fingerprint.
_Avoid_: Full public key, commit ID

### Themes

**Theme**:
A validated set of CSS and assets that changes how the Modn desktop looks
for one game. Rust validates it; it cannot run code, load remote resources or
change a destructive confirmation.
_Avoid_: Skin, mod, plugin

**Theme variant**:
An optional `theme.<variant>.css` stylesheet that applies after a theme's
`theme.css`. The theme lists its variants in `theme.json`.
_Avoid_: Theme mode, preset

**Official theme**:
A theme that the official `main` bucket ships for one game. It is the fallback
when a game has no selected theme.
_Avoid_: Default theme, built-in theme

**Local theme**:
A theme that the theme registry holds after `modn theme register <directory>`.
A theme from any other bucket stays a bucket theme.
_Avoid_: Installed theme, managed package

**Theme registry**:
The Modn-owned copy of each registered local theme under the data directory
`themes/registry/`. It records the id, game, BLAKE3 digest, source label and
registered-at time, and it checks the digest on every load.
_Avoid_: Bucket, catalog

**Theme selection**:
The Modn state record of one game's theme id and optional theme variant. The
desktop reads it; it never stores the choice.
_Avoid_: Desktop setting, theme configuration

**Protected region**:
A desktop subtree that a theme may restyle only with color and font properties:
the install review and the approval prompt. A theme cannot change a destructive
confirmation.
_Avoid_: Locked theme, hidden area

### Packages and acquisition

**Mod**:
A user-visible modification or extension for a game, independent of how it is
distributed.
_Avoid_: Archive, package file

**Package manifest**:
A bucket record describing a mod release, its game, version, artifact sources
and known requirements.
_Avoid_: Module manifest, mod archive

**Mod release**:
A specific version of a mod identified in a bucket.
_Avoid_: Game module, downloaded archive

**Archive**:
The original ZIP or 7z file containing content offered for installation.
_Avoid_: Installed mod, package manifest

**Archive pin**:
The BLAKE3 digest in a package manifest that fixes the archive bytes. Modn
verifies a downloaded or supplied archive against it.
_Avoid_: Archive checksum, upstream SHA-256

**Artifact source**:
The declared location or manual-acquisition instruction for an archive.
_Avoid_: Bucket, install destination

**Nexus source**:
A package source that pins one Nexus Mods file by game domain, mod ID, file ID
and file name. Modn downloads it from a validated Nexus CDN host and verifies
the archive against its BLAKE3 pin.
_Avoid_: Manual source, HTTPS source

**nxm link**:
The link from the Nexus "Mod Manager Download" button. It carries a short-lived
grant that lets a free Nexus account download one file. Modn matches it to a
package by game domain, mod ID and file ID.
_Avoid_: API key, download URL

**Nexus API key**:
The personal key that Modn sends only to `api.nexusmods.com` to request a Nexus
download link. `modn auth nexus set` stores it in the operating system
credential store, and `MODN_NEXUS_API_KEY` overrides the stored key.
_Avoid_: nxm link, password

**Verified artifact**:
An archive whose bytes match the digest pinned by its package manifest.
_Avoid_: Safe mod, trusted publisher

**Verified archive reuse**:
Reuse of an immutable cache payload after its bytes match another package or source alias's pinned BLAKE3 digest.
It grants no ownership and never deduplicates mutable game files.
_Avoid_: Game-file deduplication, trusted publisher

**Dependency**:
A declared relationship in which a mod release needs another mod or framework
release to function.
_Avoid_: Download order, install order (without an actual requirement)

**Installation plan**:
A resolved proposal of packages, versions, sources, destination operations,
conflicts and required approvals before any game files change.
_Avoid_: Installation, transaction

**Native injector**:
A mod release whose manifest sets `native_injector: true` because it places a
native library that the game process loads. Modn refuses to install or update it,
including as a dependency, unless the caller passes `--allow-native-injector`.
_Avoid_: Any DLL mod, trusted publisher

### Installed state and recovery

**Managed package**:
A package whose installation and owned targets are recorded in Modn's state.
_Avoid_: Bucket entry, detected mod, unmanaged mod

**Ownership claim**:
A durable record assigning responsibility for a specific file or exclusive
directory to a managed package in a particular game installation.
_Avoid_: Discovery result, package source

**Exclusive directory**:
A newly created mod directory owned as a whole by one managed package,
including runtime-generated contents inside that directory.
_Avoid_: Shared directory, file-by-file claim

**Shared location**:
A game directory where Modn uses individual-file ownership rather than
claiming the entire directory for one package.
_Avoid_: Exclusive directory

**Unmanaged content**:
Files or directories in a game installation that Modn did not claim through a
recorded installation or acknowledged adoption.
_Avoid_: Invalid mod, orphaned package

**Pending transaction**:
An incomplete recorded install or uninstall whose journal must be resolved
before another managed write can proceed.
_Avoid_: Installed package, failed mod

**Recovery**:
The continuation or rollback of a pending managed transaction using recorded
ownership and transaction evidence.
_Avoid_: Game repair, automatic cleanup

**Adoption**:
An acknowledged atomic ownership commit for existing individual files that match a selected local archive.
It changes no game bytes and claims no directory or unknown descendant.

**Owned cleanup**:
Explicitly approved removal of one managed package's recorded files, including modified files.
It preserves unrecorded descendants and retains transaction backups and recovery checks.

**Trusted module registration**:
An explicitly accepted module release of third-party Lua policy in the runtime
data directory. Its version and retained digest detect changed bytes.
Registration grants no game-write approval.
_Avoid_: Sandboxed module, bucket module, development override

**Git catalog revision**:
The explicit source and full commit published with a validated community catalog snapshot.
Only an explicit `bucket sync` fetches this source.
_Avoid_: Automatically refreshed catalog

**Verified archive reuse**:
Reuse of an immutable cache payload after its bytes match another package or source alias's pinned BLAKE3 digest.
It grants no ownership and never deduplicates mutable game files.
_Avoid_: Game-file deduplication, trusted publisher

**Adoption**:
An acknowledged atomic ownership commit for existing individual files that match a selected local archive.
It changes no game bytes and claims no directory or unknown descendant.

**Owned cleanup**:
Explicitly approved removal of one managed package's recorded files, including modified files.
It preserves unrecorded descendants and retains transaction backups and recovery checks.
## Content root

A module-declared directory outside the game installation that holds mod content, such as `%APPDATA%\Balatro\Mods` for Balatro.
A module declares it in `module.json` as `content_root` with `base` (`appdata` only) and a relative `path`.
Rust resolves and validates it, records it as the `content` installation root, and qualifies every owned file, owned directory and created folder by that root.
A plan operation selects it with `"root": "content"`.
