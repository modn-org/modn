# Bucket contract v1

A bucket catalogs mod packages. The official `main` bucket also distributes
first-party Lua game modules. Modn embeds no bucket manifests or game modules.

```text
bucket.json
games/<game-id>/<package-id>.json
games/<game-id>/module.json       # official main only
games/<game-id>/module.lua        # official main only
```

`bucket.json` declares `schema_version` (integer 1), `name` and `title`.
Each package manifest declares `schema_version`, `id`, `game`, `name`,
`version`, `archive_format`, `sha256`, `source`, and optional
`dependencies`, `homepage`, `executables` and `native_injector`.
`executables` lists up to 16
exact game-relative `.exe` paths inside a `tools` directory that the archive
installs. It is a declaration shown at the install prompt, never an approval.

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
Package identity is `bucket/game-id/package-id`. `<package-id>.json` is the
current release: search, dependency resolution and `modn install` use it. A
bucket may also publish other releases of the same package as
`games/<game-id>/<package-id>@<version>.json`. The `version` field must equal
the file name's version, the current release must exist, and each version may
appear once. Only `modn update <package> --version <version>` selects another
release. Modn versions before this rule reject such files. The `main`
snapshot publishes current releases only. See
[`modn-org/main`](https://github.com/modn-org/main) for examples.

Each `module.json` declares `api_version`, the host API version that its module
needs. Modn refuses a bucket with a module whose `api_version` it does not
support.

Only the official `main` bucket may contain `module.json` and `module.lua`.
Modn rejects registered local buckets that contain them. It also rejects a
`modules/` directory in any bucket. Registered local buckets are read-only and
revalidated before catalog lookup. Third-party game modules remain deferred (#51).

The install script runs `modn bucket update` once. Modn never fetches the bucket
implicitly. If `<data dir>/buckets/main/` is missing, Modn tells the user to run
`modn bucket update`. For development and tests, `MODN_MAIN_BUCKET_SOURCE`
selects an HTTPS URL or local ZIP path for that command.

`modn bucket update` downloads `main-bucket.zip` and its
`main-bucket.zip.sha256` sidecar from the latest
[`modn-org/main`](https://github.com/modn-org/main/releases) release over HTTPS.
It verifies the digest and validates the bucket and its modules before an atomic
swap. Failure preserves the previous copy. The checksum shares the release
origin, as in Scoop. Release signing is future work.

Use `modn search` for package discovery and `modn bucket show bucket/game/package` for an exact manifest.
`modn bucket add <name> <directory>` registers a local catalog; removing
registration does not remove its source files.

Bucket operations do not install mods or modify game files. `modn install` resolves dependencies, checks these
requirements and conflicts, and downloads `https` artifacts. Git-backed
catalog synchronization remains deferred (#53).

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
