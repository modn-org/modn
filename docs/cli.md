# Command reference

## Commands

Game registration, mod installation and bucket management use separate commands:

```sh
modn game detect --json
modn game detect --register
modn game add cyberpunk-2077 --path "/path/to/game"
modn game add cyberpunk-2077 --src "./local-module" --path "/path/to/game"
modn game list
modn game remove cyberpunk-2077
modn play cyberpunk-2077
modn play cyberpunk-2077 --path "/path/to/game"

modn bucket list
modn bucket update
modn update
modn bucket add community "/path/to/catalog"
modn bucket remove community
modn bucket trust-modules community --fingerprint <KEY_ID> --yes
modn bucket untrust-modules community

modn module register ./module --trust
modn module list
modn module pin <game-id>
modn module unpin <game-id>
modn module update <game-id>
modn module rollback <game-id>
modn module rollback <game-id> --to <version>
modn module remove <game-id>

modn search cet
modn search --game cyberpunk-2077
modn install cet --file "/path/to/CET.zip" --dry-run
modn install redscript --file "/path/to/redscript.zip"
modn install cyberpunk-2077/cet --file "/path/to/CET.zip" --yes
modn install main/cyberpunk-2077/cet --file "/path/to/CET.zip" --yes
modn install main/cyberpunk-2077/example --nxm "nxm://cyberpunk2077/mods/6945/files/123456?key=...&expires=..."
modn install "./unlisted-mod.zip" --dry-run
modn install "./unlisted-mod.zip" --yes
modn install <mod> --allow-native-injector --yes
modn update cet --dry-run
modn update cet --yes
modn update example --nxm "nxm://cyberpunk2077/mods/6945/files/123457?key=...&expires=1700000000" --yes
modn auth nexus set
modn auth nexus status
modn auth nexus clear
modn pin cet
modn unpin cet
modn update community/test-game/example --version 1.0 --file "./example-1.0.zip" --yes
modn list --json
modn history --json
modn uninstall <managed-package-id> --yes
modn uninstall cet --dry-run
modn uninstall codeware --include-deps --yes
modn uninstall red4ext --cascade --yes
```

`modn install` resolves mod IDs only, not games. Narrow a name by
`game/mod` or `bucket/game/mod`. Interactive terminals prompt for colliding
mod IDs; noninteractive invocations return candidates (structured JSON errors
under `--json`). `modn search` matches mod ID or display name and accepts
global `--game`.

## Progress events and cancellation

