# Bucket contract v1

A bucket is a regular directory with bucket.json and two independent kinds
of catalog record. The same schema works for the offline official snapshot
and explicitly registered local buckets.

~~~text
bucket.json
games/<game-id>/<package-id>.json
modules/<game-id>/<version>/module.json
modules/<game-id>/<version>/<entrypoint>.lua
~~~

The games/ and modules/ directories are independently optional. Package
identity is bucket/game-id/package-id. Module release identity is
bucket/game-id/version. Bucket names, game IDs and package IDs use lowercase
ASCII letters, digits and hyphens, beginning with a letter and not ending in
a hyphen. Module versions are opaque case-sensitive labels (1-48 ASCII
letters, digits, dots, hyphens or plus), with no trailing dot. Case-only
version variants within one game are invalid on every platform.

bucket.json contains schema_version (integer 1), name (the registered bucket
name) and title. Existing package manifests remain schema v1, in
games/<game-id>/<package-id>.json: schema_version, id, game, name, version,
archive_format, sha256, source, and optional dependencies and homepage.
One package ID has one release in this contract; multi-version package
resolution is deferred.

A Lua module release has exactly these fields in module.json:

~~~json
{
  "schema_version": 1,
  "game_id": "cyberpunk-2077",
  "version": "0.1.0",
  "api_version": 1,
  "entrypoint": "main.lua",
  "content_sha256": "<64 lowercase hex SHA-256 characters>"
}
~~~

The SHA-256 covers the exact Lua entrypoint bytes, not a ZIP, directory,
manifest or publisher signature. Each module-release directory contains
only module.json and the referenced flat UTF-8 .lua file. The reader
rejects unknown fields, schema/identity/path mismatches, unrecognized files,
linked paths, invalid digests, binary source, and unsupported host API
versions without evaluating Lua. Module manifest size is limited to 16 KiB,
script size to 256 KiB, and releases to 1,024 per bucket.

Use modn bucket search and modn bucket show bucket/game/package for packages.
Use modn bucket modules and modn bucket module-show bucket/game/version for
modules. Both share local bucket registration, exact namespace resolution,
read-only revalidation and optional --game filtering.

No catalog operation downloads archives, executes Lua, changes installed game
files, activates modules, or modifies the game-module selection. The official
main snapshot works without network access. Runtime module selection still
uses the existing separate module store until the bucket-managed lifecycle
is introduced. Git sync and community trust decisions are future work.
