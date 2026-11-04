# Community catalogs and artifact providers

These core APIs form the first MVP. CLI registration remains an integration task.
They do not change the official `main` release source.

## Git catalogs

Use `modn_core::buckets::{BucketStore, GitCatalogSource, GitCatalogRevision}`.
Call `BucketStore::sync_git(name, Some(source))` to register a community catalog.
The source contains `remote` and `reference` strings.
Construct it with `GitCatalogSource::new(remote, reference)`.
Use an HTTPS remote or an absolute local Git repository path.
Use a branch, tag, or commit reference. Remote branch names can use `origin/<branch>`.
Call `sync_git(name, None)` only after an explicit update request.
Call `git_revision(name)` to read the selected source and full commit without fetching.

Proposed CLI wiring:

```text
modn bucket add community --git https://example.org/catalog.git --ref main
modn bucket update community
modn bucket list --json
```

Keep existing `bucket add <name> <path>` mapped to `BucketStore::add`.
Keep unnamed `bucket update` mapped to the official main release updater.
Show the Git source and commit in the named catalog's output.
Do not call `sync_git` from search, resolution, installation, or listing.

Sync clones into a private staging directory without shell interpolation.
It checks out the selected revision and removes Git administrative files.
It rejects links, non-regular entries, game modules, and invalid catalog metadata.
Publication atomically replaces the catalog registration file.
The registration contains both the validated tree path and its source revision.
Fetch, checkout, and validation failures preserve the previous usable registration.
Readers that retained an older tree can continue using it.
Old trees remain on disk. This MVP performs no catalog garbage collection.
Git must be installed. This MVP has no credentials, signatures, or network timeout policy.
Git sync serializes other Git sync calls. It does not coordinate concurrent local registration edits.
Catalog metadata does not approve executable installation or grant game writes.

## Verified archive reuse

Use `modn_core::acquisition::{ArchiveCache, CacheKey, CacheUse}` as before.
`ArchiveCache::cached` and `acquire` also search existing cache entries by SHA-256.
Two package or source aliases can return the same verified payload path.
Every hit verifies the actual file bytes against the requested pinned digest.
A corrupt entry is never a hit. A subsequent download can supply verified bytes.
Refresh still fetches explicitly and verifies the pinned digest.

The existing package and URL filename layout remains compatible.
This MVP scans entries instead of maintaining a digest index.
It creates no alias file when it reuses another package's payload.
Cache listing therefore names the original stored package.
Removing that entry makes aliases download again when needed.
Existing duplicate or corrupt files remain until explicit cache removal.
This MVP deduplicates cache reuse only. It never hard-links cache bytes or mutable game files.

## GitHub release provider

Use `modn_core::acquisition::GithubReleaseAsset`.
Its fields are `repository`, `tag`, and `asset`.
Construct it with `GithubReleaseAsset::new(repository, tag, asset)`.
For example, select `owner/project`, `v1.2`, and `mod.zip`.
Call `metadata_url()` to validate the selector and obtain the GitHub API endpoint.
Call `resolve(fetch)` with an HTTP transport returning a readable metadata response body.
The transport must check HTTP status and reject unsafe redirects.
Send GitHub's normal unauthenticated API headers. No secret is required.

Resolution requires the exact repository, release tag, and asset name.
It rejects duplicate asset names and noncanonical download URLs.
It reads at most one MiB of JSON metadata.
The result is a canonical HTTPS GitHub release asset URL.
Build `CacheKey` with that URL and the package's existing SHA-256 pin.
Call `ArchiveCache::acquire` to verify the downloaded bytes before use.
Provider metadata never replaces the artifact pin or executable approval.

The integration layer must collect the provider selector separately from existing package sources.
It can map an explicit provider selection to the existing HTTPS acquisition path after resolution.
This pass does not add a `PackageSource` variant or alter manual and HTTPS manifest behavior.
Future manifest wiring must update every source match and preserve existing schemas.
Tags and asset names accept ASCII letters, digits, dots, underscores, and hyphens.
`latest`, authentication, pagination, arbitrary providers, and release discovery remain unsupported.
Transport rate limits, retries, redirect policy, and timeouts remain caller responsibilities.
