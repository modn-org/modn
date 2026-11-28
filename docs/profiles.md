# Profiles and conflict priority

A profile is a named set of managed packages for one game installation. Each
profile stores an ordered priority. Exactly one profile is active for an
installation, and the active profile decides the files on disk.

Registering an installation creates an active profile named `default`. An
install also creates `default` when the installation has no profile. New
installs join the active profile at the next priority.

## Commands

```sh
modn profile list [--path DIR]
modn profile create <name> [--clone <name>] [--path DIR]
modn profile delete <name> --yes [--path DIR]
modn profile priority <package> (--before <package> | --after <package> | --rank N) \
  [--profile NAME] [--apply] [--dry-run] [--yes] [--path DIR]
modn profile enable <package> [--profile NAME] [--dry-run] [--yes] [--path DIR]
modn profile disable <package> [--profile NAME] [--dry-run] [--yes] [--path DIR]
modn profile switch <name> [--dry-run] [--yes] [--path DIR]
modn profile apply [--profile NAME] [--dry-run] [--yes] [--path DIR]
modn conflicts [--profile NAME] [--path DIR]
```

Every command accepts the global `--json` flag. `profile switch` also accepts
`--progress json`. A successful command exits 0. A refusal exits 1; `--json`
names its error code.

- `profile list` marks the active profile and prints each profile's packages in
  rank order. JSON has `game`, `path` and `profiles`. Each profile has `name`,
  `active`, `created_at` and `packages`. Each package has `package`, `rank`,
  `priority`, `enabled` and `shadowed_files`.
- `profile create` makes an inactive profile. `--clone` copies the
  memberships, priorities and enabled flags of another profile. A duplicate
  name fails with `PROFILE_EXISTS`.
- `profile delete --yes` deletes an inactive profile. It removes the
  memberships and every package that no other profile contains. It changes no
  game files. The active profile fails with `PROFILE_ACTIVE`. The only profile
  fails with `PROFILE_LAST`. Modn checks the active profile first, so the only
  profile normally reports `PROFILE_ACTIVE`.
- `profile priority` moves one package. `--before` and `--after` name another
  package. `--rank` takes a one-based rank, where rank 1 is the winner.
  `--profile` defaults to the active profile.
- `profile enable` and `profile disable` change one membership. `--profile`
  defaults to the active profile. `--dry-run` prints the file changes without
  writing.
- `profile switch` makes another profile active. The plan lists removed,
  written, replaced and unchanged paths, plus entering and leaving packages.
- `profile apply` writes the priority winners of the active profile to disk.

Without `--apply`, `profile priority` changes ranks only. Game files keep the
previous winners. When the move changes a winner, `modn conflicts` reports
`consistent: false` and `modn doctor` reports `winner_mismatch`. Run
`modn profile apply --yes` to repair the files. With `--apply`, one run changes
the ranks and writes the new winners. It needs `--yes` when files change.
`--dry-run` writes nothing.

`profile apply` refuses an inactive profile with `PROFILE_FAILED`, also with
`--dry-run`. Switch to the profile first.

An enable or disable on the active profile changes the files when the requested
state differs. A file change needs `--yes`. A disable removes the package's
winning files, or restores the next enabled claimant from its verified archive.
An enable writes the package's bytes on each path where it wins. On an inactive
profile, only the enabled flag changes. Toggles write no history record.

When a winner is uninstalled, Modn restores the next claimant's verified bytes
from the archive cache. A missing or mismatching archive refuses the command
before any change with `PROFILE_ARCHIVE_MISSING`. The message names the
package, the path and `modn cache show`. A changed managed file refuses with
`PROFILE_DRIFT`; run `modn doctor`. Disable, apply and switch use the same
checks.

## Conflict priority

Two enabled packages of one profile may claim the same path. Modn allows the
claim. The package with the highest stored priority wins the path. A higher
priority value wins, so rank 1 is the winner. A new install joins at the next
priority and therefore wins a shared path.

