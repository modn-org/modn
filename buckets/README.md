# Bucket contract v1

A bucket catalogs mod packages. It does not distribute executable game modules.
First-party Lua game modules live in `crates/modn-core/game-modules/` and ship
with Modn, independently of buckets.

```text
bucket.json
games/<game-id>/<package-id>.json
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
Package identity is `bucket/game-id/package-id`; one version per ID is
published in each bucket snapshot. See `buckets/main/games/` for examples.

Buckets containing a `modules/` directory are rejected; it is not an
extension point. Registered local buckets are read-only and revalidated before
catalog lookup. The official `main` snapshot is compiled into Modn and
works offline.

Use `modn search` for package discovery and `modn bucket show bucket/game/package` for an exact manifest.
`modn bucket add <name> <directory>` registers a local catalog; removing
registration does not remove its source files.

Bucket operations never download artifacts, execute Lua, install mods or
modify game files. `modn install` resolves dependencies, checks these
requirements and conflicts, and downloads `https` artifacts. Git-backed
catalog synchronization remains future work.
