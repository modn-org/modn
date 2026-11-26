# Download cache

Modn keeps downloaded archives in a cache, after Scoop's download cache. The
cache is `<state-dir>/cache` unless `MODN_CACHE` names another directory.
Each entry is named `<game>#<mod>#<version>#<url hash><extension>`. Modn
downloads only over HTTPS, and redirects must stay on HTTPS. A download
attempt fails if the server sends nothing for 60 seconds, and Modn then tries
the next source in the record; see [sources.md](sources.md#artifact-source-fallback). Each attempt writes its own
`<entry>.<random>.download` file, so concurrent downloads never share bytes.
Modn checks the manifest's BLAKE3 pin and a 4 GiB size limit, then atomically
renames the file into place. A failed check removes that attempt's partial
file, and a mismatched body is never cached. A failed source is not retried
after a digest mismatch. When every source fails, one error names each source
and why it failed. A cancelled download also removes its partial file, so the
cache keeps only complete, verified entries. `--progress json` reports the bytes received and the
expected total; see [cli.md](cli.md#progress-events-and-cancellation). Unlike Scoop,
an entry exists only after its digest is verified. Modn verifies an entry
again each time it uses it; an entry that fails is never used, and the next
download replaces it. A release digest is typed: a catalog BLAKE3 pin, a digest
Modn computed after a fetch, or unknown. A Git release has no digest until Modn
fetches and hashes its packed archive. A cache key requires a known digest, so
an unknown digest never names a cache entry. The cache directory, like the state directory, must be
outside every game installation; Modn checks this before it downloads. A dry
run reads the cache and downloads nothing unless you pass `--download`.
`modn install --dry-run --download` and `modn update --dry-run --download`
store missing archives in the cache, so the preview can list their files. They
leave game files unchanged.
`modn undo` acquires the archives that it must restore, so it can fill the cache
even when it runs with `--dry-run`. It stores the entry and records an archive
source's digest in the cache index, and it changes no game file.
Modn otherwise downloads only after you confirm. A single mod asks
`Download? [y/N]` first, because its install prompt needs the archive. A stack
checks that every `manual` archive is supplied, then asks once for the whole stack.

- `modn install <mod> --no-cache` downloads again instead of using an entry.
- `modn cache show [mod]` lists entries and their total size.
- `modn cache rm <mod>` or `modn cache rm --all` removes entries. A file
  counts as an entry only if its whole name matches the entry format. Other
  files, including partial downloads, are left alone.
  `modn cache show` marks an entry named `git-<commit>-<sha256>.zip` as
  retired. An earlier build wrote that name form and its digest is SHA-256.
  Modn lists and removes retired entries, but never uses one for an install.

## Digest-based reuse

Cache lookup can reuse verified payloads across package or source aliases with the same pinned BLAKE3 digest.
Every hit verifies bytes again before use. Reuse creates no alias file and never deduplicates mutable game files.
The cache index maps a `blake3:<hex>` pin to one entry name.
A lookup that finds a missing or unknown index scans the cache files in memory and writes nothing.
A successful archive download stores its entry file and records the pin in the digest index.
A Git source stores a packed `git-<commit>-blake3-<digest>.zip` entry and writes no index.
The scan hashes each cache file with BLAKE3 and re-verifies it before use.
An index that still holds `sha256:` entries is unknown, so Modn ignores it without rewriting it.
A cache lookup changes no cache file.
A cancelled or refused operation leaves every cache file unchanged.
A preflight changes the cache only when it downloads: `install --dry-run --download`, `update --dry-run --download`, and an undo that misses the cache.
See [sources.md](sources.md) for removal behavior and provider acquisition.
