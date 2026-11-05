# Download cache

Modn keeps downloaded archives in a cache, after Scoop's download cache. The
cache is `<state-dir>/cache` unless `MODN_CACHE` names another directory.
Each entry is named `<game>#<mod>#<version>#<url hash><extension>`. Modn
downloads only over HTTPS, and redirects must stay on HTTPS. A download
fails if the server sends nothing for 60 seconds. Each attempt writes its own
`<entry>.<random>.download` file, so concurrent downloads never share bytes.
Modn checks the manifest's SHA-256 and a 4 GiB size limit, then atomically
renames the file into place. A failed check removes that attempt's partial
file and prints the URL with the expected and actual digests. Unlike Scoop,
an entry exists only after its digest is verified. Modn verifies an entry
again each time it uses it; an entry that fails is never used, and the next
download replaces it. The cache directory, like the state directory, must be
outside every game installation; Modn checks this before it downloads. A dry
run reads the cache but never downloads. Modn downloads only after you
confirm. A single mod asks `Download? [y/N]` first, because its install
prompt needs the archive. A stack checks that every `manual` archive is
supplied, then asks once for the whole stack.

- `modn install <mod> --no-cache` downloads again instead of using an entry.
- `modn cache show [mod]` lists entries and their total size.
- `modn cache rm <mod>` or `modn cache rm --all` removes entries. A file
  counts as an entry only if its whole name matches the entry format. Other
  files, including partial downloads, are left alone.

## Digest-based reuse

Cache lookup can reuse verified payloads across package or source aliases with the same pinned SHA-256.
Every hit verifies bytes. Reuse creates no alias file and never deduplicates mutable game files.
See [sources.md](sources.md) for removal behavior and provider acquisition.
