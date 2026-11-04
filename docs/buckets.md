# Buckets

## The official main bucket

Modn reads the official bucket from `<data dir>/buckets/main/`. If it is missing,
Modn tells the user to run `modn bucket update`. Modn never fetches it implicitly.
The install script runs that command once. The local copy works offline.
`MODN_MAIN_BUCKET_SOURCE` selects an HTTPS URL or local ZIP for an update.

`modn bucket update` downloads the latest release's `main-bucket.zip` and
`main-bucket.zip.sha256` over HTTPS. It verifies the digest and validates the
bucket and modules before an atomic swap. The checksum shares the release origin,
as in Scoop. Release signing is future work. Failure preserves the previous copy.
Only `main` contains modules. The separate `modn-org/main` repository is planned in #69.
Modn refreshes the copy only when requested. Third-party game modules (#51)
and Git-backed catalog sync (#53) remain deferred.

## Catalog contents

The official `main` bucket catalogs thirteen Cyberpunk
packages: RED4ext, redscript, Cyber Engine Tweaks, ArchiveXL, TweakXL,
Codeware, Mod Settings, Equipment-EX, Browser Extension, No Intro Videos,
Virtual Atelier, Input Loader and BetterLootMarkers. Each SHA-256 was checked
against the original distribution. GitHub-hosted packages record their release URL as an `https`
source. Nexus-only packages are `manual` sources naming the original file.
Modn does not mirror these archives. It downloads an `https` source from its
original URL when you install it. The snapshot does not assert compatibility with every game
version. See [provenance](../buckets/main/PROVENANCE.md) for versions, dependency
evidence and compatibility notes.

```powershell
modn bucket list --json
modn search --game cyberpunk-2077 --json
modn bucket show main/cyberpunk-2077/red4ext --json
modn bucket add community C:\path\to\bucket
modn bucket show community/test-game/example --json
modn bucket remove community
```

## Local bucket format

A local bucket contains `bucket.json` (schema_version: 1, name, title)
and `games/<game-id>/<package-id>.json` files. Each package record
has schema_version, id, game, name, version, archive_format (`zip`/`7z`),
sha256, source (`manual` with filename, or `https` with URL),
optional dependencies (same-game IDs), dependency_versions, conflicts,
executables and homepage. Unknown fields and
unsupported schemas are rejected. See `buckets/main` for examples.
Registrations point to explicitly selected directories. Package manifests
are revalidated on every read. Only the official `main` bucket may contain
`module.json` and `module.lua`. Modn rejects those files in registered local
buckets and rejects `modules/` directories. See [bucket contract v1](../buckets/README.md).

Bucket registration, listing, searching and manifest inspection are
**metadata-only** and never modify game files. A catalog entry can be
installed from a supplied, SHA-256-verified ZIP/7z with an explicit approval;
standalone local archives use the same managed transaction engine. A checksum
proves the bytes match a catalog record, not publisher trust.

## Requirements, conflicts and releases

A manifest may require a dependency version with `dependency_versions` and
declare `conflicts` with other mods of its bucket and game. Before any
download or write, Modn refuses a plan in these cases:
- a dependency's release in the bucket misses a requirement
- two releases in the installation conflict
- a release conflicts with an installed managed package, in either direction

A dry run lists these conflicts in `conflicts`. The Rust core checks
conflicts again under the transaction lock, and records each package's
conflicts in the same commit as its ownership. A conflict matches by catalog
identity or by the pinned archive digest, the same rule that dependency
readiness uses. So a mod installed through another manifest with the same
archive is still caught. A standalone `modn install <archive>` also meets the
recorded conflicts, by its digest. On older state without catalog tables,
installed packages still match by digest. A shared-archive alias in a stack must declare the same
dependencies and conflicts as the release that installs the archive.
Search, dependency resolution and `modn install` use each mod's current
release, `games/<game-id>/<mod-id>.json`, so Modn checks requirements rather
than choosing between versions. A local bucket may also publish other
releases as `games/<game-id>/<mod-id>@<version>.json`; only
`modn update --version` selects them. Installed packages carry no version.
An `other_release` dependency is not replaced by an install; replace it
explicitly with `modn update`.
