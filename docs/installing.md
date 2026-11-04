# Installing mods

## Catalog and standalone installs

Catalog installation needs each archive with the exact format and SHA-256
from its manifest. Modn downloads an `https` source itself. A `manual`
source, such as a Nexus file, is never downloaded: supply it with `--file`,
and without it Modn names the file to get. Standalone
`modn install <archive.zip|archive.7z>` needs no catalog entry and uses
the existing Lua planner and Rust ownership transaction. `--dry-run` previews
without game writes. A catalog install in an interactive terminal shows what
it will write and asks `Install? [y/N]`; scripts and `--json` must pass
`--yes`. Standalone archive installs always require `--yes`. Each catalog
dependency must be installed with its pinned digest and intact managed
content, or be installed in the same command. Installation re-checks this
while holding the transaction lock. Each catalog installation records its `bucket/game/mod` provenance and
the managed packages that satisfied its dependencies, in the same SQLite
commit as its ownership. `modn uninstall` refuses to remove a recorded
dependency while a dependent managed package remains; uninstall the dependent
first. Standalone archive installs and packages installed before this record
existed carry no dependency links.

## Dependency plan

`modn install <mod> --dry-run` also reports a read-only dependency plan.
It lists every dependency of the mod, transitively, in install order. The
plan resolves dependencies in the mod's own bucket and game. A cycle or an
unknown dependency ID fails the dry run. Each dependency has a state:
`ready` (managed with the pinned digest and intact), `damaged` (pinned digest
but drifted content), `other_release` (recorded from the same catalog mod
with another digest) or `missing`. Missing dependencies name their artifact
source. Without `--file`, the dry run names the archive to supply, and JSON
`plan` is `null`. JSON adds `dependencies` and `dependencies_ready`. Each
dependency also shows the supplied `archive`, if one matches. The plan
writes nothing.

## Download during a dry run

A dry run never downloads by default. Add `--download` to fetch the missing
`https` archives of the mod and its missing dependencies into the verified
download cache first. The dry run then shows the file plan, and a later
install reuses the cached archives. `--download` writes only to the cache,
never to the game, and applies only with `--dry-run`. `manual` sources still
need `--file`.

## Install a stack

To install a mod with its missing dependencies, pass `--file` once for each
`manual` archive, in any order. Modn downloads or reuses the `https` ones.
Modn matches each archive to a release by SHA-256 and refuses an archive that
no release in the installation pins. When a `manual` archive is missing, Modn
names it and writes nothing. One confirmation, or `--yes`, approves the whole stack. With `--yes`,
each executable that a stack manifest declares also needs `--allow-executable`.
Modn installs the stack in dependency order under one transaction lock. The
game policy plans each package after the packages before it are installed.
Each package keeps its own ownership record, provenance and dependency links.
If any package fails, Modn uninstalls the packages that this command
installed, newest first. Rollback, like `modn uninstall`, also removes the
parent directories that Modn created, once they are empty. Two
dependencies that pin the same archive digest install it once. If the process stops before the stack completes,
other writes are refused until `modn recover --yes` uninstalls the stack's
packages. A damaged dependency or another installed release is never replaced:
the install fails until you restore or uninstall it, or replace it with
`modn update`.

## Tool executables

A catalog manifest may declare the tool executables its archive installs,
for example redscript's `engine/tools/scc.exe`. A declaration only lets Modn
name them; it never approves them. The interactive prompt lists each declared
executable the archive installs that no `--allow-executable` flag already
approved, and installs them only if you answer `y`. `--yes` never
approves a declared executable: scripts pass `--allow-executable <path>` for
each one, exactly as for standalone archives. An executable the manifest does
not declare stays blocked unless `--allow-executable` names it.
`--dry-run --json` reports `unapproved_declared_executables`.
