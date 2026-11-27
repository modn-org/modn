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

## Start a new bucket

`modn catalog init` writes a bucket repository that is ready to sign and to
validate in continuous integration.

```sh
modn catalog init ./my-bucket --name my-bucket --title "My Bucket"
```

The target directory must be missing or empty. A directory with any entry,
including a hidden file, is refused and nothing is written. The command checks
the name and title with the same rules that a registered bucket applies to
`bucket.json`: the name uses lowercase letters, digits and dashes, has at most
64 characters, and is not `main`; the title is non-empty, at most 128 bytes and
free of control characters. A refusal writes nothing.

The scaffold layout is:

```text
bucket.json                          # contract v2 name and title
mods/                                # empty; add one directory per game
PROVENANCE.md                        # record the evidence for every package
templates/package.example.json       # sample record with fake placeholders
.gitignore                           # keeps *.key and key backups out of Git
.github/workflows/validate.yml       # pull request validation
.github/workflows/autoupdate.yml     # scheduled checkver pull requests
```

Git does not track an empty directory. The scaffold keeps `mods/` in the
checkout, and a repository without it is still a valid bucket. Create
`mods/<game-id>/` when you add the first package.

The sample manifest stays outside `mods/`, so the scaffold contains no real
package. Copy it to `mods/<game-id>/<package-id>.json`, replace every
placeholder, and set `blake3` to the value that `modn catalog hash` prints.
The sample uses a `github-release` source and a matching
[`checkver` block](#check-for-and-update-a-newer-upstream-release).

### Validate a pull request

`.github/workflows/validate.yml` runs on every pull request. It installs the
released `modn` of the same version, refreshes the official `main` bucket into a
throwaway state directory, registers the checkout as a local bucket, prints
`modn search --json`, and runs `modn install <bucket>/<game>/<id> --download
--dry-run` for every `mods/<game-id>/*.json` record. The dry run resolves each
dependency, verifies the archive pin in the temporary cache, and never writes
to a game. The workflow builds a small temporary game directory
for the games that Modn supports; add the marker of another game to its
`prepare_game` function when your catalog targets it.

Pull request validation cannot sign, because the secret key never enters
continuous integration. It never runs `modn catalog verify`.

### Refresh manifests on a schedule

`.github/workflows/autoupdate.yml` runs weekly and on demand. It runs
`modn catalog checkver . --update`, and when that rewrites a manifest it creates
`checkver/update-<run id>`, commits the manifests, pushes only that branch and
opens a pull request with `gh pr create`. It has no access to a signing key and
never signs. The pull request body says that `catalog.sig` is not updated:
review the manifest changes, run `modn catalog sign` and `modn catalog verify`,
then commit the new signature before a release.

Both workflows install Modn with
`cargo install --locked --git https://github.com/modn-org/modn --tag v<version> modn-cli`,
because Modn publishes a Windows executable only. The scaffold substitutes the
version of the Modn release that ran `modn catalog init`.

## Create a signing key

Run `modn catalog keygen` once per author identity. Pass a directory outside the
catalog, kept off the repository.

```sh
modn catalog keygen --out ../author-keys
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

### Record where to check for a new release

A record may also carry an optional `checkver` object. Catalog authoring tools
read this block to look for a newer upstream release. Modn does not use it when
it searches, installs, updates or syncs a bucket. The block is part of the
signed catalog content, so sign again after you change it.

Use one of three kinds. `github-release` selects a GitHub release asset, and
`prerelease` is optional and defaults to `false`:

```json
{
  "kind": "github-release",
  "repository": "owner/repo",
  "asset_pattern": "Mod-*.zip",
  "prerelease": false
}
```

`git` selects a branch:

```json
{"kind": "git", "repository": "owner/repo", "branch": "main"}
```

`nexus` selects a file in a Nexus Mods game domain:

```json
{
  "kind": "nexus",
  "game_domain": "cyberpunk2077",
  "mod_id": 6945,
  "file_pattern": "Mod-*.7z"
}
```

The field rules and validation rules are in
[bucket checkver metadata](buckets.md#checkver-metadata). `repository` is
exactly `owner/repo`. The patterns are 1 to 200 characters, where only `*` and
`?` are wildcards and a value without either is an exact file name. An invalid
block fails validation with an error that names the package and the field, such
as `checkver.repository`.

## Check for and update a newer upstream release

`modn catalog checkver <dir>` reads every current package record in the catalog
and reports whether the upstream named by its `checkver` block has a newer
release. It needs no manager state, registers nothing, and never writes a file
in the catalog.

```sh
modn catalog checkver .
modn catalog checkver . my-mod
modn catalog checkver . my-game/my-mod --json
```

The command reads `mods/<game-id>/<mod-id>.json` only. It ignores the
`<mod-id>@<version>.json` release files. It checks every package even when one
fails. One line per package reports the status:

- `up_to_date`: the manifest already records the newest upstream release.
- `update_available`: a newer upstream release exists.
- `skipped`: the record has no `checkver` block.
- `error`: the record or its upstream could not be checked. The message follows
  `error:` and never includes a secret.

The exit status is 0 when no package reports `error`, and nonzero otherwise.
`--json` prints one object with the packages in game and ID order, plus counts:

```json
{"packages":[{"game":"my-game","id":"my-mod","status":"update_available",
"kind":"github-release","current_version":"1.0.0","latest_version":"1.1.0",
"error":null}],"summary":{"up_to_date":0,"update_available":1,"skipped":0,"error":0}}
```

`kind` is null for a skipped package. `latest_version` is null when the check
cannot name a version. A `github-release` check reports the newest eligible
release tag with one leading `v` removed, and compares it with the manifest
version as semantic versions when both parse. A lower upstream version is
`up_to_date`. Set `GITHUB_TOKEN` to raise the GitHub rate limit; Modn sends it
only to `api.github.com`.

A `git` check resolves the head commit of the named branch. A different commit
is `update_available`, and the reported version is `git.<40 lowercase hex
characters>`, for example `git.0123456789abcdef0123456789abcdef01234567`.

A `nexus` check reads the mod's file list and chooses the newest file whose name
matches `file_pattern`. A different file ID is `update_available`. The check
needs an API key from `modn auth nexus set` or `MODN_NEXUS_API_KEY`; without one
the package reports the missing-key `error`. Modn sends the key only to
`api.nexusmods.com`.

A `checkver` block whose kind does not match any package source, such as a
`git` check on an archive-only record, is an `error` for that package.

## Update a package record from upstream

`--update` also refreshes every package whose status is `update_available`, or
the one package you name. It stays stateless and changes only
`mods/<game-id>/<mod-id>.json`. It never reads or writes `catalog.sig`,
`catalog.pub`, `*.key`, `rotation.*` or `revocation.*`.

```sh
modn catalog checkver . --update
modn catalog checkver . my-game/my-mod --update --keep-previous
```

For a `github-release` package, Modn downloads the selected asset to a temporary
file, refuses a download URL that is not the canonical
`https://github.com/<repo>/releases/download/<tag>/<asset>`, and bounds the
download to the 4 GiB archive limit. It hashes the bytes as they stream and
writes the new `version` (the tag without one leading `v`), the source `tag` and
`asset`, the `archive_format` from the asset extension (`.zip` or `.7z`; any
other extension is an error) and the new `blake3` pin. The pin equals what
`modn catalog hash` prints for the same bytes. Modn removes `upstream_sha256`,
which described the replaced artifact, and says so in the output.

For a `git` package, Modn sets the source `commit` to the resolved branch head
and the version to `git.<40 lowercase hex characters>`. A git record carries no
`blake3`; the commit is its pin.

A `nexus` package whose update is available reports the error
`nexus checkver update is not supported: update the manifest by hand` under
`--update`, because a Nexus download needs your own authorization. Check-only
still works for it.

Modn updates only the source that the checkver kind selects. Other fetchable
sources stay unchanged, and the output warns that they no longer match the new
pin. A failed download, hash, validation or write makes that package an
`error`, leaves its record byte-identical, continues with the other packages,
and makes the exit status nonzero.

The rewrite is atomic and minimal. Modn validates the new document, writes it
through a temporary file in the same directory, renames it over the record, and
re-reads and re-validates it. Key order, indentation (two or four spaces) and the
trailing newline stay as they were, so the diff touches only the changed values.
No temporary file remains in `mods/`.

`--keep-previous` requires `--update`. Before replacing a record, Modn writes
its exact old bytes to `mods/<game-id>/<mod-id>@<old-version>.json`. If that file
already exists with different bytes, the package is an `error` and Modn writes
nothing for it; identical bytes are accepted.

After writing at least one record, the command prints the reminder:
`Run 'modn catalog sign' and 'modn catalog verify' before you publish; checkver never changes catalog.sig.`

With `--update`, `--json` keeps the check-only object and adds `updated` (a
boolean) and `kept_previous` (the relative release path or null) to each
package, `updated` (a count) to the summary, and the top-level `reminder` only
when a record was written.

## Sign the catalog

Sign after every change to a catalog file. The signature covers a canonical
digest of the whole tree, not the modification time.

```sh
modn catalog sign . --key ../author-keys/catalog.key
```

It writes `catalog.sig` and `catalog.pub` at the catalog root, prints the
fingerprint and the digest, and exits 0. It replaces an older `catalog.sig`
atomically, so a failed run keeps the previous signature.
Pass `--revocation-public-key <file>` to copy an offline revocation public key
to `revocation.pub` before signing. The copied file is part of the signed
digest, so sign again after any change to it.

`modn catalog sign` refuses these inputs and writes nothing:

- a secret key that lives inside the catalog directory;
- a path that is not a directory;
- an encrypted secret key without `--password-env`;
- a password that does not decrypt the key.

Number a distributed catalog so that users can refuse a rollback:

```sh
modn catalog sign . --key ../author-keys/catalog.key --sequence next
```

`--sequence next` writes the next release number: the number in the existing
file plus one, or 1 when the file is absent. When the tree publishes a
`rotation.json` or `revocation.json` that names a higher number, `sign` uses
that number instead, because a revision must never be older than a statement
inside it. `--sequence <N>` writes a number you choose, but not below the
file's number or a published statement's number.
`--expires <time>` adds a strict UTC expiry in the future and needs a number
from `--sequence`, an existing `sequence.json` or a numbered key statement. The
file is rewritten completely from the flags, so `expires` appears only when you
pass it, except when `sign` only raises the number to a statement's number: then
it keeps the existing expiry. A number lower than the one already
in the file or a published statement is refused and writes nothing. Once a
release is numbered, every
later release needs a higher number; an unnumbered or lower release is refused
as a rollback. Without these flags the catalog is signed as before, and the
output points out that a numbered catalog protects users against rollback. See
[signed sequence numbers](buckets.md#signed-sequence-numbers).

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

It exits 0 and prints the fingerprint and the digest on success. A numbered
catalog also prints `sequence: <N>` and `expires: <time>`; an unnumbered catalog
prints `sequence: unnumbered`. It writes no file and no manager state, and it
refuses a malformed `sequence.json`.
A wrong key exits nonzero and names the signature key and the expected key.
A missing `catalog.sig`, a changed file, an added file and a deleted file each
exit nonzero with a message that names the current digest.

Run this command after `modn catalog sign` and before you push.

## Rotate or revoke a signing key

Create the successor key and the offline revocation key outside the catalog.
Keep the revocation secret key offline; it signs no catalog revision.

At the first publish, name the revocation key so users pin it:

```sh
modn catalog sign . --key ../author-keys/catalog.key --revocation-public-key ../author-keys/revocation.pub
```

To hand the catalog to a successor, rotate with the current signing key:

```sh
modn catalog rotate . --key ../author-keys/catalog.key --new-public-key ../author-keys/next.pub
modn catalog sign . --key ../author-keys/next.key
```

If the current signing key leaked, revoke it with the offline key instead:

```sh
modn catalog revoke . --revocation-key ../author-keys/revocation.key --revoked <OLD_KEY_ID> --replacement-public-key ../author-keys/next.pub
modn catalog sign . --key ../author-keys/next.key
```

`rotate` writes `rotation.json` and `rotation.sig`. `revoke` writes
`revocation.json` and `revocation.sig`. A numbered catalog writes the statement
for the revision that will carry it: the number in `sequence.json` plus one, so
release N+1 carries a statement that names N+1. Neither command changes
`sequence.json`; the following `modn catalog sign` signs that number and writes
it to the file, keeping any `expires`. `--sequence <N>` sets the number
explicitly and must be at least the file's number plus one. On an unnumbered
catalog a statement carries no number unless you pass `--sequence`, which
numbers the catalog at the following `sign`. Both commands refuse a secret key
inside the catalog directory.
A refusal writes nothing. Both commands read the catalog from a staged copy
without a root `.git` entry, so they work in a Git checkout. Commit the statement
files, the new signature, `sequence.json` when it changed, and `catalog.pub`
together.

A rotation keeps module trust because the old key vouched for the new key. A
revocation clears it, so users run `modn bucket trust-modules` again. Sync output
names both fingerprints.

A rotation may return to a key that an earlier rotation retired, but it needs a
new statement. Modn applies each rotation or revocation statement once. A
revision that keeps the pinned `catalog.pub` and republishes an applied
statement treats it as history and syncs normally. The replay check runs only
when no new statement applies to the pinned key. For example, after `A->B->A`,
a revision that keeps the applied `A->B` rotation and adds a valid new `A->C`
revocation is accepted. A revision signed by B that keeps `A->B` and adds
nothing that applies is refused, so a retired key cannot regain trust by
replaying an old statement. Modn records only the key that the current revision rotated away
from, so the current key is never listed as retired. A rotation cannot name a
key in `revoked_signers`; only the offline revocation key can re-authorize a
revoked key. A revocation may name a retired or revoked key as its replacement,
and that re-authorizes it. After a revocation, module releases imported under the
revoked key stay untrusted. If a later revocation re-authorizes that key as its
replacement, the retained releases are trusted again after `trust-modules`. A
new release carries the replacement key and becomes usable after
`trust-modules`.

Keep accepted statements published. Each user applies the one statement whose
old key matches their pinned key. Every other statement is history, and a new
user registers the catalog normally: the first sync pins `catalog.pub` and
ignores the published statements as history. Do not publish two statements that
both apply to the same pinned key. If a catalog needs two key changes, publish
and sync one intermediate revision before the next.

A pinned `revocation.pub` cannot change in place. If the revocation key leaks,
each user must remove the bucket and sync it again to pin a new revocation key.
See [bucket key rotation](buckets.md#catalog-key-rotation-and-revocation).

## Host the catalog in a Git repository

Put the catalog files at the root of a repository, sign them in place and commit
them.

```sh
modn catalog sign . --key ../author-keys/catalog.key
git add --all
git commit -m "Publish catalog"
git push origin main
```

Stage `bucket.json`, `mods/`, `games/` when present, `sequence.json` when present,
`catalog.sig`, `catalog.pub`, `revocation.pub` when present, and every published
`mirrors.json`, `relocation.json`, `rotation.json`, `rotation.sig`,
`revocation.json` and `revocation.sig`. The staged files must match the signed
digest.

Keep `catalog.sig` and `catalog.pub` at the root. Commit new signatures with each
catalog change. A user then registers the catalog once:

```sh
modn bucket sync community --source https://example.org/catalog.git --revision main
```

The first sync prints
`Pinned signer key <KEY_ID> for bucket community; compare this fingerprint with the catalog author.`
Tell users the fingerprint that `modn catalog keygen` printed, so each user can
compare it.
At the first sync, Modn pins the signer from `catalog.pub` and ignores any
published rotation or revocation statement as history. Later syncs verify the
revision with the pinned key. They may omit `catalog.pub`. A revision that names
another key needs a signed rotation or revocation statement whose old key is the
pinned key; see [rotate or revoke a signing key](#rotate-or-revoke-a-signing-key).
A changed key without that statement is refused.

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

## Publish mirrors and move a catalog

Add `mirrors.json` at the catalog root to name other hosts of the same signed
catalog. Each mirror must use the same source kind as the registered primary.
A ZIP mirror needs an `https://` URL that ends in `.zip`. A Git mirror needs an
`https://` remote and a revision.

```json
{
  "schema_version": 1,
  "mirrors": [{"kind": "zip", "url": "https://mirror.example/catalog.zip"}]
}
```

List at most 8 mirrors with no duplicates. Then re-sign the catalog and upload
the new ZIP or commit to the primary. A sync tries the primary first and then
the listed mirrors in order. Users do not re-register.

To move a catalog to a new host, copy the signed catalog there first. Then add
`relocation.json` at the catalog root of the old host:

```json
{
  "schema_version": 1,
  "primary": {"kind": "zip", "url": "https://new.example/catalog.zip"}
}
```

Re-sign the catalog and publish it at the old host before the old host goes
away. The next sync verifies the revision with the pinned key, follows the
record and registers the new primary. Tell users the new URL. A relocation to
the same primary does nothing.

A mirror list, a relocation record or a catalog revision that another key
signed is refused. Modn keeps the previous registration and catalog.

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
5. Run `modn catalog sign` (add `--sequence next` to number the release) and then
   `modn catalog verify`.
6. Commit `mods`, `bucket.json`, `sequence.json` when present, `catalog.sig` and
   `catalog.pub`, then push.
7. Tell users the fingerprint and, for a local check, the digest.

A user checks the release with `modn bucket sync community` and
`modn install <package>`.

## What later work adds

No catalog authoring capability named on this page is planned for later work.
