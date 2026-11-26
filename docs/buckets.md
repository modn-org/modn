# Buckets

## The official main bucket

Modn reads the official bucket from `<data dir>/buckets/main/`. If it is missing,
Modn tells the user to run `modn bucket update`. Modn never fetches it implicitly.
The install script runs that command once. The local copy works offline.
`MODN_MAIN_BUCKET_SOURCE` selects an HTTPS URL or local ZIP for an update and
bypasses release selection.

Without that override, `modn bucket update` lists non-draft, non-prerelease
[`modn-org/main`](https://github.com/modn-org/main) releases newest first. It reads
each release's `main-bucket.requires.json` asset when present. A release is
supported when its `schema_version` equals the bucket contract this build reads
and its `min_modn_version` is less than or equal to this build's version under
SemVer precedence. Modn installs the newest supported release's listed
`main-bucket.zip` and `main-bucket.zip.minisig` assets over HTTPS. It extracts the
ZIP, verifies the minisign signature and validates the bucket and modules before
an atomic swap. Failure preserves the previous copy. A malformed requirements
file skips that release with a diagnostic.

A release without `main-bucket.requires.json` is treated as readable for legacy
releases such as `v2026.11.24`. The install checks still refuse it if its contract
or modules are unsupported. A module whose `api_version` this Modn does not
support makes the whole update fail. `main` is always a trusted module source,
and only `main` can be updated with this command. The bucket has its own release
cycle, so a catalog change needs no Modn release.
Modn refreshes the copy only when requested. Signed Git and ZIP catalogs can also supply
game modules after the user trusts them. See [module sources and trust](#module-sources-and-trust).
Explicit Git-backed and ZIP-backed community catalog sync is available; see [sources.md](sources.md).

When Modn skips a newer release as unsupported, text mode writes one stderr line
such as:

```text
main release v2026.12.01 needs Modn 0.8.0; keeping v2026.11.24. Update Modn to get it.
```

JSON output adds `release` and `newer_release_needs_modn`. The latter is null
when no newer release was skipped. If no release is supported, the command
names the required Modn version, fails and leaves the installed bucket
unchanged.

`modn update` with no target refreshes the official bucket and every registered
Git or ZIP catalog in one run. It uses the same signed-release path as
`modn bucket update` and the same pinned-signer sync as
`modn bucket sync <name>`. Local folder buckets are skipped, because they have
nothing to fetch. One failing catalog does not stop the others; the command
reports each bucket and exits nonzero when any failed. It never writes game
files or install state. `modn update <target>` still replaces one installed
catalog mod.
### Community catalog sources
`modn bucket sync <name> --source <remote> --revision <reference>` registers a
Git catalog. `modn bucket sync <name> --source <url>.zip` registers a catalog
from a signed ZIP. The ZIP root holds the same files as a Git catalog:
`bucket.json`, `mods/`, optional `games/`, `catalog.sig` and `catalog.pub`.
The source must be an `https://` URL, or an absolute local ZIP path for offline
testing. Modn refuses a plain `http://` source before any network call.
Modn streams the body to a temporary file in staging and caps it at 256 MiB. It
inspects the archive and refuses traversal, absolute, backslash, symbolic-link,
duplicate and case-colliding entries. It also refuses more than 20,000 entries,
an entry above 64 MiB, more than 1 GiB uncompressed in total, and an expansion
ratio above 200. It extracts only inside staging, verifies the signature and
then validates the contract v2 layout with the same validator as Git sync.
The first sync pins the signer from `catalog.pub` and prints its fingerprint. It
validates any published rotation or revocation statement and ignores it as
history. Every later sync verifies with the pinned key only and refuses a
different signer. `--revision` is refused with a ZIP source.
A failed download, an invalid signature or invalid contents keep the previous
registration and catalog files byte for byte and remove staging.
`modn bucket list --json` reports `source_kind` as `git` or `zip` for a
registered catalog. It keeps the existing keys. It also reports `primary`, the
registered source as a tagged value, and `mirrors`, the ordered mirror list.

### Mirrors and relocation

A signed catalog may hold two optional root files. The pinned catalog signature
covers both, because the catalog digest covers every root file except
`catalog.sig` and `catalog.pub`.

`mirrors.json` lists other hosts of the same catalog:

A ZIP catalog lists ZIP mirrors:

```json
{
  "schema_version": 1,
  "mirrors": [
    {"kind": "zip", "url": "https://mirror.example/catalog.zip"},
    {"kind": "zip", "url": "https://second.example/catalog.zip"}
  ]
}
```

A Git catalog lists Git mirrors:

```json
{
  "schema_version": 1,
  "mirrors": [
    {"kind": "git", "remote": "https://git.example/catalog.git", "revision": "main"},
    {"kind": "git", "remote": "https://second.example/catalog.git", "revision": "main"}
  ]
}
```

Modn refuses a mirror list with more than 8 entries, a duplicate, a plain
`http://` URL, a local path, a URL with credentials, or a source kind that
differs from the registered primary. A Git remote or revision follows the same
rules as a Git catalog source.

`relocation.json` names the new primary host:

```json
{
  "schema_version": 1,
  "primary": {"kind": "zip", "url": "https://new.example/catalog.zip"}
}
```

Sync tries the registered primary first and then the stored mirrors in order.
A transport failure moves to the next source. A connection error, a network
error, an HTTP error, a truncated or malformed ZIP and a Git failure are
transport failures. A missing signature, a signature from another key, tampered
content and a digest mismatch are trust failures: sync stops, tries no further
source, and keeps the previous registration and catalog byte for byte. If every
source fails, one error lists each source and its reason.
When a mirror serves the revision, the registration keeps the registered
primary and the output names the failed primary and the serving mirror.
Every verified revision replaces the stored mirror list with its own list.

The Git fetch of a mirror uses the same low-speed limits as a package source:
`http.lowSpeedLimit=1` and `http.lowSpeedTime=90` for HTTP and HTTPS. A stalled
mirror fails as `timed out`, and sync tries the next mirror.

A verified revision whose catalog digest (ZIP) or commit (Git) matches the
registered revision, and whose `catalog.pub` and `catalog.sig` bytes match too,
reuses the registered snapshot and discards the staging copy. A changed digest
or a changed signer publishes a new snapshot. Sync records the superseded tree
as retired, with the bucket and the retire time, and does not delete it in that
run: `bucket list`, `bucket show` and `install` read a registration and then
open the tree it names without the sync lock, so a tree outlives the
registration switch that replaced it. A later sync that publishes a revision in
that store removes retired trees after its own registration write, while it
holds the sync lock, but only when the recorded retire time is at least one hour
old. That grace period lets a reader that loaded the previous registration
finish with its tree even when two syncs run in quick succession. A readable
record whose entry has no valid `retired_at` value keeps the tree and gets a
fresh retire time. When `.retired-catalogs` is missing or cannot be parsed,
Modn records no retired trees from it. The same holds for an entry that Modn
cannot read. Those earlier trees stay on disk without a retire record, and the
next sync records only the tree that it supersedes. A sync that fails removes
nothing, and a tree that a registration names is never removed. `bucket remove` deletes the store-created live tree
and the retired trees of that bucket after the registration file is gone; a
local folder source and a tree that another registration names stay.
A Git sync verifies the catalog, stages its game modules, commits those modules
to the module registry and only then publishes the registration. The modules
were checked against the signed catalog snapshot, so committing them first adds
no trust. A module commit failure therefore leaves the previous registration
and the previous active releases in place. A registration write that fails
after the module commit keeps the previous registration usable and leaves the
new releases active; the next sync publishes the revision, and a release that
is already active is reported `unchanged`. A sync whose fetched revision
matches the registration still stages and checks its modules; it never skips
the module step.

A ZIP sync publishes the registration before it commits staged modules. When
that commit fails, it restores the previous registration and removes the new
snapshot. If the restore itself fails, sync keeps the new tree, because the
registration may still point at it, and reports both errors.

After a revision verifies with the pinned key, a `relocation.json` whose primary
differs from the registered primary publishes the catalog and updates the
registration's primary in one atomic registration write. The output says
`catalog <name> moved from <old> to <new>`. Sync follows at most one relocation,
and a relocation to the same primary is ignored. The next sync contacts the new
primary first.

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
ZIP. Each new release must also include `main-bucket.requires.json`; see
[bucket release requirements](releasing.md#bucket-release-requirements).
The official `main` catalog has no stored registration, so the rotation and
revocation statements on this page apply to registered community catalogs.

### Catalog key rotation and revocation

A catalog can replace its signing key or revoke it. Three optional root files
carry the statements. All three are inside the [catalog digest](#catalog-digest),
and `catalog.sig` covers them:

- `revocation.pub` holds the public key of an offline revocation key.
- `rotation.json` and `rotation.sig` rotate the pinned signing key.
- `revocation.json` and `revocation.sig` revoke the pinned signing key.

`rotation.json` has this exact shape:

```json
{"schema_version":1,"kind":"rotation","from":"<pinned key id>","to":"<new public key>"}
```

`revocation.json` has this exact shape:

```json
{"schema_version":1,"kind":"revocation","revoked":"<pinned key id>","replacement":"<new public key>"}
```

The `.sig` file is a minisign signature. It signs the bytes
`modn-rotation-v1\0catalog: <name>\n` followed by the exact `rotation.json`
bytes. A revocation signature signs `modn-revocation-v1\0catalog: <name>\n`
followed by the exact `revocation.json` bytes. The catalog name binds the
statement to one catalog, and the prefix binds it to one kind. A statement
cannot be replayed in another catalog or as another kind. Modn applies each
statement once. A revision that keeps the pinned `catalog.pub` and republishes
an applied statement treats it as history and syncs normally. The replay check
runs only when no new statement applies to the pinned key. For example, after
`A->B->A`, a revision that keeps the applied `A->B` rotation and adds a valid
new `A->C` revocation is accepted. The new statement applies. A revision whose
`catalog.pub` names another key and that keeps the applied statement, adding
nothing that applies, is refused, so a retired key cannot regain trust by
replaying that statement.

Modn pins the revocation key at the first sync and prints both fingerprints. A
revision whose `revocation.pub` differs by any public key byte or disappears is
refused. A catalog that had no `revocation.pub` at its first sync cannot add one
later. To pin a new revocation key, remove the bucket and sync it again.

At the first sync Modn trusts `catalog.pub` on first use. It checks every
published `rotation.json` and `revocation.json` for shape, kind, schema version
and the key ID in the signature file, then ignores the statement as history. It
verifies no signature at this point. A statement adds no
trust for a user who never pinned the old key. The output names the ignored
statements. A malformed statement or an unknown kind or version refuses the
revision.

Every later sync evaluates each published statement against the pinned signing
key. A statement applies only when its `from` or `revoked` field names the
pinned key (matched by key ID) and the matching pinned key signed the
domain-separated statement. Modn then verifies the signature with the full
pinned key, so a statement with the same key ID but another key is refused, not
ignored. Every other statement is history and adds no trust. An author may keep
it published.

Sync accepts a rotation when all of these hold:

- `rotation.sig` verifies with the pinned signing key.
- `from` is the pinned key ID.
- `catalog.pub` publishes exactly the key in `to` with the same full public key
  bytes, and `catalog.sig` verifies with it.

Sync then pins the public key text from the signed statement, not the revision's
`catalog.pub`, and records the old key ID in `previous_signers`. Modn also drops
the new key ID from `previous_signers` and `revoked_signers`, so a rotation may
name a key that an earlier rotation retired. The old key vouched for the new key,
so module trust follows the new signer. The output prints
`signing key rotated: <old> -> <new>`.

A rotation that names a key in `revoked_signers` is refused. Only the offline
revocation key can re-authorize a revoked key. The pinned key is never recorded
as retired, so the current key always verifies its own revisions.

Sync accepts a revocation when all of these hold:

- `revocation.sig` verifies with the pinned revocation key.
- `revoked` is the pinned signing key ID.
- `catalog.pub` publishes exactly the key in `replacement` with the same full
  public key bytes, and `catalog.sig` verifies with it.

Sync then pins the public key text from the signed statement and records the old
key ID in `revoked_signers`. The replacement key ID is removed from both
`previous_signers` and `revoked_signers`, so a revocation re-authorizes a key
that an earlier rotation retired. Module trust is cleared, because the user must
trust modules again explicitly. The output prints
`signing key revoked: <old> replaced by <new>`.

Modn refuses a revision when both a rotation and a revocation apply to the
pinned key, when the revision needs more than one key change from the pinned key
(sync an intermediate revision or remove the bucket and register it again), a
rotation signed by the revocation key, a revocation signed by the signing key,
an unknown kind or schema version, a `catalog.pub` whose full public key bytes
differ from the pinned key or the key a statement names, and a revision signed by
a key in `previous_signers` or `revoked_signers`. Every refusal keeps the
previous registration and catalog byte for byte.

A revision that repeats a statement this pin already applied treats it as
history, because the registration records the BLAKE3 of the applied
`rotation.json` or `revocation.json`. Such a revision syncs normally while its
`catalog.pub` is the pinned key. If it names another key and no new statement
applies, Modn refuses the replay. A statement whose old key no longer matches
the pin is also history.

`bucket list --json` reports `signer_key_id`, `revocation_key_id` when pinned,
and the `previous_signers` and `revoked_signers` arrays when non-empty.

If a signing key leaks, rotate it with the pinned signing key, or revoke it with
the offline revocation key and name an uncompromised replacement. Sign the new
revision with the replacement key and sync. If the offline revocation key leaks,
remove the bucket and sync it again to pin a new revocation key; Modn cannot
change a pinned revocation key in place.

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

Create one signing key with `modn catalog keygen`. Run `modn catalog sign` before
each publish, and `modn catalog verify` to check the tree. Replace `<dir>` with
the catalog path.

```powershell
modn catalog keygen --out ../author-keys
modn catalog sign . --key ../author-keys/catalog.key
modn catalog verify . --key catalog.pub
```

`modn catalog sign` writes `catalog.sig` and the matching `catalog.pub` at the
catalog root. `modn catalog hash` prints the BLAKE3 pin of an archive for a
package record. `modn catalog rotate` and `modn catalog revoke` write the signed
key statements described above. A password-protected key takes
`--password-env <VAR>`. Every command needs no manager state and registers
nothing. See [catalog authoring](catalog-authoring.md) for the full workflow.

Sign the tree that you ship. Any change to a catalog file changes the digest and
invalidates the signature. A Git catalog publishes `catalog.sig` and `catalog.pub`
at its root. A `main` release also uploads the same signature as
`main-bucket.zip.minisig`. That file comes from the main bucket release pipeline,
which uses the stock `minisign` CLI with `-S`, never `-l`, and produces the same
statement format. See [releasing](releasing.md).

## Module sources and trust

`main` is always a trusted module source. Only a signed catalog can become
another trusted module source. Local folder buckets cannot be module sources.
An attempt to trust one is refused with an error that names the bucket and
states that only `main` and the signed catalogs the user trusts for modules
supply game modules.

```sh
modn bucket trust-modules community
modn bucket trust-modules community --fingerprint 0E250AD52A3BA2AA --yes
modn bucket untrust-modules community
```

`bucket trust-modules` needs a registered signed catalog. It prints the
pinned signer key ID and refuses without `--yes`. The refusal names the key and
the `--yes` flag. An optional `--fingerprint <KEY_ID>` must equal the pinned
signer key ID; a mismatch writes nothing.

The trust record stores the signer key ID for the whole bucket. It does not
record one game. A new game module in the same trusted bucket needs no new
acknowledgement. A rotation keeps the trust record and moves it to the new key.
A retention release imported under the old key stays usable, because the old key
vouched for the new one. A revocation clears the trust record, and its modules
become untrusted. If the offline revocation key later re-authorizes the revoked
signer as its replacement, the bucket authorizes the releases it retained from
that signer again; the user must still run `trust-modules` after the revocation.
Otherwise a module release imported under the revoked key stays untrusted after
`trust-modules` with the replacement key; publish a new release signed by the
replacement key to supply module code again.

`bucket untrust-modules` removes the trust record and leaves the bucket
registered. Modules retained from that bucket are refused at use. Run
`trust-modules` again to restore trust.

`bucket sync` imports the modules of a trusted bucket. It reports each release
as `imported`, `unchanged`, `not_newer` or `pinned`. A sync of an untrusted
catalog reports the modules it will not import. It reads game directory names
and checks for `module.json` or `module.lua`, and it parses and executes
nothing. Each matching game is reported as ignored with the reason
`untrusted_source`. Modn refuses those modules when a module is used. A
catalog module for a game that `main` provides is ignored with the reason
`official_priority`.

Only `main` and the signed catalogs that the user trusts for modules may
contain `games/<game-id>/module.json` and `module.lua`. Those folders may hold
nothing else. In those sources, a package manifest or any other entry under
`games/` is refused at sync.
Every other bucket never validates, parses or executes content under `games/`,
so content there cannot supply a usable module. A local folder bucket has no
sync, so Modn reads nothing from `games/` there for modules. A local folder
bucket cannot supply game modules, and `modules/` directories are unsupported
in every bucket. A mod whose ID is `module` is a normal package at
`mods/<game-id>/module.json`. See
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
Delivery, Virtual Car Dealer and Weather Probability Rebalance. Each upstream
SHA-256 was checked against the original distribution; a v2 record keeps it as
`upstream_sha256` provenance. GitHub-hosted packages record their release URL as an `https`
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

A local bucket contains `bucket.json` (schema_version: 2, name, title)
and `mods/<game-id>/<package-id>.json` files. Each package record
has schema_version: 2, id, game, name, version, sources (`manual` with filename,
`https` with URL, `github-release`, `git` with repository and commit, or `nexus`
with game_domain, mod_id, file_id and file_name), optional redistributable,
upstream_sha256, dependencies (same-game IDs), dependency_versions, conflicts,
executables, native_injector and homepage.
`sources` is an ordered, non-empty list. Modn tries each fetchable source in
order and uses the first one whose bytes match the `blake3` pin. It moves on
after a timeout, a network or HTTP failure, or a digest mismatch. A `manual`
source is never started automatically. See
[artifact source fallback](sources.md#artifact-source-fallback).
A record lists either archive sources or `git` sources, never both. Modn refuses
a mixed record, because one archive pin cannot cover a packed Git archive.
An archive record declares `blake3`, 64 lowercase hex characters that pin the
archive. Every archive source kind (`manual`, `https`, `github-release`,
`nexus`) also declares `archive_format` (`zip` or `7z`).
A `git` record declares no `blake3`, because its commit is the pin.
Its `archive_format` is optional and must be `zip` when present.
`redistributable` defaults to `false`. `upstream_sha256` is provenance only.
A `nexus` source
requires `archive_format` and accepts only `.zip` or `.7z` archives; Modn
refuses `.rar`. Unknown fields and unsupported schemas are rejected. Modn
refuses a version 1 record or a version 1 `bucket.json` with a message that
requires the bucket to be republished for Modn contract v2. See
[`modn-org/main`](https://github.com/modn-org/main) for examples.
Registrations point to explicitly selected directories. Package manifests
are revalidated on every read. A local folder bucket needs no signature, because
it holds your own files. `catalog.sig` and `catalog.pub` serve distributed
catalogs. A local folder bucket cannot supply game modules, and `modules/`
directories are unsupported. Only `main` and a signed catalog the user
trusts for modules may supply modules from `games/<game-id>/module.json` and
`module.lua`. See
[bucket contract v2](../buckets/README.md).

`native_injector` is an optional boolean and defaults to `false`. A manifest
sets it to `true` when the release places a native library that the game process
loads. Any other JSON type is malformed and is rejected. `modn install` and
`modn update` refuse a plan that contains such a release, including a
dependency, unless the caller passes `--allow-native-injector`. The refusal
happens before any download or game write. See
[Native injector approval](cli.md#native-injector-approval).

Bucket registration, listing, searching and manifest inspection are
**metadata-only** and never modify game files. A catalog entry can be
installed from a supplied, BLAKE3-verified ZIP/7z with an explicit approval;
standalone local archives use the same managed transaction engine. A checksum
proves the bytes match a catalog record, not publisher trust.

## Themes in buckets

Any bucket kind may ship themes. A bucket holds themes at this path, relative to
the bucket root:

```text
themes/<game-id>/<theme-id>/theme.json
themes/<game-id>/<theme-id>/theme.css
themes/<game-id>/<theme-id>/theme.<variant>.css      (one per listed variant)
themes/<game-id>/<theme-id>/assets/...               (PNG, WebP, WOFF2 only)
```

Themes need no trust step. A theme is low-trust content that Rust validates:
CSS, images and fonts only. It cannot run code, load remote resources or change
a destructive confirmation.
A broken theme is a diagnostic. It never fails `bucket update`, `bucket sync`,
`install`, `update` or `list`. See [Game themes](themes.md) for the full
contract.

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
release, `mods/<game-id>/<mod-id>.json`, so Modn checks requirements rather
than choosing between versions. A local bucket may also publish other
releases as `mods/<game-id>/<mod-id>@<version>.json`; only
`modn update --version` selects them. Installed packages carry no version.
An `other_release` dependency is not replaced by an install; replace it
explicitly with `modn update`.
