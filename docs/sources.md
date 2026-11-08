# Community catalogs and artifact providers

These core APIs and explicit CLI commands form the first MVP.
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

CLI commands:

```text
modn bucket sync community --source https://example.org/catalog.git --revision main
modn bucket sync community
modn bucket list --json
```

Keep existing `bucket add <name> <path>` mapped to `BucketStore::add`.
Keep unnamed `bucket update` mapped to the official main release updater.
Show the Git source, commit and pinned key ID in the named catalog's output.
Do not call `sync_git` from search, resolution, installation, or listing.

Sync clones into a private staging directory without shell interpolation.
It checks out the selected revision and removes Git administrative files.
It rejects links, non-regular entries, game modules, and invalid catalog metadata.
Publication atomically replaces the catalog registration file.
The registration contains the validated tree path, its source revision and the pinned signer key.
Fetch, checkout, and validation failures preserve the previous usable registration.
Readers that retained an older tree can continue using it.
Old trees remain on disk. This MVP performs no catalog garbage collection.

A Git catalog is signed. The first revision must have `catalog.pub` and
`catalog.sig` at its root. Modn verifies the revision with the key in
`catalog.pub` and pins that key. It prints
`Pinned signer key <key ID> for bucket <name>; compare this fingerprint with the catalog author.`
Later syncs verify the revision with the pinned key only. A later revision may
omit `catalog.pub`.
A `catalog.pub` with another key ID is refused, because Modn does not rotate
keys yet (#122). An unsigned or invalid revision is refused, and there is no
`--allow-unsigned` option.
A registration made before catalog signing fails with a request to remove the
bucket and sync it again.
`modn bucket list` shows the pinned key ID after the catalog title.

Git must be installed. This MVP has no credentials or network timeout policy.
Git sync serializes other Git sync calls. It does not coordinate concurrent local registration edits.
Catalog metadata does not approve executable installation or grant game writes.

## Verified archive reuse

Use `modn_core::acquisition::{ArchiveCache, CacheKey, CacheUse}` as before.
`ArchiveCache::cached` and `acquire` search existing cache entries by SHA-256.
The cache root holds a digest index in `.digest-index.json`.
The index maps an archive pin to one cache entry file name.
Two package or source aliases can return the same verified payload path.
Every hit verifies the actual file bytes against the requested pinned digest.
A stale index entry, whose file is missing or changed, is dropped.
A corrupt entry is never a hit. A subsequent download can supply verified bytes.
Refresh still fetches explicitly and verifies the pinned digest.
A missing, unreadable, or unknown-format index triggers a scan.
The scan finds the verified entry and rebuilds the index.
The index is never authority: only re-verified bytes are returned.
A successful download adds its entry to the index.
Index writes use a temporary file in the cache root and an atomic rename.
A crash therefore leaves the previous index or none.
Modn is pre-alpha, so an index in an unknown format is corrupt and is never migrated.
The pin names its hash algorithm, so issue #117 can switch to BLAKE3 by rebuilding the index.

The existing package and URL filename layout remains compatible.
It creates no alias file when it reuses another package's payload.
Cache listing therefore names the original stored package.
The index file is not a cache entry and cache listing never shows it.
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

Package manifests support this pinned source alongside existing manual and HTTPS sources:

```json
{"kind":"github-release","repository":"owner/project","tag":"v1.2","asset":"mod.zip"}
```

The package still requires its original archive SHA-256 pin.
Install resolves provider metadata only after explicit download or installation consent.
`install --dry-run --download` permits metadata and archive acquisition without game writes.
Ordinary previews derive the canonical asset URL locally and make no network requests.
Verified cache hits, including digest aliases, need no provider metadata request.
Metadata transport sends GitHub API headers and refuses redirects.
Archive transport retains HTTPS-only retrieval and the existing byte verification path.
The CLI fixture transport exists only in test builds; production has no transport override.
Deterministic command tests cover metadata identity, URL and hash failures before game writes.
Tags and asset names accept ASCII letters, digits, dots, underscores, and hyphens.
`latest`, authentication, pagination, arbitrary providers, and release discovery remain unsupported.
Transport rate limits, retries, redirect policy, and timeouts remain caller responsibilities.

## Nexus Mods provider

A package manifest can pin one Nexus Mods file:

```json
{"kind":"nexus","game_domain":"cyberpunk2077","mod_id":6945,"file_id":123456,"file_name":"Mod.zip"}
```

`game_domain` is the lowercase Nexus domain, such as `cyberpunk2077`.
`mod_id` and `file_id` are numbers from 1 to 9999999999.
`file_name` has no path separators, control characters or reserved characters.
The archive SHA-256 pin and `archive_format` remain required.
`archive_format` must match the file name: `.zip` or `.7z`.
Modn refuses a `.rar` archive.
The source rejects credential fields such as `key`, `expires`, `user_id` and `apikey`.

### Credentials

Run `modn auth nexus set` and pipe the API key as the first line of stdin.
Modn stores the key in the operating system credential store, Windows Credential Manager on Windows.
`modn auth nexus status` reports whether a key is available and from which source.
`modn auth nexus clear` removes the stored key.
A non-empty `MODN_NEXUS_API_KEY` environment variable overrides the stored key.
Modn loads the key only when a Nexus download is needed.
A verified cache hit needs no Nexus request and no key.

Premium accounts need only the API key.
A free account also passes the link from the Nexus "Mod Manager Download" button:

```sh
modn install main/cyberpunk-2077/example --nxm "nxm://cyberpunk2077/mods/6945/files/123456?key=...&expires=..."
modn update example --nxm "nxm://cyberpunk2077/mods/6945/files/123457?key=...&expires=..."
```

`--nxm` is repeatable.
Modn matches each link to a package by game domain, mod ID and file ID.
An unmatched link is an error.
A link expires quickly, and the shell may keep it in history.
Modn does not register as the `nxm://` handler.

### Transport and safety

Modn sends the API key only to `api.nexusmods.com`.
It requests the download link with the `apikey` header and the nxm grant as query values.
The response must be JSON of at most 1 MiB and must name a Nexus CDN host.
The download comes from a validated `nexus-cdn.com` or `nexusmods.com` host without the key.
The file passes through the verified cache and must match its SHA-256 pin.
Secrets never appear in output, state, journals or cache names.
The cache names the file by `nexus://<game_domain>/mods/<mod_id>/files/<file_id>/<file_name>`.

Errors name the action:
- `400` means the nxm link is invalid or expired.
- `401` means Nexus rejected the API key.
- `403` means the account needs Premium, or a free account needs `--nxm`.
- `404` means the Nexus file does not exist.
- `410` means the download link expired.
- `429` means the Nexus rate limit is reached; Modn does not retry automatically.

Ordinary previews make no Nexus request.
`--dry-run --download` may acquire the archive without game writes.

## Git source

A package manifest can name one Git commit as its artifact source:

```json
{"kind":"git","repository":"owner/project","commit":"<40 lowercase hex characters>"}
```

The remote is `https://github.com/<owner>/<project>.git`.
The commit is the integrity pin, so the package has no `sha256`.
A branch, tag, shortened commit, uppercase commit or extra field is refused.
`archive_format` may be `zip` when present; no other format is accepted.

Modn fetches into a private staging directory.
It runs `git init --bare` and then `git fetch --depth 1 --no-tags`.
Git runs as an argv-only child process, without system or global configuration.
The hooks directory is empty, and credential prompts are disabled.
Only HTTPS transport is allowed.
Git must be on PATH.
A fetch or tree-read failure returns `GIT_SOURCE_FETCH_FAILED`.
A commit that Git cannot resolve, or resolves to another object, returns `GIT_SOURCE_COMMIT_MISMATCH`.

Modn reads the tree with `ls-tree` and `cat-file`.
It does not check out files and does not read attributes.
It refuses symbolic links, submodules and other non-regular entries with `GIT_SOURCE_UNSUPPORTED_ENTRY`.
It writes the regular files into a deterministic ZIP.
The ZIP SHA-256 is the archive digest.
The cache is keyed by the package release, commit and digest.
A later install of the same release reuses the cached ZIP and makes no Git call.

Modn fetches only after you approve a download or installation.
`install --dry-run` without `--download` makes no Git call.
It reports that a download is needed.
`install --dry-run --download` may fetch without game writes.
Branches, tags, authentication, Git LFS and submodules are unsupported.
