# Buckets

## The official main bucket

Modn reads the official bucket from `<data dir>/buckets/main/`. If it is missing,
Modn tells the user to run `modn bucket update`. Modn never fetches it implicitly.
The install script runs that command once. The local copy works offline.
`MODN_MAIN_BUCKET_SOURCE` selects an HTTPS URL or local ZIP for an update.

`modn bucket update` downloads `main-bucket.zip` and `main-bucket.zip.minisig`
from the latest [`modn-org/main`](https://github.com/modn-org/main) release over
HTTPS. It extracts the ZIP, verifies the minisign signature and validates the
bucket and modules before an atomic swap. Failure preserves the previous copy.
A module whose `api_version` this Modn does not support makes the whole update
fail. `main` is always a trusted module source, and only `main` can be updated
with this command. The bucket has its own release cycle, so a catalog change
needs no Modn release.
Modn refreshes the copy only when requested. Signed Git catalogs can also supply
game modules after the user trusts them. See [module sources and trust](#module-sources-and-trust).
Explicit Git-backed community catalog sync is available; see [sources.md](sources.md).

`modn update` with no target refreshes the official bucket and every registered
Git catalog in one run. It uses the same signed-release path as
`modn bucket update` and the same pinned-signer sync as
`modn bucket sync <name>`. Local folder buckets are skipped, because they have
nothing to fetch. One failing catalog does not stop the others; the command
reports each bucket and exits nonzero when any failed. It never writes game
files or install state. `modn update <target>` still replaces one installed
catalog mod.

### Main catalog signature

Modn verifies a minisign (Ed25519) signature with the catalog signer key built
into the release. The key ID is `0E250AD52A3BA2AA`.
Modn accepts only the prehashed signature format and refuses the legacy format.
The signature covers this signed statement:

```text
modn-catalog-signature/v1
catalog: main
digest: blake3:<hex>
```

The `catalog:` line is the `name` field of the extracted root `bucket.json`.
The `digest:` line is the [catalog digest](#catalog-digest) of the extracted
files.
On success, the command prints `Verified main bucket signed by 0E250AD52A3BA2AA.`

Modn refuses an unsigned release, an invalid signature, a signature from another
key, and a release whose `bucket.json` name is not `main`. The refusal keeps the
installed copy. The `main-bucket.zip.sha256` sidecar is no longer used.

Each `modn-org/main` release must include `main-bucket.zip.minisig` next to the
ZIP. The bucket repository's release pipeline does not produce that file yet.
Key rotation and revocation are planned (#122).

### Catalog digest

The catalog digest is BLAKE3. Its input starts with the domain prefix
`modn-catalog-blake3/v1` and one NUL byte. Then each regular file follows in
byte-wise path order: its relative UTF-8 path, one NUL byte, its little-endian
u64 size and its bytes. Root `catalog.sig` and `catalog.pub` are excluded from
the digest. The digest prints as `blake3:<hex>`.

Modn refuses a symbolic link, another non-regular entry, two paths that differ
only by case, a root `.git` entry and a non-UTF-8 path. It refuses a root
`bucket.json` that is not a regular file, exceeds 32 KiB, or is malformed.

### Signing a catalog

Install the stock `minisign` CLI, then run these commands. Replace `<dir>` with
the catalog path. The statement file must hold the bytes exactly, with no
byte-order mark and no added newline.

```powershell
cmd /c "cargo run -p modn-core --example catalog_digest -- <dir> > statement.txt"
minisign -S -s <secret key> -m statement.txt -x <dir>/catalog.sig
Copy-Item <public key> <dir>/catalog.pub
```

Windows PowerShell 5.1 `>` writes UTF-16, so keep the `cmd /c` redirection.

`catalog_digest` prints the statement for the catalog tree. `minisign -S` signs
that file and writes `catalog.sig` at the catalog root. Do not pass `-l`, which
writes the legacy format that Modn refuses. `catalog.pub` holds the signer
public key as a bare key line or a two-line `minisign.pub` file.

Sign the tree that you ship. Any change to a catalog file changes the digest and
invalidates the signature. A Git catalog publishes `catalog.sig` and `catalog.pub`
at its root. A `main` release also uploads the same signature as
`main-bucket.zip.minisig`.

## Module sources and trust

`main` is always a trusted module source. Only a signed Git catalog can become
another trusted module source. Local folder buckets cannot be module sources.

```sh
modn bucket trust-modules community
modn bucket trust-modules community --fingerprint 0E250AD52A3BA2AA --yes
modn bucket untrust-modules community
```

`bucket trust-modules` needs a registered signed Git catalog. It prints the
pinned signer key ID and refuses without `--yes`. The refusal names the key and
the `--yes` flag. An optional `--fingerprint <KEY_ID>` must equal the pinned
signer key ID; a mismatch writes nothing.

The trust record stores the signer key ID for the whole bucket. It does not
record one game. A new game module in the same trusted bucket needs no new
acknowledgement. If the bucket's pinned signer key changes, the stored key ID no
longer matches, and its modules become untrusted. Key rotation is planned (#122).

`bucket untrust-modules` removes the trust record and leaves the bucket
registered. Modules retained from that bucket are refused at use. Run
`trust-modules` again to restore trust.

`bucket sync` imports the modules of a trusted bucket. It reports each release
as `imported`, `unchanged`, `not_newer` or `pinned`. For an untrusted bucket it
reports each module as ignored with the reason `untrusted_source`. It neither
parses nor executes those modules. A catalog module for a game that `main`
provides is ignored with the reason `official_priority`.

A local folder bucket cannot contain `module.json` or `module.lua`. Modn refuses
such a bucket when it validates the registration. A Git catalog may contain
modules only at `games/<game-id>/module.json` and `module.lua`. See
[game modules](game-modules.md#trusted-module-sources).

## Catalog contents

The official `main` bucket catalogs thirty-one Cyberpunk
packages: RED4ext, redscript, Cyber Engine Tweaks, ArchiveXL, TweakXL,
Codeware, Mod Settings, Equipment-EX, Browser Extension, No Intro Videos,
Virtual Atelier, Input Loader, BetterLootMarkers, All Vanilla Clothes Atelier
Store, Autoloot, Clean Voiceovers, Custom Quickslots, Cyberarms Patch,
Cyberpunk 2077 HD Reworked Project - Balanced, Disclosed Hidden Gems, Drone
Companions, Hide Read Shards, Limited HUD, Native Settings UI, Native Settings
UI Side Menu Add-on, Ping, Preem Scanner, Preem Scopes, Virtual Atelier
Delivery, Virtual Car Dealer and Weather Probability Rebalance. Each SHA-256 was checked
against the original distribution. GitHub-hosted packages record their release URL as an `https`
source. Manifests can also pin a Nexus Mods file with the `nexus` source kind.
A manifest with a `nexus` source names the game domain, mod ID, file ID and file name.
The current `main` snapshot still lists its Nexus packages as `manual` sources naming the original file.
Modn does not mirror these archives. It downloads a source from its publisher when you install it. The snapshot does not assert compatibility with every game
version. See [provenance](https://github.com/modn-org/main/blob/main/PROVENANCE.md) for versions, dependency
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
sha256, source (`manual` with filename, `https` with URL, `github-release`,
`git` with repository and commit, or `nexus` with game_domain, mod_id, file_id
and file_name), optional dependencies (same-game IDs), dependency_versions,
conflicts, executables, native_injector and homepage. A `nexus` source
requires `archive_format` and accepts only `.zip` or `.7z` archives; Modn
refuses `.rar`. Unknown fields and unsupported schemas are rejected. See
[`modn-org/main`](https://github.com/modn-org/main) for examples.
Registrations point to explicitly selected directories. Package manifests
are revalidated on every read. A local folder bucket needs no signature, because
it holds your own files. `catalog.sig` and `catalog.pub` serve distributed
catalogs. A local folder bucket cannot contain `module.json` or `module.lua`,
and `modules/` directories are unsupported. A signed Git catalog may contain
modules only at `games/<game-id>/module.json` and `module.lua`. See
[bucket contract v1](../buckets/README.md).

`native_injector` is an optional boolean and defaults to `false`. A manifest
sets it to `true` when the release places a native library that the game process
loads. Any other JSON type is malformed and is rejected. `modn install` and
`modn update` refuse a plan that contains such a release, including a
dependency, unless the caller passes `--allow-native-injector`. The refusal
happens before any download or game write. See
[Native injector approval](cli.md#native-injector-approval).

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
