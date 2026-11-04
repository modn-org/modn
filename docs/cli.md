# Command reference

## Commands

Game registration, mod installation and bucket management use separate commands:

```sh
modn game add cyberpunk-2077 --path "/path/to/game"
modn game add cyberpunk-2077 --src "./local-module" --path "/path/to/game"
modn game list
modn game remove cyberpunk-2077

modn bucket list
modn bucket update
modn bucket add community "/path/to/catalog"
modn bucket remove community

modn search cet
modn search --game cyberpunk-2077
modn install cet --file "/path/to/CET.zip" --dry-run
modn install redscript --file "/path/to/redscript.zip"
modn install cyberpunk-2077/cet --file "/path/to/CET.zip" --yes
modn install main/cyberpunk-2077/cet --file "/path/to/CET.zip" --yes
modn install "./unlisted-mod.zip" --dry-run
modn install "./unlisted-mod.zip" --yes
modn update cet --dry-run
modn update cet --yes
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