The losing package keeps a **shadowed claim**. Its file is not the active copy,
but the claim stays recorded. The claim supplies the bytes again when the
winner leaves.

An unmanaged file is never overwritten. Manifest-declared `conflicts` between
mods stay hard refusals with `CONFLICT`. Priority resolves path overlap only.

`modn conflicts` is read-only. It lists each path that two or more enabled
packages of the selected profile claim. Claims appear in priority order and
the winner is marked. `--json` prints `game`, `path`, `profile`, `active` and
`conflicts`. Each conflict has `root`, `path`, `winner`, `consistent` and
`claims`. Each claim has `package`, `rank`, `priority`, `winner`, `blake3` and
`bytes`. An inactive profile has a null `winner` and `consistent: true`.

## Switch and recovery

`profile switch <name>` makes another profile active. `--dry-run` prints the
plan and writes nothing. A real run needs `--yes`. Refusals come in this order:
`PENDING_TRANSACTION`, `PROFILE_NOT_FOUND`, `PROFILE_ACTIVE`, `PROFILE_DRIFT`,
`PROFILE_ARCHIVE_MISSING`, then `CONFLICT` for an unmanaged file. Modn checks
again under the transaction lock.

With `--progress json`, a `cancel` line on stdin before the commit point stops
the switch with `CANCELLED`. The old profile stays active and every game byte
stays unchanged.

One SQLite transaction commits the active profile, the winner rows and a
history record of kind `profile_switch`. `modn undo` of that record fails with
`UNDO_CONFLICT`; reverse a switch with a new switch.

A switch never touches an unmanaged file. Exclusive directories belong to the
installation, so a switch neither removes nor recreates them. It creates a
missing parent directory for a written file. A rollback does not remove that
directory, so an empty directory can remain.

