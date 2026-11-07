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

**Official bucket**:
The Modn-maintained `main` catalog and first-party game modules. Modn uses a
local copy installed by `modn bucket update`. The install script runs that command.
Only `main` contains modules. Its source and releases are in the
`modn-org/main` repository.
_Avoid_: Built-in module, automatically safe mods

**Community bucket**:
A mod-package catalog maintained by a third party and explicitly registered
by the user.
_Avoid_: Untrusted bucket (after trust is granted), unofficial plugin

**Bucket registration**:
The local record that makes a named bucket available for resolution.
_Avoid_: Bucket installation, mod installation

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

**Game module**:
A versioned Lua integration that defines game-specific discovery,
installation policy, diagnostics and tool behavior.
_Avoid_: Mod, plugin, adapter, Rust game handler

**Trusted third-party game module**:
A user-approved executable Lua policy copied into a separate local registry.
_Avoid_: Community bucket module, automatically safe module

**First-party game module**:
A Lua 5.4 integration owned by Modn and distributed in the official bucket.
_Avoid_: Independently installed game plugin

**Development module override**:
A temporary caller-selected Lua module source for one command, never persisted.
_Avoid_: Installed module, active third-party module

**Trusted module registration**:
An explicitly accepted snapshot of third-party Lua policy in the runtime data directory.
Its retained digest detects changed bytes. Registration grants no game-write approval.
_Avoid_: Sandboxed module, bucket module, development override

**Git catalog revision**:
The explicit source and full commit published with a validated community catalog snapshot.
Only an explicit `bucket sync` fetches this source.
_Avoid_: Automatically refreshed catalog

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

**Artifact source**:
The declared location or manual-acquisition instruction for an archive.
_Avoid_: Bucket, install destination

**Verified artifact**:
An archive whose bytes match the digest pinned by its package manifest.
_Avoid_: Safe mod, trusted publisher

**Verified archive reuse**:
Reuse of an immutable cache payload after its bytes match another package or source alias's pinned SHA-256.
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
An explicitly accepted snapshot of third-party Lua policy in the runtime data directory.
Its retained digest detects changed bytes. Registration grants no game-write approval.
_Avoid_: Sandboxed module, bucket module, development override

**Git catalog revision**:
The explicit source and full commit published with a validated community catalog snapshot.
Only an explicit `bucket sync` fetches this source.
_Avoid_: Automatically refreshed catalog

**Verified archive reuse**:
Reuse of an immutable cache payload after its bytes match another package or source alias's pinned SHA-256.
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
An API 2 module declares it in `module.json` as `content_root` with `base` (`appdata` only) and a relative `path`.
Rust resolves and validates it, records it as the `content` installation root, and qualifies every owned file, owned directory and created folder by that root.
A plan operation selects it with `"root": "content"`.
