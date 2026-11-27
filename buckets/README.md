# Bucket contract v2

A bucket catalogs mod packages. The official `main` bucket also distributes
first-party Lua game modules. Modn embeds no bucket manifests or game modules.

```text
bucket.json
mods/<game-id>/<package-id>.json           # current release
mods/<game-id>/<package-id>@<version>.json # other release
games/<game-id>/module.json                # main and trusted catalogs only
games/<game-id>/module.lua                 # main and trusted catalogs only
catalog.sig                                # distributed catalogs
catalog.pub                                # distributed catalogs
```

`bucket.json` declares `schema_version` (integer 2), `name` and `title`.
Modn refuses a version 1 bucket and asks the publisher to republish it for
Modn contract v2. There is no compatibility layer.
A distributed catalog is signed. `catalog.sig` holds the minisign signature of
the catalog statement, and `catalog.pub` holds the signer public key. A local
folder bucket needs no signature. See
[Signing a catalog](../docs/buckets.md#signing-a-catalog).
Each package manifest declares `schema_version` (integer 2), `id`, `game`,
`name`, `version` and `sources`, and optional `redistributable`,
`upstream_sha256`, `dependencies`, `homepage`, `executables` and
`native_injector`.
`sources` is an ordered, non-empty list of artifact sources. Modn tries each
fetchable source in order and uses the first one whose bytes match the BLAKE3
pin. It moves on after a timeout, a network or HTTP failure, or a digest
mismatch. See [artifact source fallback](../docs/sources.md#artifact-source-fallback).
A record lists either `git` sources or archive sources, never both. A mixed
record is refused, because one archive pin cannot cover a packed Git archive.
An archive record declares `blake3`, the archive pin of 64 lowercase hex
characters. A `manual`, `https`, `github-release` or `nexus` archive record also declares
`archive_format` (`zip` or `7z`).
A `git` record declares no `blake3`, because its pinned commit is the integrity
pin. Its `archive_format` is optional and must be `zip` when present.
`redistributable` is a boolean that defaults to `false` and
states that the author allows anyone to redistribute the archive.
`upstream_sha256`, when known, is provenance only; Modn validates its shape,
stores it and never verifies an archive against it.
A record with `sha256` or a singular `source` is refused as an unknown field.
`executables` lists up to 16 exact game-relative `.exe` paths inside a `tools`
directory that the archive installs. It is a declaration shown at the install
prompt, never an approval.

Additional optional fields describe compatibility and native code:

- `dependency_versions` maps a listed dependency ID to a version
  requirement, for example `{"red4ext": ">=1.31.0"}`. A requirement is up to
  four comma-separated comparators (`>=`, `>`, `<=`, `<`, `=`), all of which
  must hold. Versions compare as dotted numbers with an optional leading `v`;
  missing components count as zero. The bucket's release of that dependency
  must satisfy the requirement, or Modn refuses to plan the install. A version
  that is not dotted numeric cannot be compared and also fails the plan.
- `conflicts` lists up to 32 mod IDs of the same bucket and game that must
  not be installed together with this one. A conflict cannot name the
  package itself or one of its dependencies. Modn records each installed
  package's conflicts, so the check holds in both directions. As with
  dependencies, a managed package with the archive digest that the bucket
  pins for a mod counts as that mod.
- `native_injector` is a boolean that defaults to `false`. Set it to `true`
  when the archive places a native library that the game process loads. A
  non-boolean value is malformed. `modn install` and `modn update` refuse such
  a release, including as a dependency, unless the caller passes
  `--allow-native-injector`.

IDs use lowercase ASCII letters, digits and
hyphens, beginning with a letter and not ending with a hyphen.
Package identity is `bucket/game-id/package-id`. `mods/<game-id>/<package-id>.json` is the
current release: search, dependency resolution and `modn install` use it. A
bucket may also publish other releases of the same package as
`mods/<game-id>/<package-id>@<version>.json`. The `version` field must equal
the file name's version, the current release must exist, and each version may
appear once. Only `modn update <package> --version <version>` selects another
release. Modn versions before this rule reject such files. The `main`
snapshot publishes current releases only. See
[`modn-org/main`](https://github.com/modn-org/main) for examples.

Each `module.json` declares `api_version`, the host API version that its module
needs. Bucket contract v2 uses host API version 3. Modn refuses a bucket with a
module that declares an earlier version and tells the publisher to republish the
module for contract v2.

`main` and the signed Git catalogs that the user trusts for modules may contain
`games/<game-id>/module.json` and `module.lua`. Those folders may hold nothing
else. `modn bucket trust-modules` validates this layout before it records trust,
so a catalog with an extra entry is refused and stays untrusted. Modn refuses a package manifest or any other entry under `games/` in
those buckets. In every other bucket, sync reads the game folder names under
`games/` and checks by file metadata whether a folder holds `module.json` or
`module.lua`. It never parses, validates or runs module content. It reports only
folders with a valid game ID that hold one of those files, each as ignored with the
reason `untrusted_source`. An empty `games/<game-id>/` folder produces no entry.
Modn refuses the module when it is used. A mod whose ID is `module` is a
normal package at `mods/<game-id>/module.json`. Modn rejects a `modules/`
directory in any bucket. Registered local buckets are read-only and
revalidated before catalog lookup.

The install script runs `modn bucket update` once. Modn never fetches the bucket
implicitly. If `<data dir>/buckets/main/` is missing, Modn tells the user to run
`modn bucket update`. For development and tests, `MODN_MAIN_BUCKET_SOURCE`
selects an HTTPS URL or local ZIP path for that command.

`modn bucket update` downloads `main-bucket.zip` and
`main-bucket.zip.minisig` from the latest
[`modn-org/main`](https://github.com/modn-org/main/releases) release over HTTPS.
It verifies the minisign signature over the catalog digest and validates the
bucket and its modules before an atomic swap. Failure preserves the previous copy.
Modn refuses an unsigned, invalid or wrong-key release. The `main-bucket.zip.sha256`
sidecar is no longer used. Each release also publishes a signed root
`sequence.json` whose number is higher than the previous release. After a user
accepts a numbered release, an older or unnumbered release is refused as a
rollback. See
[Signed sequence numbers](../docs/buckets.md#signed-sequence-numbers).

Use `modn search` for package discovery and `modn bucket show bucket/game/package` for an exact manifest.
`modn bucket add <name> <directory>` registers a local catalog; removing
registration does not remove its source files.

Bucket operations do not install mods or modify game files. `modn install` resolves dependencies, checks these
requirements and conflicts, and downloads `https` artifacts. Git-backed
catalogs are registered with `modn bucket sync <name> --source <remote>
--revision <ref>`; see [sources.md](../docs/sources.md).

## Themes

A bucket may also ship themes at this path:

```text
themes/<game-id>/<theme-id>/theme.json
themes/<game-id>/<theme-id>/theme.css
themes/<game-id>/<theme-id>/theme.<variant>.css      (one per listed variant)
themes/<game-id>/<theme-id>/assets/...               (PNG, WebP, WOFF2 only)
```

The `main` bucket may hold an official theme for a game. Any bucket kind may
ship themes, and themes need no trust step: Rust validates them, and they cannot
run code, load remote resources or change a destructive confirmation. A broken
theme is a diagnostic; it never fails `bucket update` or `bucket sync`. See
[Game themes](../docs/themes.md) for the contract.

## Adding a package to `main`

Open a pull request in [`modn-org/main`](https://github.com/modn-org/main). Its
README describes the manifest, the provenance row and the release steps. A
bucket release reaches users through `modn bucket update` without a Modn
release.

This repository pins a `main` commit as the `buckets/main` submodule. The
catalog, milestone gate, verify-modn and ownership tests use that copy. To test
a newer bucket, update the submodule:

```sh
git -C buckets/main fetch origin
git -C buckets/main checkout <commit>
git add buckets/main
```
