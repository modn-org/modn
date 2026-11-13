# Operation history

Modn records each committed install, update and uninstall in manager state.
`modn history` prints those records newest first. History is evidence of what
Modn changed; it is not a second ownership ledger and grants no authority.

## What a record holds

Every record has a stable, short operation ID, its kind (`install`, `update`
or `uninstall`), the registered game ID, the canonical installation root and a
UTC timestamp. For each package it holds:

- the catalog mod ID, or the standalone archive stem;
- the managed-package ID before and after the operation, when each exists;
- the release before and after, with its version and source descriptor;
- the release's qualified `bucket/game/mod` catalog identity, and its
  declared dependencies, conflicts and native-injector flag, when recorded;
- the archive SHA-256, the current cache key, and BLAKE3 digest, written as
  `blake3:<hex>`;
- every owned file the operation created, replaced or removed, with its
  SHA-256 content digest before and after.

A source descriptor uses the bucket manifest shape: `manual`, `https`,
`github-release`, `nexus` or `git`. A standalone archive is a `manual` source.
The descriptor is stored in the record, so undo does not need the bucket
snapshot or catalog revision that produced it.

```json
[
  {
    "id": "txn-a1b2c3",
    "kind": "update",
    "game_id": "cyberpunk-2077",
    "installation": "C:\\Games\\Cyberpunk 2077",
    "recorded_at": "2026-11-09T14:27:35.120Z",
    "undoes": "txn-0f1e2d",
    "packages": [
      {
        "mod_id": "red4ext",
        "package_id_before": "txn-0f1e2d",
        "package_id_after": "txn-a1b2c3",
        "release_before": {
          "version": "1.24.0",
          "catalog": {"bucket": "main", "game_id": "cyberpunk-2077", "mod_id": "red4ext"},
          "declarations": {"dependencies": [], "conflicts": [], "native_injector": true},
          "source": {"kind": "github-release", "repository": "WopsS/RED4ext", "tag": "v1.24.0", "asset": "red4ext-1.24.0.zip"},
          "archive_sha256": "…",
          "archive_blake3": "blake3:…"
        },
        "release_after": {
          "version": "1.25.0",
          "catalog": {"bucket": "main", "game_id": "cyberpunk-2077", "mod_id": "red4ext"},
          "declarations": {"dependencies": [], "conflicts": [], "native_injector": true},
          "source": {"kind": "github-release", "repository": "WopsS/RED4ext", "tag": "v1.25.0", "asset": "red4ext-1.25.0.zip"},
          "archive_sha256": "…",
          "archive_blake3": "blake3:…"
        },
        "files": [
          {"root": "game", "destination": "red4ext/Red4ext.dll", "digest_before": "…", "digest_after": "…"}
        ]
      }
    ]
  }
]
```

`digest_before` is absent for a created file. `digest_after` is absent for a
removed file. An install has `release_after` only; an uninstall has
`release_before` only. A record written by `modn undo` also has an `undoes`
field that names the operation it reversed. Other records omit the field.

## Commands

```sh
modn history
modn history --json
modn history --game cyberpunk-2077 --json
modn history --state-dir D:\modn-state --json
```

Text output is a short table of ID, kind, game, packages and time. JSON
output is one array of operations, newest first. `--game` keeps one game ID.
`--state-dir` selects the manager profile that holds `state.sqlite`, including
its history. Reading history never creates a database.

## Undo

`modn undo [<operation-id>] --yes` reverses one committed operation. Without an
ID it selects the newest operation for the selected installation. `--path`
selects a registered installation; with several installations of one game, an
omitted path is refused instead of guessed. An explicit ID from another
installation or game is refused with `UNDO_NOT_FOUND`, and the message names
the installation that owns the record. The inverse follows the recorded kind:
an install is removed, an update restores the earlier release, and an uninstall
restores the recorded release. An undo writes a normal record for the change it
makes and sets `undoes` to the target's ID. It never edits or deletes the target
record.

The undo refuses before any change in these cases:

- A later operation on the same installation touched one of the target's
  packages or files. The error names that newer operation; undo it first. A
  second undo of the same target is refused for the same reason.