`--progress json` streams machine-readable events for `install`, `update` and
`uninstall`. Each event is one JSON object on one line on **stderr**. The
command's normal output on stdout is unchanged, including under `--json`.
Progress mode never prompts, so pass `--yes`, or use `--dry-run` to preview.
It also reports failures as the JSON error object described under
[Error codes](#error-codes), without the human `error:` prefix.

Every operation reports these phases in order:

| Event | Meaning |
|---|---|
| `{"phase":"resolve"}` | The command starts to resolve its target and plan. |
| `{"phase":"download","package":"<reference>","bytes":<n>,"total":<n>}` | An archive download advanced to `bytes`. `total` is the expected size, or `null` when the source does not state it. |
| `{"phase":"extract","package":"<reference>"}` | Modn extracts the archive into manager-owned staging. |
| `{"phase":"stage","package":"<reference>"}` | Modn verifies the staged content and records the staged transaction. |
| `{"phase":"commit","package":"<reference>"}` | The commit point: Modn starts to change the game. |
| `{"phase":"result","outcome":"succeeded"\|"failed"\|"cancelled","error":"<CODE>"}` | The operation ended. `error` is absent on success. |

`package` is the catalog reference, or the managed-package ID when the package
has none. A stack install reports one download, extract, stage and commit
group per package. [transactions.md](transactions.md) defines the commit
point.

Write one line `cancel` to stdin to ask the operation to stop. Modn checks the
request at the start and between phases. Before the commit point, the command
exits nonzero with `CANCELLED`, reports a `cancelled` result, and leaves game
files, manager state and ownership records as they were. A cancelled download
removes its partial file and keeps no cache entry. From the commit point on,
Modn ignores the request and finishes, or rolls back, the operation as it does
without one. Close stdin when no cancellation is wanted.

## Catalog refresh

`modn update` without a target refreshes every catalog and changes no game
files or install state. It first refreshes the official `main` bucket exactly as
`modn bucket update` does. Then it syncs each registered Git catalog to the
latest revision of its registered source, exactly as `modn bucket sync <name>`
does, and verifies the signature against the pinned signer. Local folder
buckets are skipped because they have nothing to fetch.

Each catalog is independent. A failing catalog is reported, the remaining
catalogs still refresh, and the command exits with a nonzero status. Human
output prints one line per bucket. `--json` prints one value with a `buckets`
array; each entry has `name`, `kind`, and `changed`, `skipped` or `error`.

The options for one installed mod need a target and are refused without one:
`--version`, `--file`, `--path`, `--game`, `--nxm`, `--dry-run`, `--no-cache`,
`--download`, `--allow-executable` and `--allow-native-injector`. A refresh always
fetches, so `modn update --dry-run` is refused rather than ignored. Use
`modn update <target>` to replace one installed catalog mod as before.

## Module releases and trust

A game module release is a required `version` plus the SHA-256 digest of
`module.json` and its entrypoint. The registry keeps retained releases and one
active release under `<data>/game-modules/<game-id>/`. See
[game modules](game-modules.md#module-releases).

```sh
modn bucket trust-modules <bucket> [--fingerprint <KEY_ID>] --yes
modn bucket untrust-modules <bucket>
modn module list [--json]
modn module pin <game-id>
modn module unpin <game-id>
modn module update <game-id>
modn module rollback <game-id> [--to <version>]
```

`bucket trust-modules <bucket>` trusts the modules of one registered signed Git
catalog. It prints the pinned signer key ID and refuses without `--yes`.
`--fingerprint <KEY_ID>` is optional and must match that key ID. Trust covers the
whole bucket. A signer key change ends trust.

`bucket untrust-modules <bucket>` removes the trust record and keeps the bucket
registered. Retained modules from that bucket are then refused at use.

`module list` verifies and lists every retained release. Each entry has the
module identity, `sha256`, `source`, `active`, `source_trusted` and `pinned`.
Text output marks an active release `[active]` and the pinned release `[pinned]`.
It also warns when a release's bucket is not a trusted module source.

`module pin <game-id>` records the active version and digest in `pin.json`.
While pinned, automatic imports, `module update` and `module rollback` do not
move the active release. `module unpin` removes the pin.

`module update <game-id>` fetches the module's source bucket and switches to a
newer retained release. Under `--json` it reports `game_id`, `from`, `to` and
`outcome`. The outcomes are `imported`, `unchanged`, `not_newer` and `pinned`.
A local registration has no source bucket, so update refuses it.

`module rollback <game-id>` switches the active release to the previous retained
release. `--to <version>` selects a specific retained release. Rollback changes
only future plans; it never changes installed files or game files. It refuses
while the module is pinned.

`bucket sync <name>` reports the modules it found. Text output prints each
imported release and each ignored module. `--json` adds a `modules` value with
two arrays:

- `imported`: each entry has `game_id`, `version`, `sha256` and `outcome`.
- `ignored`: each entry has `game_id` and `reason`, either `untrusted_source` or
  `official_priority`.

The reason `untrusted_source` means Modn did not parse or execute the module.
Run `bucket trust-modules` to import it. The reason `official_priority` means
`main` provides the game, so Modn uses the official module. `doctor` reports
that case as `MODULE_HIDDEN_BY_OFFICIAL`.

## Game detection

`modn game detect` reads the Steam, GOG and Epic libraries on Windows. It
matches each listed folder against the store identities that game modules
declare, and the module confirms the folder. See
[game-modules.md](game-modules.md#store-identities-and-detection).

```sh
modn game detect --json
modn game detect --register --json
modn --game cyberpunk-2077 game detect --json
```

Without `--register`, detection writes nothing, not even an empty state
database. `--register` registers each new confirmed installation with the
store as its `source`. It writes manager state only, never game files. An
installation that is already registered keeps its registration and is reported
again. Global `--game` limits detection to one game; otherwise Modn checks every
official module and every registered third-party module. `--state-dir` selects
the profile whose registrations are read and written.

`--json` prints one object:

```json
{"installations":[{"game_id":"cyberpunk-2077","store":"steam","store_id":"1091500",
  "path":"C:\\...\\Cyberpunk 2077","status":"new"}],
 "refused":[{"game_id":"balatro","store":"steam","store_id":"2379780",
  "path":"C:\\...\\Balatro",
  "reason":"DETECT_FOLDER_UNCONFIRMED: the folder does not contain Balatro.exe"}],
 "skipped":[]}
```

`status` is `new`, `registered` (by this run) or `already_registered`.
`refused` lists folders that matched a store identity but that the module did
not confirm. `skipped` lists games whose module could not be read. One
unreadable registered module is reported in `skipped` and does not stop
detection of the other games.

`modn game add <game-id>` without `--path` uses the same detection for one
game. It registers the installation only when exactly one is confirmed.

## Launching a game

`modn play <game-id>` starts one registered installation. Use `--path` when
several installations are registered for the same game. An unregistered folder
is refused with `LAUNCH_INSTALLATION_UNREGISTERED`; register it with
`modn game add` first.

```sh
modn play cyberpunk-2077 --json
modn play balatro --path "C:\\...\\Balatro"
```

Play is read-only: it writes no manager state and no game files. It chooses the
store URI when detection shows that the store installed the selected folder.
Otherwise it starts the module's declared executable with the game folder as its
working directory. A game module declares both as data; see
[game-modules.md](game-modules.md#launch-targets). Play holds the transaction
lock from its pending-transaction check until the process starts or the URI
opens, so a concurrent install or uninstall cannot begin in between. The lock is
released before the game runs.

`--json` reports what was launched:

```json
{"game_id":"cyberpunk-2077","installation":"C:\\...\\Cyberpunk 2077",
 "target":{"kind":"store_uri","store":"steam","uri":"steam://rungameid/1091500"},
 "warnings":[]}
```

Play refuses while a transaction awaits recovery with
`LAUNCH_RECOVERY_PENDING`; run `modn recover` first. It refuses an unusable or
unsafe target with `LAUNCH_TARGET_UNAVAILABLE` or `LAUNCH_TARGET_UNSAFE`. When
manager-owned files are missing or changed outside Modn, it warns with
`LAUNCH_MANAGED_FILES_CHANGED` and still launches. Run `modn doctor` to inspect
the changed files.

## Games, packages and uninstall

`modn game remove <game-id>` only unregisters an installation and never
deletes game files. It refuses to orphan managed mods or pending transactions;
use `--path` if multiple installations have the same ID. `modn list`
shows manager-owned packages, not unmanaged content. `modn uninstall`
accepts the managed-package ID shown by `modn list`, or the catalog name the
package was installed under: `mod`, `game/mod` or `bucket/game/mod`. `modn list`
shows that name beside each ID. A name that matches several packages is refused.
Packages from a standalone archive have no catalog name, only an ID.

A package's `directories` are its exclusive folders, which it owns entirely.
Modn also records each folder it creates for owned files, such as
`red4ext/plugins`. These folders belong to the game installation, not to one
package. An uninstall removes each one when it is empty and keeps it while
other content, such as a runtime log, remains. `modn list` prints these
folders after the packages, and `doctor --json` reports them as
`created_directories`. `list --json` stays an array of packages.

`modn uninstall <package> --dry-run` lists the packages it would remove, in
order, and writes nothing. `--include-deps` also removes each dependency that
no remaining package needs. `--cascade` also removes every package that needs
the selected one, dependents first. Without `--cascade`, a needed package is
refused. Modn does not record whether you asked for a package or it came in as
a dependency, so `--include-deps` also removes a dependency you installed on
its own, once nothing else needs it. Review the dry run first. `--yes` and
`--dry-run` exclude each other. Each package is a separate recoverable
removal under one lock. If one fails, the earlier removals stay done, the rest
stay installed, and the error names both.

## Operation history

`modn history` lists committed install, update and uninstall operations,
newest first. `--json` prints one array; the default prints a short table of
ID, kind, game, packages and time. `--game <game-id>` keeps one game.
`--state-dir` selects the manager profile, including its history.

Each record holds the operation ID and kind, game ID, installation root, UTC
time, and for every package the mod ID, package IDs before and after, release
versions and source descriptors, archive SHA-256 and BLAKE3 digests, and the
owned files with content digests before and after. A record is written in the
same SQLite transaction as the ownership change. A cancelled, refused,
dry-run or rolled-back operation writes none. See
[operation history](history.md) for the JSON shape and storage tables.

## Undo

`modn undo [<operation-id>] --yes` reverses one committed operation from
`modn history`. Without an ID it selects the newest operation for the selected
installation. `--path <directory>` selects the installation and `--game
<game-id>` selects a registered game; when several installations of one game
are registered, a missing `--path` is refused instead of guessing. An explicit
ID from another installation or game is refused with `UNDO_NOT_FOUND`, and the
message names the installation that owns the record. The inverse follows the
recorded kind: an install is removed, an update installs the earlier release
again, and an uninstall installs the recorded release again. The command
writes one new history record whose `undoes` field names the target.

Undo refuses before any change when a later operation on the same installation
touched one of the target's packages or files. The error is `UNDO_CONFLICT` and
names that newer operation; undo it first. A package whose recorded source is
`manual` cannot be fetched again: the error is `UNDO_SOURCE_MANUAL`, and you
must place its archive with `modn install --file`. `git` and `nexus` releases
are restored only from the archive cache. When the archive is no longer cached,
the error is `UNDO_FAILED`; reinstall the release from the catalog. A Nexus
download needs a new nxm link. An update undo needs the
installed package's catalog identity. An update undo also checks every restored
file against the record: the archive member that the current module selects for
a destination must have the recorded `digest_before` SHA-256, or the error is
`UNDO_FAILED` with no change. A planned file whose root differs from the record
is refused the same way. Restored files are written below their recorded root,
including a module-declared appdata content root. Normal refusals stay: a pinned
package (`PACKAGE_PINNED`), a package another installed package needs, an
unfinished transaction (`PENDING_TRANSACTION`) or an ownership `CONFLICT` blocks
the undo. A multi-package undo commits every package or none.

`--dry-run` verifies the archives and prints the inverse without writing.
`--yes` is required to change game files. `--json` prints one object.
`--progress json` reports phases and accepts `cancel` before the commit point.
The default error code is `UNDO_FAILED`.

## Native-injector and executable approval

Modn stores one approval per mod and game installation. It records the release
capabilities you approved: whether the release is a native injector, and the
executable payload paths it declares. An install or update covered by that
approval needs no flag. `modn uninstall` keeps the approval. A release that
drops a capability does not shrink it.

A release that adds a capability the stored approval does not cover is refused
before a game write. A new native injector, including a native-injector
dependency, returns `NATIVE_INJECTOR_NOT_ALLOWED`. A new executable path
returns `APPROVAL_REQUIRED`. The JSON error names the mod ID and the uncovered
capabilities, for example:

```json
{"error":"APPROVAL_REQUIRED","mod_id":"example","package":"main/cyberpunk-2077/example","uncovered":{"native_injector":false,"executables":["engine/tools/new.exe"]},"flags":["--allow-executable"]}
```

Pass the matching flag to approve the release:

```sh
modn install main/cyberpunk-2077/example --allow-native-injector --yes
modn install main/cyberpunk-2077/example --allow-executable engine/tools/new.exe --yes
modn update example --allow-native-injector --yes
```

A successful install or update widens the stored approval. A cancelled,
refused, dry-run or rolled-back operation stores nothing. An interactive prompt
can approve new executable paths. `install --dry-run --json` reports
`approval.required`, `approval.covered_by_stored_approval`, the stored approval
and the uncovered capabilities. There is no command to revoke an approval;
`modn uninstall` does not remove it.

## Nexus Mods credentials and downloads

`modn auth nexus set` reads the API key from the first line of stdin and saves it in the operating system credential store.
The key never appears on the command line.
`modn auth nexus status` reports whether a key is set, and whether it comes from the credential store or `MODN_NEXUS_API_KEY`.
`modn auth nexus clear` removes the stored key.
`MODN_NEXUS_API_KEY` overrides a stored key.

A Premium account needs only the API key for a `nexus` source.
A free account also passes the Nexus "Mod Manager Download" link with `--nxm`:

```sh
modn install main/cyberpunk-2077/example --nxm "nxm://cyberpunk2077/mods/6945/files/123456?key=...&expires=..."
modn update example --nxm "nxm://cyberpunk2077/mods/6945/files/123457?key=...&expires=..."
```

`--nxm` is repeatable on `install` and `update`.
Modn matches each link to a package by game domain, mod ID and file ID.
An unmatched link is an error.
The link expires quickly and may remain in shell history.
Modn does not register as the `nxm://` handler.
See [sources.md](sources.md) for transport and secrecy guarantees.

## Error codes

Under `--json`, every failure prints one object to stderr:
`{"error":"<CODE>","message":"<text>"}`. The code is stable and
SCREAMING_SNAKE_CASE. A specific code names a condition a caller can act on;
otherwise the command's default code is used. No command reports a generic
`ERROR` code.

| Code | Meaning |
|---|---|
| `ALREADY_INSTALLED` | The update target already uses the selected archive. |
| `AMBIGUOUS_TARGET` | A mod name matches several catalog mods. |
| `APPROVAL_REQUIRED` | A write needs `--yes` or an interactive confirmation, or a release adds an executable path the stored approval does not cover. |
| `ARCHIVE_INVALID` | An archive cannot be read or changed during the operation. |
| `ARCHIVE_REQUIRED` | A mod has no downloadable source and needs `--file`. |
| `CANCELLED` | The operation was cancelled before its commit point. |
| `CONFLICT` | A declared or ownership conflict blocks the operation. |
| `DEPENDENCY_MISSING` | A dependency is not installed or not intact. |
| `DETECT_FOLDER_UNCONFIRMED` | The game module did not confirm a folder that matched its store identity. |
| `DETECT_FOLDER_UNREADABLE` | A detected game folder cannot be accessed. |
| `DETECT_MODULE_FAILED` | The game module failed while confirming a folder. |
| `DETECT_MODULE_UNREADABLE` | The game module of a requested game cannot be read. |
| `DIGEST_MISMATCH` | A downloaded or supplied archive does not match its pinned digest. |
| `DOWNLOAD_FAILED` | A download did not complete. |
| `GAME_MODULE_ERROR` | A game module failed to load or run. |
| `GAME_NOT_SELECTED` | Modn cannot select one registered game installation. |
| `GIT_SOURCE_COMMIT_MISMATCH` | A Git source resolved to another commit. |
| `GIT_SOURCE_FETCH_FAILED` | A Git source could not be fetched or read. |
| `GIT_SOURCE_UNSUPPORTED_ENTRY` | A Git source contains an entry Modn does not install. |
| `HISTORY_FAILED` | Default code of `modn history`. |
| `INVALID_ARGUMENTS` | The command line is invalid. |
| `IO_ERROR` | A filesystem operation failed. |
| `LAUNCH_DRIFT_CHECK_FAILED` | Manager-owned file drift cannot be checked before launch. |
| `LAUNCH_GAME_MISMATCH` | `--game` does not match the game `modn play` selects. |
| `LAUNCH_INSTALLATION_INVALID` | The selected play installation cannot be resolved. |
| `LAUNCH_INSTALLATION_UNREGISTERED` | The selected play installation is not registered. |
| `LAUNCH_MANAGED_FILES_CHANGED` | Manager-owned files changed since Modn installed them. |
| `LAUNCH_RECOVERY_PENDING` | An unfinished transaction must be recovered before launch. |
| `LAUNCH_START_FAILED` | The game process or store URI cannot be started. |
| `LAUNCH_TARGET_UNAVAILABLE` | The game module declares no launch target for this installation. |
| `LAUNCH_TARGET_UNSAFE` | A launch target is missing, is not a file, or resolves outside the game. |
| `MOD_NOT_FOUND` | No catalog mod matches the target. |
| `MODULE_LAUNCH_TARGET_INVALID` | A module declares an invalid launch target. |
| `MODULE_STORE_ID_INVALID` | A module declares a store identity that is not valid. |
| `NATIVE_INJECTOR_NOT_ALLOWED` | A native-injector package needs `--allow-native-injector`, or its stored approval does not cover native code. |
| `PACKAGE_NEEDED` | Another installed package needs the selected package. |
| `PACKAGE_NOT_FOUND` | No managed package matches the selector. |
| `PACKAGE_PINNED` | The managed package is pinned. |
| `PENDING_TRANSACTION` | An interrupted transaction must be recovered first. |
| `PLAN_BLOCKED` | The installation plan has blockers. |
| `STATE_ERROR` | Manager state cannot be read or written. |
| `UNDO_CONFLICT` | A later operation changed a package or file that the undo target owns. |
| `UNDO_FAILED` | Default code of `modn undo`. |
| `UNDO_NOT_FOUND` | No committed operation matches the undo target. |
| `UNDO_SOURCE_MANUAL` | The target release has a manual source and cannot be downloaded again. |
| `ADOPT_FAILED` | Default code of `modn adopt`. |
| `ASSET_FAILED` | Default code of `modn asset`. |
| `AUTH_FAILED` | Default code of `modn auth`. |
| `BUCKET_FAILED` | Default code of `modn bucket`. |
| `CACHE_FAILED` | Default code of `modn cache`. |
| `DOCTOR_FAILED` | Default code of `modn doctor`. |
| `GAME_FAILED` | Default code of `modn game`. |
| `INSPECT_FAILED` | Default code of `modn inspect`. |
| `INSTALL_FAILED` | Default code of `modn install`. |
| `LIST_FAILED` | Default code of `modn list`. |
| `MODULE_FAILED` | Default code of `modn module`. |
| `PIN_FAILED` | Default code of `modn pin` and `modn unpin`. |
| `PLAY_FAILED` | Default code of `modn play`. |
| `RECOVER_FAILED` | Default code of `modn recover`. |
| `SCAN_FAILED` | Default code of `modn scan`. |
| `SEARCH_FAILED` | Default code of `modn search`. |
| `UNINSTALL_FAILED` | Default code of `modn uninstall`. |
| `UPDATE_FAILED` | Default code of `modn update`. |

## Additional commands

- `modn scan --path <game>` and `modn doctor --path <game>` read the game root and an API 2 module's content root. Content-root entries are labelled `content`. See [inventory.md](inventory.md).
- `modn bucket sync <name> --source <remote> --revision <reference>` registers a Git catalog. The first sync pins the signer key, and later syncs refuse a key change. See [sources.md](sources.md).
- `modn adopt <archive> --dry-run` previews manual adoption. Commit requires `--acknowledge --yes`.
- `modn uninstall <package> --force --yes` approves cleanup of modified owned files.

[lifecycle.md](lifecycle.md) defines adoption and cleanup authority, refusals and recovery.
[mcp.md](mcp.md) and [desktop.md](desktop.md) describe the companion interfaces.
