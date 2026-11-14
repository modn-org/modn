# Publishing a signed catalog

A catalog is a directory that lists mod packages for Modn users.
A distributed catalog is signed. Modn refuses an unsigned catalog.
This page shows how to create a signing key, sign the catalog, verify it and
publish it. It also shows how to host the archives that the catalog lists.

The author tools write and check the same minisign signature that
`modn bucket sync` verifies. They do not need manager state and they do not
register a catalog. See [buckets](buckets.md) for the catalog contract and
[command reference](cli.md#catalog-authoring) for the commands.

## Catalog layout

```text
bucket.json                        # schema_version, name and title
mods/<game-id>/<package-id>.json   # package record of the current release
mods/<game-id>/<package-id>@<version>.json  # other releases, optional
catalog.sig                        # written by modn catalog sign
catalog.pub                        # written by modn catalog sign
```

`bucket.json` holds `{"schema_version": 2, "name": "<catalog name>", "title": "<title>"}`.
The `name` must be unique among the catalogs that a user registers.

To move an existing contract v1 `main` checkout to contract v2, see [Rehash main for contract v2](releasing.md#rehash-main-for-contract-v2).

## Create a signing key

Run `modn catalog keygen` once per author identity. Pass a directory outside the
catalog, kept off the repository.

```sh
modn catalog keygen --out ./author-keys
```

It writes `catalog.key` and `catalog.pub`, prints the fingerprint and exits 0.
The fingerprint is the 16 hexadecimal digit minisign key ID, shown in uppercase.
Users compare it with the value that `modn bucket sync` prints.

The command never prints the secret key. It refuses to overwrite an existing key
file. On Unix the secret key file gets mode 0600. On Windows the file inherits
the access control list of the user directory, so keep the directory private.

Use `--name <name>` for another file name. The default is `catalog`.
Use `--out <dir>` for another directory. The default is the current directory.

### Protect the secret key with a password

Pass `--password-env <VAR>` with the name of an environment variable that holds
the password. Never put the password in a command argument.

```sh
modn catalog keygen --out ./author-keys --password-env MODN_CATALOG_KEY_PASSWORD
```

An encrypted key needs the same variable for `modn catalog sign`. A missing
variable and a wrong password are refused. Each refusal writes nothing.

Keep the secret key offline. A common arrangement uses a separate machine or a
removable disk that holds `catalog.key` and is not connected during normal work.
Copy only `catalog.pub` into the catalog. Never commit `catalog.key` and never
upload it. A leaked secret key lets anyone publish a catalog that users accept
under your fingerprint.

## Pin an archive

`modn catalog hash` prints the BLAKE3 digest of an archive file. The value goes
into the package record as `blake3`.

```sh
modn catalog hash ./my-mod-1.0.zip
```

It prints 64 lowercase hexadecimal characters and a newline, and nothing else.
Use `--json` to get `{"blake3":"<hex>"}` instead.
The command streams the file and never loads it whole. It refuses a missing file
and a directory.

## Write the package record

Put one record at `mods/<game-id>/<package-id>.json`. A record names the archive
that it pins and at least one source.

```json
{
  "schema_version": 2,
  "id": "my-mod",
  "game": "cyberpunk-2077",
  "name": "My Mod",
  "version": "1.0",
  "archive_format": "zip",
  "blake3": "<64 lowercase hex characters from modn catalog hash>",
  "sources": [
    {"kind": "https", "url": "https://mods.example.org/my-mod/my-mod-1.0.zip"}
  ]
}
```

The `blake3` value must equal the archive bytes. Modn verifies a download against
that pin before it installs anything.
`sources` is an ordered list. Modn tries the entries in order and refuses an
archive that does not match the pin.
See [buckets](../buckets/README.md) for every field.

## Sign the catalog

Sign after every change to a catalog file. The signature covers a canonical
digest of the whole tree, not the modification time.

```sh
modn catalog sign . --key ../author-keys/catalog.key
```

It writes `catalog.sig` and `catalog.pub` at the catalog root, prints the
fingerprint and the digest, and exits 0. It replaces an older `catalog.sig`
atomically, so a failed run keeps the previous signature.

`modn catalog sign` refuses these inputs and writes nothing:

- a secret key that lives inside the catalog directory;
- a path that is not a directory;
- an encrypted secret key without `--password-env`;
- a password that does not decrypt the key.

The digest covers every regular file below the root except the root
`catalog.sig` and `catalog.pub`. A changed, added or deleted file changes the
digest. Modn refuses a symbolic link, another non-regular entry and two paths
that differ only by case. A root `.git` entry is not part of the published tree,
so `modn catalog sign` and `modn catalog verify` ignore it. Sign the repository
root directly.

## Verify before publishing

Use the public key to check the catalog without registering it.

```sh
modn catalog verify . --key catalog.pub
```

It exits 0 and prints the fingerprint and the digest on success. It writes no
file and no manager state.
A wrong key exits nonzero and names the signature key and the expected key.
A missing `catalog.sig`, a changed file, an added file and a deleted file each
exit nonzero with a message that names the current digest.

Run this command after `modn catalog sign` and before you push.

## Host the catalog in a Git repository

Put the catalog files at the root of a repository, sign them in place and commit
them.

```sh
modn catalog sign . --key ../author-keys/catalog.key
git add bucket.json mods catalog.sig catalog.pub
git commit -m "Publish catalog"
git push origin main
```

Keep `catalog.sig` and `catalog.pub` at the root. Commit new signatures with each
catalog change. A user then registers the catalog once:

```sh
modn bucket sync community --source https://example.org/catalog.git --revision main
```

The first sync prints
`Pinned signer key <KEY_ID> for bucket community; compare this fingerprint with the catalog author.`
Tell users the fingerprint that `modn catalog keygen` printed, so each user can
compare it.
Later syncs verify the revision with the pinned key. They may omit `catalog.pub`.
A revision that names another key is refused. Modn has no key rotation yet
(#122), so a lost or leaked key needs a new catalog name.

Git must be installed. The repository may be public or reachable by the user's
Git credentials.

## Host the catalog as a ZIP
A static web server or object store can host a signed catalog as one ZIP file.
Zip the signed catalog directory itself, so the ZIP root holds `bucket.json`,
`mods/`, optional `games/`, `catalog.sig` and `catalog.pub`. Do not add a
top-level folder around the catalog files.
Upload the ZIP and publish its HTTPS URL. A user then registers the catalog once:
```sh
modn bucket sync community --source https://example.org/catalog.zip
```
The source must end in `.zip`. Modn downloads the ZIP, verifies the signature and
publishes it with the same validation as a Git catalog. The first sync prints
the pinned fingerprint. Later syncs verify with the pinned key only.
`--revision` is refused with a ZIP source, because a ZIP has no Git revision.
Re-sign and upload the ZIP again after every catalog change. Keep the same URL so
users refresh it with `modn update`. Host mod archives at separate URLs; the
catalog ZIP holds records, not archives.
A plain `http://` URL is refused. An absolute local ZIP path also works, for
offline testing.

## Host archives at stable HTTPS URLs

Serve each archive at an HTTPS URL that keeps its bytes for as long as the record
names that version. Use one directory per mod and version, for example
`https://mods.example.org/my-mod/my-mod-1.0.zip`.

Rules that keep installs working:

- Keep a published URL and its bytes unchanged. Publish a new version at a new
  URL and add a new record.
- Serve the archive itself, not an HTML page and not a redirect to a login
  prompt.
- Use HTTPS only. Modn refuses other schemes.
- Do not move an archive behind a URL that changes per download.

The `sources` entry also accepts `{"kind": "manual", "filename": "my-mod-1.0.zip"}`
for a file that the user supplies, and `{"kind": "github-release", ...}` for a
GitHub release asset. See [sources](sources.md).

## Publish a release

1. Build the archive.
2. Run `modn catalog hash` on the archive and put the value in the record.
3. Upload the archive to its HTTPS URL and check that the URL serves the bytes.
4. Add or edit the record under `mods/<game-id>/`.
5. Run `modn catalog sign` and then `modn catalog verify`.
6. Commit `mods`, `bucket.json`, `catalog.sig` and `catalog.pub`, then push.
7. Tell users the fingerprint and, for a local check, the digest.

A user checks the release with `modn bucket sync community` and
`modn install <package>`.

## What later work adds

These capabilities are planned and not available:

- mirrors and catalog relocation (#120);
- signer key rotation and offline revocation (#122).

The commands on this page do not implement them. They do not fetch, mirror or
rotate anything, and a catalog with a changed signer key is refused.