`modn recover --yes` resolves an interrupted switch, restore, toggle or apply.
The old profile stays active, or the new profile is active, but never a mix.
A file that you edited after an interrupted transaction is kept, and recovery
reports it with `CONFLICT`. See
[transactions.md](transactions.md#profile-switch-transactions) for the journal
and recovery evidence.

| Crash point | Recovery result |
|---|---|
| Before the journal | No pending transaction. The old profile stays active. |
| After the journal, before a move | The journal rolls back. The old profile stays active. |
| After some quarantines | Every `.old` witness returns. The old profile stays active. |
| After all moves, before the commit | Replaced and removed paths return; created paths go away. The old profile stays active. |
| After the commit, before cleanup | The new profile stays active. Witnesses and staging are removed. |

## Known limits

- An update of a package that shares a path with another managed package fails
  with `CONFLICT`. It applies to a winner and to a shadowed claim. Uninstall
  and reinstall the overlapping package first.
- `modn undo` does not reverse a profile switch.
- State schema is 8. An earlier schema is refused and there is no migration.
- This milestone adds the CLI only. The desktop and MCP inherit the new
  `list --json` and `doctor --json` keys.

## Worked example

The example uses a disposable `cyberpunk-2077` installation. Two archives write
the same file, `archive/pc/mod/overlap.archive`, and a third archive writes
its own file.

```sh
GAME=C:/fixtures/game
STATE=C:/fixtures/state
modn --game cyberpunk-2077 --state-dir "$STATE" game add cyberpunk-2077 --path "$GAME"
modn --game cyberpunk-2077 --state-dir "$STATE" install C:/fixtures/archives/alpha.zip --path "$GAME" --yes
modn --game cyberpunk-2077 --state-dir "$STATE" install C:/fixtures/archives/first.zip --path "$GAME" --yes
modn --game cyberpunk-2077 --state-dir "$STATE" install C:/fixtures/archives/second.zip --path "$GAME" --yes
modn --game cyberpunk-2077 --state-dir "$STATE" conflicts
```

The later install wins the shared path. The output is abridged to the lines
that matter.

```text
game/archive/pc/mod/overlap.archive
  WINNER #1 txn-81kYd8 priority 2 blake3 cd856376 6 bytes
         #2 txn-Hq4W0i priority 1 blake3 22896bcb 5 bytes
  on disk: txn-81kYd8

1 overlapping files
```

Create a spare profile. Then raise the older package to rank 1 without an
apply.

```sh
modn --game cyberpunk-2077 --state-dir "$STATE" profile create spare
modn --game cyberpunk-2077 --state-dir "$STATE" profile priority txn-Hq4W0i --rank 1
```

```text
Created inactive profile "spare" with 0 packages.
txn-Hq4W0i is now rank 1 in profile "default". Game files still follow the previous priority order; run 'modn profile apply --yes' or repeat with --apply.
```

The ranks changed, but the old bytes remain. `conflicts` names the mismatch and
`doctor` reports it.

```text
  WINNER #1 txn-Hq4W0i priority 2 blake3 22896bcb 5 bytes
         #2 txn-81kYd8 priority 1 blake3 cd856376 6 bytes
  on disk: txn-81kYd8; run `modn doctor`
```

```text
Profile: default (3 packages, 1 overlapping files, inconsistent)
profile issue winner_mismatch: archive/pc/mod/overlap.archive (txn-Hq4W0i) the stored winner is txn-81kYd8
  Next: modn profile apply --yes
```

Apply the order. Then disable and enable the winner to show the file changes.

```sh
modn --game cyberpunk-2077 --state-dir "$STATE" profile apply --yes
modn --game cyberpunk-2077 --state-dir "$STATE" profile disable txn-Hq4W0i --yes
modn --game cyberpunk-2077 --state-dir "$STATE" profile enable txn-Hq4W0i --yes
```

```text
Changed 1 file(s):
  replace archive/pc/mod/overlap.archive with txn-Hq4W0i (was txn-81kYd8)
Changed 1 file(s) for txn-Hq4W0i in profile "default":
  restore archive/pc/mod/overlap.archive with txn-81kYd8 (was txn-Hq4W0i)
Changed 1 file(s) for txn-Hq4W0i in profile "default":
  replace archive/pc/mod/overlap.archive with txn-Hq4W0i (was txn-81kYd8)
```

Preview and run a switch to the empty profile. The dry run prints the same plan
without the final line.

```sh
modn --game cyberpunk-2077 --state-dir "$STATE" profile switch spare --dry-run
modn --game cyberpunk-2077 --state-dir "$STATE" profile switch spare --yes
```

```text
Switching profile "default" -> "spare"
  2 to remove, 0 to write (0 bytes), 0 unchanged
  entering: (none)
  leaving: txn-81kYd8, txn-Hq4W0i, txn-klJ7ii
  remove archive/pc/mod/alpha.archive (game)
  remove archive/pc/mod/overlap.archive (game)
Switched to profile "spare".
```

`profile list` marks the new active profile. Switch back and uninstall the
winner. The shadowed package supplies the file again.

```sh
modn --game cyberpunk-2077 --state-dir "$STATE" profile list
modn --game cyberpunk-2077 --state-dir "$STATE" profile switch default --yes
modn --game cyberpunk-2077 --state-dir "$STATE" uninstall txn-Hq4W0i --yes
```

```text
PROFILE  ACTIVE  PACKAGES
spare    *       none
default          #1 txn-Hq4W0i, #2 txn-81kYd8, #3 txn-klJ7ii
Uninstalled txn-Hq4W0i
```

After the uninstall, `archive/pc/mod/overlap.archive` holds the bytes of
`txn-81kYd8`.

The `winner_mismatch`, `shadowed_claim_unrecoverable` and
`other_profile_archive_missing` kinds are the profile health issues that
`modn doctor` reports. See [inventory.md](inventory.md#profile-health).
