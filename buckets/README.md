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
`dependencies`, `homepage` and `executables`. `executables` lists up to 16
exact game-relative `.exe` paths inside a `tools` directory that the archive
installs. It is a declaration shown at the install prompt, never an approval.

Two more optional fields describe compatibility:

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

IDs use lowercase ASCII letters, digits and
hyphens, beginning with a letter and not ending with a hyphen.
Package identity is `bucket/game-id/package-id`. `<package-id>.json` is the
current release: search, dependency resolution and `modn install` use it. A
bucket may also publish other releases of the same package as
`games/<game-id>/<package-id>@<version>.json`. The `version` field must equal
the file name's version, the current release must exist, and each version may
appear once. Only `modn update <package> --version <version>` selects another
release. Modn versions before this rule reject such files. The `main`
snapshot publishes current releases only. See `buckets/main/games/` for
examples.

Only the official `main` bucket may contain `module.json` and `module.lua`.
Modn rejects registered local buckets that contain them. It also rejects a
`modules/` directory in any bucket. Registered local buckets are read-only and
revalidated before catalog lookup. Third-party game modules remain deferred (#51).

When `<data dir>/buckets/main/` is missing, the first command needing the
catalog or a game module bootstraps it. Modn downloads `main-bucket.zip` from
`https://github.com/sooskca/modn/releases/download/v<version>/main-bucket.zip`
for the running Modn version. It checks the bucket tree's SHA-256 content digest
against the digest built into the executable. It validates the bucket, stages
it, and installs it by rename. Failure leaves no partial copy. First use needs
network access; later use works offline. For development and tests,
`MODN_MAIN_BUCKET_SOURCE` selects an HTTPS URL or local ZIP path instead.
The digest check still applies.

`modn bucket update` downloads the latest release's `main-bucket.zip` and
`main-bucket.zip.sha256` sidecar over HTTPS. It verifies the digest and validates
the bucket and its modules before an atomic swap. Failure preserves the previous
copy. Modn never refreshes it without an explicit request.

Use `modn search` for package discovery and `modn bucket show bucket/game/package` for an exact manifest.
`modn bucket add <name> <directory>` registers a local catalog; removing
registration does not remove its source files.

Bucket operations do not install mods or modify game files. `modn install` resolves dependencies, checks these
requirements and conflicts, and downloads `https` artifacts. Git-backed
catalog synchronization remains deferred (#53).

## Adding a package to `main`

Add `buckets/main/games/<game-id>/<package-id>.json` and a row in
`buckets/main/PROVENANCE.md`. The release bucket contains these manifests.