- The target release has a `manual` source, so Modn has no URL to fetch again.
  Install the archive with `modn install --file` instead.
- The target release has a `git` or `nexus` source and its archive is no
  longer cached. Undo restores these releases only from the archive cache; a
  Nexus download needs a new nxm link. Reinstall the release from the catalog.
  A `github-release` source reuses its verified cached archive without
  contacting GitHub, and downloads again only on a cache miss.
- A restored archive does not match the recorded SHA-256, or the current game
  policy maps the release to another root or other files than the record.
- An update undo or an uninstall undo selects an archive member whose SHA-256
  differs from the recorded `digest_before` of a restored file. The undo checks
  the selected members before any game file changes. The error is `UNDO_FAILED`
  and no game file, ownership row or history record changes.
- A package is pinned, another installed package needs it, an unfinished
  transaction exists, or ownership conflicts block the change.

`modn undo --dry-run` runs the restore preflight checks, including the selected
archive member digests and the native-injector policy, and reports the same
refusal without writing. It does not start a transaction, so it does not run
the checks that need the live installation: a pinned package, a package that
another installed package needs, an unfinished transaction and an ownership
conflict. The real undo can still refuse for those. A cancellation before the
commit point stops the archive reads and leaves the same state unchanged.

An update undo and an uninstall undo rebuild the release from the record's
catalog identity, dependencies, conflicts and native-injector flag. The
restored release therefore keeps the catalog name for `modn update`, blocks
uninstalling a package it requires, and is checked against the native-injector
policy. A restored release that is a native injector needs its stored approval
or `--allow-native-injector`; otherwise the undo refuses with
`NATIVE_INJECTOR_NOT_ALLOWED` before a game write. Executable restores need the
stored approval or an explicit `--allow-executable` path, as an install does.

A history record written before this release data was stored has no
`declarations`. Undo of its update or uninstall refuses with `UNDO_FAILED` and
asks you to install the release again; it changes nothing. Undoing an install
still removes its packages, because removal needs no release data. A
multi-package undo uses one stack or one multi-package removal, so it commits
every package or none.

## Storage and consistency

History lives in `state.sqlite`:

| Table | Contents |
|---|---|
| `history_operations` | ID, kind, game ID, installation root, UTC time and the optional `undoes` target; an autoincrement sequence orders records. |
| `history_packages` | One row per package with before/after package IDs and serialized release descriptors. |
| `history_files` | One row per owned file with before/after SHA-256 digests. |
| `managed_package_releases` | Release version, source descriptor, archive BLAKE3, qualified catalog reference and declared dependencies, conflicts and native-injector flag for each installed package, so a later update or uninstall can record the side it removes. |

These tables are additive and add no schema version of their own. A writable
open creates them when they are missing, and it also creates the approvals
table, so the manager schema version of state that this build writes is 5 (see
[the data model](data-model.md#approvals-schema-version-5)). An older Modn
build refuses that state. The `catalog` and `declarations` columns were added
to `managed_package_releases` in a later release; a writable open adds them
only when they are absent, and a read-only open writes nothing. Rows written
before that change have no values there, like packages installed before the
table existed: their removal records the archive SHA-256 without version,
source or declarations.

The record is inserted in the same SQLite transaction that changes ownership:

- Install and update: the transaction that records the new package and retires
  the old release also inserts the record. An update record has both release
  sides and the union of old and new files.
- Uninstall and forced cleanup: the transaction that deletes ownership also
  inserts the record. Each removed package is one record, so `--include-deps`
  and `--cascade` create one record per removed package.
- Stack install: member ownership commits first, and finalizing the stack
  inserts one record for all members in the transaction that removes the stack
  marker. An interrupted stack is rolled back by `modn recover` and leaves no
  record.

A cancelled operation before its commit point, a refused or dry-run command,
and an automatic or explicit rollback therefore write no record. If history
cannot be written, the ownership transaction fails and rolls back with it. A
crash during the SQLite commit leaves either all of the record and ownership
or neither. History therefore never disagrees with ownership.
