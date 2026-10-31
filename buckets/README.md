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
`dependencies` and `homepage`. IDs use lowercase ASCII letters, digits and
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
modify game files. Artifact download, dependency resolution and Git-backed
catalog synchronization remain future work.
