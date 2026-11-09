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
not confirm. `skipped` lists games whose module could not be read.

`modn game add <game-id>` without `--path` uses the same detection for one
game. It registers the installation only when exactly one is confirmed.

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

## Native injector approval

A package manifest sets `native_injector: true` when its release places a
native library that the game process loads. `modn install` and `modn update`
refuse such a plan, including a native-injector dependency, before any download
or game write. The error names the package and the flag. Under `--json` it is
`{"error":"NATIVE_INJECTOR_NOT_ALLOWED","package":"<reference>","flag":"--allow-native-injector"}`.
`--dry-run` refuses with the same code. Pass the flag to allow the package:

```sh
modn install main/cyberpunk-2077/example --allow-native-injector --yes
modn update example --allow-native-injector --yes
```

An already-installed native dependency does not require the flag again.
`modn uninstall` never needs it.

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
| `APPROVAL_REQUIRED` | A write needs `--yes` or an interactive confirmation. |
| `ARCHIVE_INVALID` | An archive cannot be read or changed during the operation. |
| `ARCHIVE_REQUIRED` | A mod has no downloadable source and needs `--file`. |
| `CANCELLED` | The operation was cancelled before its commit point. |
| `CONFLICT` | A declared or ownership conflict blocks the operation. |
| `DEPENDENCY_MISSING` | A dependency is not installed or not intact. |
| `DIGEST_MISMATCH` | A downloaded or supplied archive does not match its pinned digest. |
| `DOWNLOAD_FAILED` | A download did not complete. |
| `GAME_MODULE_ERROR` | A game module failed to load or run. |
| `GAME_NOT_SELECTED` | Modn cannot select one registered game installation. |
| `GIT_SOURCE_COMMIT_MISMATCH` | A Git source resolved to another commit. |
| `GIT_SOURCE_FETCH_FAILED` | A Git source could not be fetched or read. |
| `GIT_SOURCE_UNSUPPORTED_ENTRY` | A Git source contains an entry Modn does not install. |
| `INVALID_ARGUMENTS` | The command line is invalid. |
| `IO_ERROR` | A filesystem operation failed. |
| `MOD_NOT_FOUND` | No catalog mod matches the target. |
| `NATIVE_INJECTOR_NOT_ALLOWED` | A native-injector package needs `--allow-native-injector`. |
| `PACKAGE_NEEDED` | Another installed package needs the selected package. |
| `PACKAGE_NOT_FOUND` | No managed package matches the selector. |
| `PACKAGE_PINNED` | The managed package is pinned. |
| `PENDING_TRANSACTION` | An interrupted transaction must be recovered first. |
| `PLAN_BLOCKED` | The installation plan has blockers. |
| `STATE_ERROR` | Manager state cannot be read or written. |
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
